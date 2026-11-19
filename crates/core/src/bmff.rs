//! ISO base media file format (MOV, MP4, HEIC, CR3): just enough box walking to find capture
//! dates, the EXIF item and embedded previews.
use crate::tiff::{self, TiffInfo};
use chrono::NaiveDateTime;
use std::io::{Cursor, Read, Seek, SeekFrom};

#[derive(Debug, Clone, Copy)]
pub struct BoxHdr {
    pub typ: [u8; 4],
    pub uuid: Option<[u8; 16]>,
    /// First byte of the payload.
    pub data: u64,
    /// One past the last byte of the box.
    pub end: u64,
}

const MAX_BOXES: usize = 4096;

/// The boxes between `start` and `end` (a file, or one box's payload).
pub fn children<R: Read + Seek>(r: &mut R, start: u64, end: u64) -> Vec<BoxHdr> {
    let mut out = Vec::new();
    let mut pos = start;
    while pos + 8 <= end && out.len() < MAX_BOXES {
        let mut h = [0; 8];
        if r.seek(SeekFrom::Start(pos)).is_err() || r.read_exact(&mut h).is_err() {
            break;
        }
        let size32 = u32::from_be_bytes(h[..4].try_into().unwrap()) as u64;
        let typ: [u8; 4] = h[4..8].try_into().unwrap();
        let mut data = pos + 8;
        let size = match size32 {
            0 => end - pos,
            1 => {
                let mut l = [0; 8];
                if r.read_exact(&mut l).is_err() {
                    break;
                }
                data += 8;
                u64::from_be_bytes(l)
            }
            s => s,
        };
        if size < data - pos || pos + size > end {
            break;
        }
        let mut uuid = None;
        if &typ == b"uuid" {
            let mut u = [0; 16];
            if r.read_exact(&mut u).is_err() {
                break;
            }
            uuid = Some(u);
            data += 16;
        }
        out.push(BoxHdr { typ, uuid, data, end: pos + size });
        pos += size;
    }
    out
}

fn find<'a>(boxes: &'a [BoxHdr], typ: &[u8; 4]) -> Option<&'a BoxHdr> {
    boxes.iter().find(|b| &b.typ == typ)
}

fn read_vec<R: Read + Seek>(r: &mut R, b: &BoxHdr, max: u64) -> Option<Vec<u8>> {
    let n = b.end.checked_sub(b.data)?;
    if n > max {
        return None;
    }
    let mut v = vec![0; n as usize];
    r.seek(SeekFrom::Start(b.data)).ok()?;
    r.read_exact(&mut v).ok()?;
    Some(v)
}

/// A `meta` box is a full box in MP4/HEIF (4 bytes version+flags) but not in QuickTime.
fn meta_children(c: &mut Cursor<Vec<u8>>, meta: &BoxHdr) -> Vec<BoxHdr> {
    let mut first = [0; 8];
    c.seek(SeekFrom::Start(meta.data)).ok();
    let full = c.read_exact(&mut first).is_ok() && first[..4] == [0, 0, 0, 0] && &first[4..8] != b"hdlr";
    children(c, meta.data + if full { 4 } else { 0 }, meta.end)
}

const MAC_EPOCH_OFFSET: i64 = 2_082_844_800; // 1904-01-01 → 1970-01-01

