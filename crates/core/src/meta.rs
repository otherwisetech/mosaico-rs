//! Per-file metadata: capture date (with its source), orientation and embedded previews.
use crate::bmff;
use crate::date::{self, DateSource};
use crate::media::Format;
use crate::tiff::{self, TiffInfo};
use chrono::NaiveDateTime;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::SystemTime;

#[derive(Debug, Clone, Default)]
pub struct Meta {
    /// Date from the file's contents (EXIF or video metadata), if any.
    pub embedded: Option<(NaiveDateTime, DateSource)>,
    pub orientation: u16,
    /// Embedded JPEG previews as (offset, length) in the file.
    pub previews: Vec<(u64, u64)>,
}

/// Read what the file itself says. Never fails: unreadable or unknown data gives an empty Meta.
pub fn read(path: &Path, format: Format) -> Meta {
    let Ok(f) = File::open(path) else { return Meta::default() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut r = BufReader::with_capacity(16 << 10, f);
    read_from(&mut r, len, format)
}

pub fn read_from<R: Read + Seek>(r: &mut R, len: u64, format: Format) -> Meta {
    if matches!(format, Format::Mov | Format::Mp4) {
        let embedded = bmff::video_date(r, len).map(|d| (d, DateSource::Video));
        return Meta { embedded, orientation: 1, previews: Vec::new() };
    }
    let info: Option<TiffInfo> = match format {
        Format::Jpeg => jpeg_exif(r, 0),
        Format::Png => png_exif(r, len),
        Format::Webp => webp_exif(r, len),
        Format::Heic => bmff::heif_exif(r, len),
        Format::Cr3 => bmff::cr3(r, len),
        Format::Raf => raf(r),
        Format::Tiff | Format::Cr2 | Format::Nef | Format::Arw | Format::Dng | Format::Orf | Format::Rw2 => tiff::read(r, 0).map(|mut t| {
            // RW2 (and some others) keep the dates only inside their embedded JPEG
            if t.best_date().is_none() {
                for &(o, _) in t.jpegs.clone().iter().take(2) {
                    if let Some(inner) = jpeg_exif(r, o) {
                        let previews = std::mem::take(&mut t.jpegs);
                        t.merge(TiffInfo { jpegs: Vec::new(), ..inner });
                        t.jpegs = previews;
                    }
                }
            }
            t
        }),
        Format::Gif | Format::Mov | Format::Mp4 => None,
    };
    let info = info.unwrap_or_default();
    Meta { embedded: info.best_date().map(|d| (d, DateSource::Exif)), orientation: info.orientation.filter(|o| (1..=8).contains(o)).unwrap_or(1), previews: info.jpegs }
}

/// The final capture date: embedded metadata → file name → modified time (uncertain).
pub fn capture_date(meta: &Meta, path: &Path, modified: Option<SystemTime>) -> (NaiveDateTime, DateSource) {
    if let Some(e) = meta.embedded {
        return e;
    }
    if let Some(d) = path.file_name().and_then(|n| n.to_str()).and_then(date::from_filename) {
        return (d, DateSource::Filename);
    }
    let local = modified.map(chrono::DateTime::<chrono::Local>::from).map(|d| d.naive_local()).unwrap_or_default();
    (local, DateSource::Modified)
}

/// EXIF of the JPEG starting at `start` (its APP1 segment), offsets absolute in `r`.
pub fn jpeg_exif<R: Read + Seek>(r: &mut R, start: u64) -> Option<TiffInfo> {
    let mut b = [0; 2];
    r.seek(SeekFrom::Start(start)).ok()?;
    r.read_exact(&mut b).ok()?;
    if b != [0xFF, 0xD8] {
        return None;
    }
    let mut pos = start + 2;
    for _ in 0..64 {
        r.seek(SeekFrom::Start(pos)).ok()?;
        r.read_exact(&mut b).ok()?;
        if b[0] != 0xFF {
            return None;
        }
        if b[1] == 0xFF {
            pos += 1; // fill byte
            continue;
        }
        if matches!(b[1], 0xDA | 0xD9) {
            return None;
        }
        let mut l = [0; 2];
        r.read_exact(&mut l).ok()?;
        let len = u16::from_be_bytes(l) as u64;
        if b[1] == 0xE1 && len >= 14 {
            let mut id = [0; 6];
            r.read_exact(&mut id).ok()?;
            if &id == b"Exif\0\0" {
                return tiff::read(r, pos + 10);
            }
        }
        pos += 2 + len;
    }
    None
}

fn png_exif<R: Read + Seek>(r: &mut R, len: u64) -> Option<TiffInfo> {
    let mut pos = 8;
    while pos + 12 <= len {
        let mut h = [0; 8];
        r.seek(SeekFrom::Start(pos)).ok()?;
        r.read_exact(&mut h).ok()?;
        let n = u32::from_be_bytes(h[..4].try_into().ok()?) as u64;
        match &h[4..8] {
            b"eXIf" => return tiff::read(r, pos + 8),
            b"IEND" => return None,
            _ => pos += 12 + n,
        }
    }
    None
}

fn webp_exif<R: Read + Seek>(r: &mut R, len: u64) -> Option<TiffInfo> {
    let mut pos = 12;
    while pos + 8 <= len {
        let mut h = [0; 8];
        r.seek(SeekFrom::Start(pos)).ok()?;
        r.read_exact(&mut h).ok()?;
        let n = u32::from_le_bytes(h[4..8].try_into().ok()?) as u64;
        if &h[..4] == b"EXIF" {
            let mut id = [0; 6];
            r.read_exact(&mut id).ok()?;
            let skip = if &id == b"Exif\0\0" { 6 } else { 0 };
            return tiff::read(r, pos + 8 + skip);
        }
        pos += 8 + n + (n & 1);
    }
    None
}

/// Fujifilm RAF: a fixed header pointing at a full JPEG preview that carries the EXIF.
fn raf<R: Read + Seek>(r: &mut R) -> Option<TiffInfo> {
    let mut h = [0; 92];
    r.seek(SeekFrom::Start(0)).ok()?;
    r.read_exact(&mut h).ok()?;
    if !h.starts_with(b"FUJIFILMCCD-RAW") {
        return None;
    }
    let off = u32::from_be_bytes(h[84..88].try_into().ok()?) as u64;
    let len = u32::from_be_bytes(h[88..92].try_into().ok()?) as u64;
    let mut info = jpeg_exif(r, off).unwrap_or_default();
    info.jpegs.push((off, len));
    Some(info)
}
