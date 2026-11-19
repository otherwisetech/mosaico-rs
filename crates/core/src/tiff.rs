//! Minimal TIFF/EXIF IFD walker: capture dates, orientation and embedded JPEG previews.
//! Used for EXIF blocks inside JPEG/HEIC/PNG/WebP/CR3 and for TIFF-based RAWs (CR2, NEF, ARW,
//! DNG, ORF, RW2), which only differ in their magic number. Reads by seeking; never loads the
//! whole file.
use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Default, Clone)]
pub struct TiffInfo {
    pub date_original: Option<String>,
    pub date_digitized: Option<String>,
    pub date_time: Option<String>,
    /// EXIF orientation of the main image (1 = upright).
    pub orientation: Option<u16>,
    /// Embedded JPEGs as (absolute offset, length) in the underlying reader.
    pub jpegs: Vec<(u64, u64)>,
}

impl TiffInfo {
    /// DateTimeOriginal, then CreateDate (DateTimeDigitized), then DateTime.
    pub fn best_date(&self) -> Option<chrono::NaiveDateTime> {
        [&self.date_original, &self.date_digitized, &self.date_time].into_iter().flatten().find_map(|s| crate::date::parse_exif(s))
    }

    pub fn merge(&mut self, other: TiffInfo) {
        self.date_original = self.date_original.take().or(other.date_original);
        self.date_digitized = self.date_digitized.take().or(other.date_digitized);
        self.date_time = self.date_time.take().or(other.date_time);
        self.orientation = self.orientation.or(other.orientation);
        self.jpegs.extend(other.jpegs);
    }
}

const MAX_IFDS: usize = 32;
const MAX_ENTRIES: u16 = 2000;

struct Walker<'a, R> {
    r: &'a mut R,
    base: u64,
    le: bool,
}