/// Creation date of a QuickTime / MP4 file: Apple's `com.apple.quicktime.creationdate` (local
/// wall time), then `©day`, then `mvhd` creation time (UTC, shown in local time).
pub fn video_date<R: Read + Seek>(r: &mut R, len: u64) -> Option<NaiveDateTime> {
    let top = children(r, 0, len);
    let moov = find(&top, b"moov")?;
    let buf = read_vec(r, moov, 64 << 20)?;
    let n = buf.len() as u64;
    let mut c = Cursor::new(buf);
    let kids = children(&mut c, 0, n);
    if let Some(meta) = find(&kids, b"meta")
        && let Some(d) = apple_creation_date(&mut c, meta)
    {
        return Some(d);
    }
    if let Some(udta) = find(&kids, b"udta") {
        let u = children(&mut c, udta.data, udta.end);
        if let Some(day) = find(&u, b"\xA9day")
            && let Some(v) = read_vec(&mut c, day, 256)
            && v.len() > 4
            && let Some(d) = crate::date::parse_exif(&String::from_utf8_lossy(&v[4..]))
        {
            return Some(d);
        }
    }
    let mvhd = find(&kids, b"mvhd")?;
    let v = read_vec(&mut c, mvhd, 1024)?;
    let secs = match v.first()? {
        1 => u64::from_be_bytes(v.get(4..12)?.try_into().ok()?) as i64,
        _ => u32::from_be_bytes(v.get(4..8)?.try_into().ok()?) as i64,
    };
    let utc = chrono::DateTime::from_timestamp(secs.checked_sub(MAC_EPOCH_OFFSET)?, 0)?;
    let local = utc.with_timezone(&chrono::Local).naive_local();
    let max = chrono::Local::now().naive_local() + chrono::Duration::days(366);
    (secs > MAC_EPOCH_OFFSET + 315_532_800 && local < max).then_some(local) // after 1980
}

fn apple_creation_date(c: &mut Cursor<Vec<u8>>, meta: &BoxHdr) -> Option<NaiveDateTime> {
    let m = meta_children(c, meta);
    let keys = read_vec(c, find(&m, b"keys")?, 1 << 20)?;
    let count = u32::from_be_bytes(keys.get(4..8)?.try_into().ok()?);
    let mut at = 8usize;
    let mut index = None;
    for i in 1..=count {
        let size = u32::from_be_bytes(keys.get(at..at + 4)?.try_into().ok()?) as usize;
        if size < 8 {
            return None;
        }
        if keys.get(at + 8..at + size)? == b"com.apple.quicktime.creationdate" {
            index = Some(i);
            break;
        }
        at += size;
    }
    let index = index?;
    let ilst = *find(&m, b"ilst")?;
    for item in children(c, ilst.data, ilst.end) {
        if u32::from_be_bytes(item.typ) != index {
            continue;
        }
        let data = *find(&children(c, item.data, item.end), b"data")?;
        let v = read_vec(c, &data, 256)?;
        return crate::date::parse_exif(&String::from_utf8_lossy(v.get(8..)?));
    }
    None
}

/// EXIF of a HEIF/HEIC image: the `Exif` item located through `meta/iinf` and `meta/iloc`.
pub fn heif_exif<R: Read + Seek>(r: &mut R, len: u64) -> Option<TiffInfo> {
    let top = children(r, 0, len);
    let meta = find(&top, b"meta")?;
    let buf = read_vec(r, meta, 16 << 20)?;
    let n = buf.len() as u64;
    let mut c = Cursor::new(buf);
    let m = children(&mut c, 4, n);
    let iinf = read_vec(&mut c, find(&m, b"iinf")?, 1 << 20)?;
    let iloc = read_vec(&mut c, find(&m, b"iloc")?, 1 << 20)?;
    let exif_id = exif_item(&iinf)?;
    let (off, _len) = item_extent(&iloc, exif_id)?;
    let mut hdr = [0; 4];
    r.seek(SeekFrom::Start(off)).ok()?;
    r.read_exact(&mut hdr).ok()?;
    let skip = u32::from_be_bytes(hdr) as u64;
    tiff::read(r, off + 4 + skip)
}

fn be(b: &[u8], at: usize, n: usize) -> Option<u64> {
    Some(b.get(at..at + n)?.iter().fold(0u64, |a, x| (a << 8) | *x as u64))
}

