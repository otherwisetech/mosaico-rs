//! Date and preview extraction from every container, on fixtures built in memory.
use crate::date::DateSource;
use crate::fixtures::{self, bmff::bx, tiff::Val};
use crate::media::Format;
use crate::meta::{self, Meta};
use crate::thumb;
use chrono::{NaiveDate, NaiveDateTime};
use std::io::Cursor;
use std::path::Path;

fn dt(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, s).unwrap()
}

fn meta_of(bytes: &[u8], f: Format) -> Meta {
    meta::read_from(&mut Cursor::new(bytes.to_vec()), bytes.len() as u64, f)
}

const D: &str = "2023:05:14 10:22:33";

#[test]
fn jpeg_exif_date_order() {
    let all = fixtures::exif_block(Some(D), Some("2001:01:01 00:00:00"), Some("2002:02:02 00:00:00"), 1, None);
    let m = meta_of(&fixtures::jpeg_with_exif(&all, 8, 8), Format::Jpeg);
    assert_eq!(m.embedded, Some((dt(2023, 5, 14, 10, 22, 33), DateSource::Exif)));
    // CreateDate when DateTimeOriginal is missing, then DateTime
    let created = fixtures::exif_block(None, Some("2001:01:01 01:02:03"), Some("2002:02:02 00:00:00"), 1, None);
    assert_eq!(meta_of(&fixtures::jpeg_with_exif(&created, 8, 8), Format::Jpeg).embedded.unwrap().0, dt(2001, 1, 1, 1, 2, 3));
    let modified = fixtures::exif_block(None, None, Some("2002:02:02 04:05:06"), 1, None);
    assert_eq!(meta_of(&fixtures::jpeg_with_exif(&modified, 8, 8), Format::Jpeg).embedded.unwrap().0, dt(2002, 2, 2, 4, 5, 6));
    // a zeroed date counts as missing
    let zero = fixtures::exif_block(Some("0000:00:00 00:00:00"), None, None, 1, None);
    assert_eq!(meta_of(&fixtures::jpeg_with_exif(&zero, 8, 8), Format::Jpeg).embedded, None);
    assert_eq!(meta_of(&fixtures::plain_jpeg(8, 8), Format::Jpeg).embedded, None);
}

#[test]
fn jpeg_thumbnail_and_orientation() {
    let thumb = fixtures::plain_jpeg(160, 120);
    let exif = fixtures::exif_block(Some(D), None, None, 6, Some(&thumb));
    let bytes = fixtures::jpeg_with_exif(&exif, 640, 480);
    let m = meta_of(&bytes, Format::Jpeg);
    assert_eq!(m.orientation, 6);
    assert_eq!(m.previews.len(), 1);
    let (o, l) = m.previews[0];
    assert_eq!(&bytes[o as usize..o as usize + l as usize], &thumb[..]);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.jpg");
    std::fs::write(&path, &bytes).unwrap();
    let job = thumb::Job { id: 0, path: path.clone(), size: bytes.len() as u64, modified: None, format: Format::Jpeg, orientation: 6, previews: m.previews.clone() };
    // rotated 90°: the 160×120 preview becomes 120×160
    let img = thumb::render(&job).unwrap();
    assert_eq!((img.width(), img.height()), (120, 160));
    // without a preview the full image is decoded and shrunk to THUMB_PX
    let img = thumb::render(&thumb::Job { previews: vec![], orientation: 1, ..job.clone() }).unwrap();
    assert_eq!((img.width(), img.height()), (256, 192));
    // cached on disk, keyed by path+size+mtime
    let cache = dir.path().join("cache");
    let out = thumb::make(&job, &cache).unwrap().unwrap();
    assert!(out.starts_with(&cache) && out.is_file());
    assert_eq!(thumb::make(&job, &cache).unwrap().unwrap(), out);
    assert_ne!(thumb::cache_key(&path, 1, None), thumb::cache_key(&path, 2, None));
}