impl<R: Read + Seek> Walker<'_, R> {
    fn u16_at(&mut self, off: u64) -> Option<u16> {
        let mut b = [0; 2];
        self.r.seek(SeekFrom::Start(self.base + off)).ok()?;
        self.r.read_exact(&mut b).ok()?;
        Some(if self.le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }

    fn u32_at(&mut self, off: u64) -> Option<u32> {
        let mut b = [0; 4];
        self.r.seek(SeekFrom::Start(self.base + off)).ok()?;
        self.r.read_exact(&mut b).ok()?;
        Some(if self.le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }

    fn bytes_at(&mut self, off: u64, n: usize) -> Option<Vec<u8>> {
        let mut v = vec![0; n];
        self.r.seek(SeekFrom::Start(self.base + off)).ok()?;
        self.r.read_exact(&mut v).ok()?;
        Some(v)
    }
}

/// Size in bytes of one value of a TIFF field type.
fn type_size(t: u16) -> u64 {
    match t {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 => 8,
        _ => 0,
    }
}

struct Entry {
    tag: u16,
    typ: u16,
    count: u32,
    /// Offset (relative to the TIFF header) of the value, inline or not.
    value_off: u64,
}

/// Walk a TIFF structure whose header starts at `base`. Returns None when it isn't TIFF.
pub fn read<R: Read + Seek>(r: &mut R, base: u64) -> Option<TiffInfo> {
    let mut hdr = [0; 8];
    r.seek(SeekFrom::Start(base)).ok()?;
    r.read_exact(&mut hdr).ok()?;
    let le = match &hdr[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let mut w = Walker { r, base, le };
    let first = if le { u32::from_le_bytes(hdr[4..8].try_into().unwrap()) } else { u32::from_be_bytes(hdr[4..8].try_into().unwrap()) } as u64;
    let mut info = TiffInfo::default();
    let mut queue = vec![(first, true)];
    let mut seen = HashSet::new();
    while let Some((off, is_ifd0)) = queue.pop() {
        if off < 8 || !seen.insert(off) || seen.len() > MAX_IFDS {
            continue;
        }
        let Some(n) = w.u16_at(off) else { continue };
        if n == 0 || n > MAX_ENTRIES {
            continue;
        }
        let mut entries = Vec::with_capacity(n as usize);
        for i in 0..n as u64 {
            let e = off + 2 + i * 12;
            let (Some(tag), Some(typ), Some(count)) = (w.u16_at(e), w.u16_at(e + 2), w.u32_at(e + 4)) else { break };
            let size = type_size(typ) * count as u64;
            let value_off = if size <= 4 { e + 8 } else { w.u32_at(e + 8).unwrap_or(0) as u64 };
            entries.push(Entry { tag, typ, count, value_off });
        }
        let get = |tag: u16| entries.iter().find(|e| e.tag == tag);
        let num = |w: &mut Walker<R>, e: &Entry, i: u64| -> Option<u64> {
            match e.typ {
                3 => w.u16_at(e.value_off + i * 2).map(u64::from),
                4 | 13 => w.u32_at(e.value_off + i * 4).map(u64::from),
                1 | 7 => w.bytes_at(e.value_off + i, 1).map(|b| b[0] as u64),
                _ => None,
            }
        };
        let ascii = |w: &mut Walker<R>, e: &Entry| -> Option<String> {
            if e.typ != 2 || e.count == 0 || e.count > 64 {
                return None;
            }
            let b = w.bytes_at(e.value_off, e.count as usize)?;
            Some(String::from_utf8_lossy(&b).trim_end_matches('\0').to_string())
        };
        for e in &entries {
            match e.tag {
                0x9003 => info.date_original = info.date_original.take().or_else(|| ascii(&mut w, e)),
                0x9004 => info.date_digitized = info.date_digitized.take().or_else(|| ascii(&mut w, e)),
                0x0132 => info.date_time = info.date_time.take().or_else(|| ascii(&mut w, e)),
                0x0112 if is_ifd0 => info.orientation = num(&mut w, e, 0).map(|v| v as u16),
                // Exif IFD and SubIFDs (RAW previews live in these)
                0x8769 => {
                    if let Some(v) = num(&mut w, e, 0) {
                        queue.push((v, false));
                    }
                }
                0x014A => {
                    for i in 0..(e.count as u64).min(8) {
                        if let Some(v) = num(&mut w, e, i) {
                            queue.push((v, false));
                        }
                    }
                }
                // Panasonic RW2: a full JPEG (with its own EXIF) stored as one tag value
                0x002E if e.typ == 7 && e.count > 4 => info.jpegs.push((base + e.value_off, e.count as u64)),
                _ => {}
            }
        }
        if let (Some(o), Some(l)) = (get(0x0201), get(0x0202)) {
            if let (Some(o), Some(l)) = (num(&mut w, o, 0), num(&mut w, l, 0)) {
                info.jpegs.push((base + o, l));
            }
        }
        // a single JPEG-compressed strip: CR2's IFD0 preview, DNG reduced-resolution previews
        let compression = get(0x0103).and_then(|e| num(&mut w, e, 0));
        let subfile = get(0x00FE).and_then(|e| num(&mut w, e, 0)).unwrap_or(0);
        if let (Some(so), Some(sc)) = (get(0x0111), get(0x0117))
            && so.count == 1
            && (compression == Some(6) || (compression == Some(7) && subfile == 1))
            && let (Some(o), Some(l)) = (num(&mut w, so, 0), num(&mut w, sc, 0))
        {
            info.jpegs.push((base + o, l));
        }
        if let Some(next) = w.u32_at(off + 2 + n as u64 * 12)
            && next != 0
        {
            queue.push((next as u64, false));
        }
    }
    // keep only offsets that really start a JPEG
    let mut ok = Vec::new();
    for (o, l) in std::mem::take(&mut info.jpegs) {
        let mut soi = [0; 3];
        if l > 100 && w.r.seek(SeekFrom::Start(o)).is_ok() && w.r.read_exact(&mut soi).is_ok() && soi == [0xFF, 0xD8, 0xFF] && !ok.contains(&(o, l)) {
            ok.push((o, l));
        }
    }
    info.jpegs = ok;
    Some(info)
}
