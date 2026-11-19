//! Which files are photos, RAWs, videos or sidecars, by extension.
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Kind {
    Photo,
    Raw,
    Video,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Format {
    Jpeg,
    Png,
    Heic,
    Gif,
    Webp,
    Tiff,
    Cr2,
    Cr3,
    Nef,
    Arw,
    Dng,
    Raf,
    Orf,
    Rw2,
    Mov,
    Mp4,
}

impl Format {
    pub const ALL: [Format; 16] = [
        Format::Jpeg,
        Format::Png,
        Format::Heic,
        Format::Gif,
        Format::Webp,
        Format::Tiff,
        Format::Cr2,
        Format::Cr3,
        Format::Nef,
        Format::Arw,
        Format::Dng,
        Format::Raf,
        Format::Orf,
        Format::Rw2,
        Format::Mov,
        Format::Mp4,
    ];

    pub fn from_path(path: &Path) -> Option<Format> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "jpg" | "jpeg" | "jpe" => Format::Jpeg,
            "png" => Format::Png,
            "heic" | "heif" | "hif" => Format::Heic,
            "gif" => Format::Gif,
            "webp" => Format::Webp,
            "tif" | "tiff" => Format::Tiff,
            "cr2" => Format::Cr2,
            "cr3" => Format::Cr3,
            "nef" => Format::Nef,
            "arw" => Format::Arw,
            "dng" => Format::Dng,
            "raf" => Format::Raf,
            "orf" => Format::Orf,
            "rw2" => Format::Rw2,
            "mov" => Format::Mov,
            "mp4" | "m4v" => Format::Mp4,
            _ => return None,
        })
    }

    pub fn kind(self) -> Kind {
        match self {
            Format::Cr2 | Format::Cr3 | Format::Nef | Format::Arw | Format::Dng | Format::Raf | Format::Orf | Format::Rw2 => Kind::Raw,
            Format::Mov | Format::Mp4 => Kind::Video,
            _ => Kind::Photo,
        }
    }

    /// Short uppercase label for placeholder tiles and summaries.
    pub fn label(self) -> &'static str {
        match self {
            Format::Jpeg => "JPG",
            Format::Png => "PNG",
            Format::Heic => "HEIC",
            Format::Gif => "GIF",
            Format::Webp => "WEBP",
            Format::Tiff => "TIFF",
            Format::Cr2 => "CR2",
            Format::Cr3 => "CR3",
            Format::Nef => "NEF",
            Format::Arw => "ARW",
            Format::Dng => "DNG",
            Format::Raf => "RAF",
            Format::Orf => "ORF",
            Format::Rw2 => "RW2",
            Format::Mov => "MOV",
            Format::Mp4 => "MP4",
        }
    }

    /// Whether the `image` crate can decode the whole file when there is no embedded preview.
    pub fn decodable(self) -> bool {
        matches!(self, Format::Jpeg | Format::Png | Format::Gif | Format::Webp | Format::Tiff)
    }
}

/// Edit/metadata files that travel with their photo: `IMG_1.xmp`, `IMG_1.CR2.xmp`, `IMG_1.AAE`.
pub fn is_sidecar(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("xmp") || e.eq_ignore_ascii_case("aae"))
}

/// Directories never entered: hidden ones, app bundles and photo-library packages (their
/// insides belong to the app that owns them), plus well-known system/thumbnail folders.
pub fn skip_dir(name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }
    const PACKAGES: &[&str] = &["photoslibrary", "photolibrary", "migratedphotolibrary", "aplibrary", "lrdata", "lrlibrary", "app", "bundle", "framework", "plugin", "kext", "imovielibrary", "fcpbundle", "tvlibrary", "theater"];
    if let Some((_, ext)) = name.rsplit_once('.')
        && PACKAGES.iter().any(|p| ext.eq_ignore_ascii_case(p))
    {
        return true;
    }
    const SYSTEM: &[&str] = &["$RECYCLE.BIN", "System Volume Information", "@eaDir", "node_modules", "Library"];
    SYSTEM.contains(&name)
}

/// macOS packages without a telling extension carry Finder's "has bundle" flag
/// (`com.apple.FinderInfo`, flags byte 8 bit 0x20): Photo Booth Library, some app libraries.
#[cfg(target_os = "macos")]
pub fn is_package(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    let mut buf = [0u8; 32];
    // SAFETY: valid NUL-terminated strings and a buffer of the given length.
    let n = unsafe { libc::getxattr(c.as_ptr(), c"com.apple.FinderInfo".as_ptr(), buf.as_mut_ptr().cast(), buf.len(), 0, libc::XATTR_NOFOLLOW) };
    n >= 10 && buf[8] & 0x20 != 0
}

#[cfg(not(target_os = "macos"))]
pub fn is_package(_path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_skips() {
        assert_eq!(Format::from_path(Path::new("a/IMG_1.JPG")), Some(Format::Jpeg));
        assert_eq!(Format::from_path(Path::new("x.HEIC")).map(Format::kind), Some(Kind::Photo));
        assert_eq!(Format::from_path(Path::new("x.cr3")).map(Format::kind), Some(Kind::Raw));
        assert_eq!(Format::from_path(Path::new("x.m4v")).map(Format::kind), Some(Kind::Video));
        assert_eq!(Format::from_path(Path::new("notes.txt")), None);
        assert!(is_sidecar(Path::new("IMG_1.CR2.xmp")));
        assert!(is_sidecar(Path::new("IMG_1.AAE")));
        assert!(skip_dir(".git") && skip_dir("Photos Library.photoslibrary") && skip_dir("Foo.app") && skip_dir("Library"));
        assert!(!skip_dir("2023 Trip") && !skip_dir("DCIM"));
    }
}
