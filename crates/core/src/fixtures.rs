//! Builders for test fixtures: tiny TIFF/EXIF blocks, JPEGs carrying EXIF, QuickTime movies
//! and HEIF containers. Public (but hidden) so the integration tests can use them too.
#![doc(hidden)]

pub mod tiff {

    
    pub enum Val {
        Ascii(String),
        Short(u16),
        Long(u32),
        /// Pointer to a nested IFD (written after this one).
        Ifd(Vec<(u16, Val)>),
    }

    /// Little-endian TIFF with IFD0 = `ifd0`, optional IFD1 thumbnail JPEG, magic `magic`.
    pub fn tiff(magic: u16, ifd0: Vec<(u16, Val)>, thumb: Option<&[u8]>) -> Vec<u8> {
        let mut out = b"II".to_vec();
        out.extend(magic.to_le_bytes());
        out.extend(8u32.to_le_bytes());
        let next_patch = write_ifd(&mut out, ifd0);
        if let Some(j) = thumb {
            let at = out.len() as u32;
            out[next_patch..next_patch + 4].copy_from_slice(&at.to_le_bytes());
            let jpeg_at = at + 2 + 2 * 12 + 4;
            write_ifd(&mut out, vec![(0x0201, Val::Long(jpeg_at)), (0x0202, Val::Long(j.len() as u32))]);
            assert_eq!(out.len() as u32, jpeg_at);
            out.extend_from_slice(j);
        }
        out
    }

    /// Writes one IFD at the end of `out` (values right after it); returns where its
    /// next-IFD pointer is, for chaining.
    fn write_ifd(out: &mut Vec<u8>, mut entries: Vec<(u16, Val)>) -> usize {
        entries.sort_by_key(|(t, _)| *t);
        let start = out.len();
        let n = entries.len();
        let data_start = start + 2 + n * 12 + 4;
        let mut data: Vec<u8> = Vec::new();
        let mut table = Vec::new();
        let mut nested = Vec::new();
        table.extend((n as u16).to_le_bytes());
        for (tag, v) in entries {
            table.extend(tag.to_le_bytes());
            match v {
                Val::Ascii(s) => {
                    let mut b = s.into_bytes();
                    b.push(0);
                    table.extend(2u16.to_le_bytes());
                    table.extend((b.len() as u32).to_le_bytes());
                    if b.len() <= 4 {
                        b.resize(4, 0);
                        table.extend(b);
                    } else {
                        table.extend(((data_start + data.len()) as u32).to_le_bytes());
                        data.extend(b);
                        if data.len() % 2 == 1 {
                            data.push(0);
                        }
                    }
                }
                Val::Short(s) => {
                    table.extend(3u16.to_le_bytes());
                    table.extend(1u32.to_le_bytes());
                    table.extend(s.to_le_bytes());
                    table.extend([0, 0]);
                }
                Val::Long(l) => {
                    table.extend(4u16.to_le_bytes());
                    table.extend(1u32.to_le_bytes());
                    table.extend(l.to_le_bytes());
                }
                Val::Ifd(sub) => {
                    table.extend(4u16.to_le_bytes());
                    table.extend(1u32.to_le_bytes());
                    nested.push((table.len(), sub));
                    table.extend(0u32.to_le_bytes());
                }
            }
        }
        let next_patch = start + table.len();
        table.extend(0u32.to_le_bytes());
        out.extend(table);
        out.extend(data);
        for (patch, sub) in nested {
            let at = out.len() as u32;
            out[start + patch..start + patch + 4].copy_from_slice(&at.to_le_bytes());
            write_ifd(out, sub);
        }
        next_patch
    }
}

pub mod bmff {
    pub fn bx(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(typ);
        v.extend_from_slice(payload);
        v
    }

