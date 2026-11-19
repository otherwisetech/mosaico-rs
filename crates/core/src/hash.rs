//! Content hashing (blake3) for identical-file detection and copy verification.
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

pub(crate) const BUF: usize = 1 << 20;

pub fn file_hash(path: &Path) -> io::Result<blake3::Hash> {
    let mut f = File::open(path)?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0; BUF];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok(h.finalize());
        }
        h.update(&buf[..n]);
    }
}

/// Same size and same blake3 hash.
pub fn same_content(a: &Path, b: &Path) -> io::Result<bool> {
    if std::fs::metadata(a)?.len() != std::fs::metadata(b)?.len() {
        return Ok(false);
    }
    Ok(file_hash(a)? == file_hash(b)?)
}