#[test]
fn tiff_based_raws() {
    let exif = fixtures::exif_block(Some(D), None, None, 8, None);
    for (magic, f) in [(42u16, Format::Nef), (42, Format::Cr2), (42, Format::Arw), (42, Format::Dng), (0x4F52, Format::Orf), (0x0055, Format::Rw2)] {
        let mut b = exif.clone();
        b[2..4].copy_from_slice(&magic.to_le_bytes());
        let m = meta_of(&b, f);
        assert_eq!(m.embedded, Some((dt(2023, 5, 14, 10, 22, 33), DateSource::Exif)), "{f:?}");
        assert_eq!(m.orientation, 8);
    }
}

#[test]
fn raw_preview_in_strip_and_rw2_inner_jpeg() {
    // CR2-style: IFD0 holds a JPEG-compressed strip (compression 6)
    let preview = fixtures::plain_jpeg(300, 200);
    let mut ifd0 = vec![(0x0103, Val::Short(6)), (0x0111, Val::Long(0)), (0x0117, Val::Long(preview.len() as u32)), (0x0132, Val::Ascii(D.into()))];
    ifd0.sort_by_key(|(t, _)| *t);
    let mut b = fixtures::tiff::tiff(42, ifd0, None);
    let at = b.len() as u32;
    b.extend(&preview);
    // patch StripOffsets (entry #2: tag 0x0111), value at 8 + 2 + 1*12 + 8
    let entry = 8 + 2 + 12 + 8;
    b[entry..entry + 4].copy_from_slice(&at.to_le_bytes());
    let m = meta_of(&b, Format::Cr2);
    assert_eq!(m.previews, vec![(at as u64, preview.len() as u64)]);
    assert_eq!(m.embedded.unwrap().0, dt(2023, 5, 14, 10, 22, 33));

    // RW2-style: no dates in the TIFF, a full JPEG with EXIF in tag 0x002E
    let inner = fixtures::jpeg_with_exif(&fixtures::exif_block(Some("2019:07:08 09:10:11"), None, None, 1, None), 16, 16);
    let mut rw2 = vec![b'I', b'I', 0x55, 0, 8, 0, 0, 0, 1, 0];
    rw2.extend(0x002Eu16.to_le_bytes());
    rw2.extend(7u16.to_le_bytes());
    rw2.extend((inner.len() as u32).to_le_bytes());
    rw2.extend(26u32.to_le_bytes());
    rw2.extend(0u32.to_le_bytes());
    assert_eq!(rw2.len(), 26);
    rw2.extend(&inner);
    let m = meta_of(&rw2, Format::Rw2);
    assert_eq!(m.embedded.unwrap().0, dt(2019, 7, 8, 9, 10, 11));
    assert_eq!(m.previews, vec![(26, inner.len() as u64)]);
}

#[test]
fn heic_exif_item() {
    let exif = fixtures::exif_block(Some(D), None, None, 1, None);
    let m = meta_of(&fixtures::bmff::heif(&exif), Format::Heic);
    assert_eq!(m.embedded, Some((dt(2023, 5, 14, 10, 22, 33), DateSource::Exif)));
    // no embedded JPEG and not decodable: placeholder
    assert!(m.previews.is_empty() && !Format::Heic.decodable());
}

#[test]
fn video_creation_dates() {
    let secs_2023: u32 = (chrono::NaiveDate::from_ymd_opt(2023, 5, 14).unwrap().and_hms_opt(15, 0, 0).unwrap().and_utc().timestamp() + 2_082_844_800) as u32;
    // Apple's local-time key wins over mvhd
    let mov = fixtures::bmff::movie(secs_2023, Some("2023-05-14T10:22:33-0500"));
    assert_eq!(meta_of(&mov, Format::Mov).embedded, Some((dt(2023, 5, 14, 10, 22, 33), DateSource::Video)));
    // mvhd only: UTC shown in local time
    let mp4 = fixtures::bmff::movie(secs_2023, None);
    let want = chrono::DateTime::from_timestamp(secs_2023 as i64 - 2_082_844_800, 0).unwrap().with_timezone(&chrono::Local).naive_local();
    assert_eq!(meta_of(&mp4, Format::Mp4).embedded, Some((want, DateSource::Video)));
    // an unset creation time (1904) is no date
    assert_eq!(meta_of(&fixtures::bmff::movie(0, None), Format::Mp4).embedded, None);
}