fn exif_item(iinf: &[u8]) -> Option<u32> {
    let version = *iinf.first()?;
    let (count, mut at) = if version == 0 { (be(iinf, 4, 2)?, 6) } else { (be(iinf, 4, 4)?, 8) };
    for _ in 0..count {
        let size = be(iinf, at, 4)? as usize;
        if size < 8 || iinf.get(at + 4..at + 8)? != b"infe" {
            return None;
        }
        let v = *iinf.get(at + 8)?;
        let p = at + 12;
        let (id, typ) = match v {
            2 => (be(iinf, p, 2)?, iinf.get(p + 4..p + 8)?),
            3 => (be(iinf, p, 4)?, iinf.get(p + 6..p + 10)?),
            _ => (0, &b"----"[..]),
        };
        if typ == b"Exif" {
            return Some(id as u32);
        }
        at += size;
    }
    None
}

/// File offset and length of the first extent of item `id`.
fn item_extent(iloc: &[u8], id: u32) -> Option<(u64, u64)> {
    let version = *iloc.first()?;
    let offset_size = (*iloc.get(4)? >> 4) as usize;
    let length_size = (*iloc.get(4)? & 15) as usize;
    let base_size = (*iloc.get(5)? >> 4) as usize;
    let index_size = if version >= 1 { (*iloc.get(5)? & 15) as usize } else { 0 };
    let (count, mut at) = if version < 2 { (be(iloc, 6, 2)?, 8) } else { (be(iloc, 6, 4)?, 10) };
    for _ in 0..count {
        let id_size = if version < 2 { 2 } else { 4 };
        let item = be(iloc, at, id_size)? as u32;
        at += id_size;
        let method = if version >= 1 {
            let m = be(iloc, at, 2)? & 15;
            at += 2;
            m
        } else {
            0
        };
        at += 2; // data_reference_index
        let base = be(iloc, at, base_size)?;
        at += base_size;
        let extents = be(iloc, at, 2)?;
        at += 2;
        let mut first = None;
        for _ in 0..extents {
            at += index_size;
            let off = be(iloc, at, offset_size)?;
            at += offset_size;
            let len = be(iloc, at, length_size)?;
            at += length_size;
            first.get_or_insert((base + off, len));
        }
        if item == id && method == 0 {
            return first;
        }
    }
    None
}

const CANON_UUID: [u8; 16] = [0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a, 0x48];
const CANON_PREVIEW_UUID: [u8; 16] = [0xea, 0xf4, 0x2b, 0x5e, 0x1c, 0x98, 0x4b, 0x88, 0xb9, 0xfb, 0xb7, 0xdc, 0x40, 0x6e, 0x4d, 0x16];

/// Canon CR3: EXIF in `moov/uuid/CMT1..CMT2` (TIFF), previews in `THMB` and the top-level
/// `PRVW` uuid box.
pub fn cr3<R: Read + Seek>(r: &mut R, len: u64) -> Option<TiffInfo> {
    let top = children(r, 0, len);
    let mut info = TiffInfo::default();
    if let Some(moov) = find(&top, b"moov") {
        let kids = children(r, moov.data, moov.end);
        if let Some(canon) = kids.iter().find(|b| b.uuid == Some(CANON_UUID)) {
            for b in children(r, canon.data, canon.end) {
                match &b.typ {
                    b"CMT1" | b"CMT2" => {
                        if let Some(t) = tiff::read(r, b.data) {
                            info.merge(t);
                        }
                    }
                    b"THMB" => info.jpegs.extend(find_jpeg(r, b.data, b.end)),
                    _ => {}
                }
            }
        }
    }
    for b in top.iter().filter(|b| b.uuid == Some(CANON_PREVIEW_UUID)) {
        info.jpegs.extend(find_jpeg(r, b.data, b.end));
    }
    Some(info)
}

/// The first JPEG start-of-image marker in the first 256 bytes of a region, to its end.
fn find_jpeg<R: Read + Seek>(r: &mut R, start: u64, end: u64) -> Option<(u64, u64)> {
    let n = (end.saturating_sub(start)).min(256) as usize;
    let mut b = vec![0; n];
    r.seek(SeekFrom::Start(start)).ok()?;
    r.read_exact(&mut b).ok()?;
    let at = b.windows(3).position(|w| w == [0xFF, 0xD8, 0xFF])? as u64;
    Some((start + at, end - start - at))
}