    /// A QuickTime-style file with `mvhd` creation time (seconds since 1904) and optionally an
    /// Apple creationdate key.
    pub fn movie(mvhd_secs: u32, apple_date: Option<&str>) -> Vec<u8> {
        let mut mvhd = vec![0, 0, 0, 0];
        mvhd.extend(mvhd_secs.to_be_bytes());
        mvhd.extend(mvhd_secs.to_be_bytes());
        mvhd.extend([0; 88]);
        let mut moov = bx(b"mvhd", &mvhd);
        if let Some(d) = apple_date {
            let key = b"com.apple.quicktime.creationdate";
            let mut keys = vec![0, 0, 0, 0, 0, 0, 0, 1];
            keys.extend(((key.len() + 8) as u32).to_be_bytes());
            keys.extend(b"mdta");
            keys.extend(key);
            let mut data = vec![0, 0, 0, 1, 0, 0, 0, 0];
            data.extend(d.as_bytes());
            let item = bx(&1u32.to_be_bytes(), &bx(b"data", &data));
            let mut meta = bx(b"hdlr", &[0; 24]);
            meta.extend(bx(b"keys", &keys));
            meta.extend(bx(b"ilst", &item));
            moov.extend(bx(b"meta", &meta));
        }
        let mut f = bx(b"ftyp", b"qt  \0\0\0\0qt  ");
        f.extend(bx(b"moov", &moov));
        f.extend(bx(b"mdat", &[0; 16]));
        f
    }

    /// A HEIF-like file whose `meta` box locates an `Exif` item holding `tiff`.
    pub fn heif(tiff: &[u8]) -> Vec<u8> {
        let ftyp = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
        let mut infe = vec![2, 0, 0, 0];
        infe.extend(7u16.to_be_bytes());
        infe.extend([0, 0]);
        infe.extend(b"Exif");
        infe.push(0);
        let mut iinf = vec![0, 0, 0, 0, 0, 1];
        iinf.extend(bx(b"infe", &infe));
        // iloc v0, offset_size 4, length_size 4, base_offset_size 0; offset patched below
        let mut iloc = vec![0, 0, 0, 0, 0x44, 0x00, 0, 1];
        iloc.extend(7u16.to_be_bytes());
        iloc.extend([0, 0]); // data_reference_index
        iloc.extend(1u16.to_be_bytes());
        let off_at_in_iloc = iloc.len();
        iloc.extend(0u32.to_be_bytes());
        let mut item = 6u32.to_be_bytes().to_vec();
        item.extend(b"Exif\0\0");
        item.extend_from_slice(tiff);
        iloc.extend((item.len() as u32).to_be_bytes());
        let mut meta_payload = vec![0, 0, 0, 0];
        meta_payload.extend(bx(b"hdlr", &[0; 24]));
        meta_payload.extend(bx(b"iinf", &iinf));
        let iloc_box_at = meta_payload.len();
        meta_payload.extend(bx(b"iloc", &iloc));
        let meta = bx(b"meta", &meta_payload);
        let mut f = ftyp.clone();
        let meta_at = f.len();
        f.extend(meta);
        let mdat_at = f.len();
        f.extend(bx(b"mdat", &item));
        let item_off = (mdat_at + 8) as u32;
        let patch = meta_at + 8 + iloc_box_at + 8 + off_at_in_iloc;
        f[patch..patch + 4].copy_from_slice(&item_off.to_be_bytes());
        f
    }

}

/// A real (tiny, decodable) JPEG with an EXIF APP1 block made from `tiff`.
pub fn jpeg_with_exif(tiff: &[u8], w: u32, h: u32) -> Vec<u8> {
    let jpeg = plain_jpeg(w, h);
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    out.extend(((tiff.len() + 8) as u16).to_be_bytes());
    out.extend(b"Exif\0\0");
    out.extend_from_slice(tiff);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// A plain JPEG of a horizontal gradient.
pub fn plain_jpeg(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, 128]));
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80).encode_image(&img).unwrap();
    buf
}

/// EXIF TIFF block with DateTimeOriginal / CreateDate / DateTime (any may be None) and an
/// orientation, optionally carrying an IFD1 JPEG thumbnail.
pub fn exif_block(original: Option<&str>, digitized: Option<&str>, datetime: Option<&str>, orientation: u16, thumb: Option<&[u8]>) -> Vec<u8> {
    use tiff::Val;
    let mut exif = Vec::new();
    if let Some(d) = original {
        exif.push((0x9003, Val::Ascii(d.into())));
    }
    if let Some(d) = digitized {
        exif.push((0x9004, Val::Ascii(d.into())));
    }
    let mut ifd0 = vec![(0x0112, Val::Short(orientation))];
    if let Some(d) = datetime {
        ifd0.push((0x0132, Val::Ascii(d.into())));
    }
    if !exif.is_empty() {
        ifd0.push((0x8769, Val::Ifd(exif)));
    }
    tiff::tiff(42, ifd0, thumb)
}