#[test]
fn png_webp_raf_cr3() {
    let exif = fixtures::exif_block(Some(D), None, None, 1, None);
    let want = Some((dt(2023, 5, 14, 10, 22, 33), DateSource::Exif));

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend((13u32).to_be_bytes());
    png.extend(b"IHDR");
    png.extend([0; 17]);
    png.extend((exif.len() as u32).to_be_bytes());
    png.extend(b"eXIf");
    png.extend(&exif);
    png.extend([0; 4]);
    png.extend([0, 0, 0, 0]);
    png.extend(b"IEND");
    png.extend([0; 4]);
    assert_eq!(meta_of(&png, Format::Png).embedded, want);

    let mut chunk = b"EXIF".to_vec();
    let payload = [b"Exif\0\0".as_slice(), &exif].concat();
    chunk.extend((payload.len() as u32).to_le_bytes());
    chunk.extend(&payload);
    if payload.len() % 2 == 1 {
        chunk.push(0);
    }
    let mut webp = b"RIFF".to_vec();
    webp.extend(((chunk.len() + 4) as u32).to_le_bytes());
    webp.extend(b"WEBP");
    webp.extend(&chunk);
    assert_eq!(meta_of(&webp, Format::Webp).embedded, want);

    let inner = fixtures::jpeg_with_exif(&exif, 32, 32);
    let mut raf = b"FUJIFILMCCD-RAW 0201FF383501".to_vec();
    raf.resize(84, 0);
    raf.extend(100u32.to_be_bytes());
    raf.extend((inner.len() as u32).to_be_bytes());
    raf.resize(100, 0);
    raf.extend(&inner);
    let m = meta_of(&raf, Format::Raf);
    assert_eq!(m.embedded, want);
    assert!(m.previews.contains(&(100, inner.len() as u64)));

    // CR3: moov / uuid(Canon) / CMT2 holds the Exif IFD as a TIFF of its own
    let cmt2 = fixtures::tiff::tiff(42, vec![(0x9003, Val::Ascii(D.into()))], None);
    let prvw_jpeg = fixtures::plain_jpeg(40, 30);
    let canon_uuid: [u8; 16] = [0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a, 0x48];
    let prvw_uuid: [u8; 16] = [0xea, 0xf4, 0x2b, 0x5e, 0x1c, 0x98, 0x4b, 0x88, 0xb9, 0xfb, 0xb7, 0xdc, 0x40, 0x6e, 0x4d, 0x16];
    let uuid_box = |u: [u8; 16], inner: &[u8]| bx(b"uuid", &[u.as_slice(), inner].concat());
    let moov = bx(b"moov", &uuid_box(canon_uuid, &[bx(b"CMT1", &fixtures::tiff::tiff(42, vec![(0x0112, Val::Short(1))], None)), bx(b"CMT2", &cmt2)].concat()));
    let prvw = uuid_box(prvw_uuid, &[vec![0; 8], bx(b"PRVW", &[vec![0; 16], prvw_jpeg.clone()].concat())].concat());
    let cr3 = [bx(b"ftyp", b"crx \0\0\0\x01crx isom"), moov, prvw].concat();
    let m = meta_of(&cr3, Format::Cr3);
    assert_eq!(m.embedded, want);
    assert_eq!(m.previews.len(), 1);
    let (o, l) = m.previews[0];
    assert_eq!(&cr3[o as usize..(o + l) as usize], &prvw_jpeg[..]);
}

#[test]
fn fallbacks_filename_then_modified() {
    let none = Meta::default();
    let (d, s) = meta::capture_date(&none, Path::new("/x/PXL_20230514_102233123.jpg"), None);
    assert_eq!((d, s), (dt(2023, 5, 14, 10, 22, 33), DateSource::Filename));
    let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    let (d, s) = meta::capture_date(&none, Path::new("/x/holiday.jpg"), Some(when));
    assert_eq!(s, DateSource::Modified);
    assert!(s.uncertain());
    assert_eq!(d, chrono::DateTime::<chrono::Local>::from(when).naive_local());
    // embedded metadata beats the file name
    let exif = Meta { embedded: Some((dt(2020, 1, 2, 3, 4, 5), DateSource::Exif)), ..Default::default() };
    assert_eq!(meta::capture_date(&exif, Path::new("IMG_20230514_102233.jpg"), None).1, DateSource::Exif);
}
