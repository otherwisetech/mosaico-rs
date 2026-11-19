//! Find media under the chosen source folders and read each one's capture date.
use crate::date::DateSource;
use crate::media::{self, Format};
use crate::meta;
use chrono::NaiveDateTime;
use rayon::prelude::*;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::time::SystemTime;

/// Live counters for the scan screen; set `cancel` to stop early.
#[derive(Debug, Default)]
pub struct Progress {
    /// Media files found so far.
    pub found: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    /// Media files whose metadata has been read.
    pub read: AtomicU64,
    pub errors: AtomicU64,
    pub cancel: AtomicBool,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub format: Format,
    pub taken: NaiveDateTime,
    pub source: DateSource,
    pub orientation: u16,
    /// Embedded JPEG previews (offset, length), for thumbnails.
    pub previews: Vec<(u64, u64)>,
}

impl Item {
    pub fn uncertain(&self) -> bool {
        self.source.uncertain()
    }
}

#[derive(Debug, Default)]
pub struct Library {
    /// Source folders, canonical, without nested duplicates.
    pub roots: Vec<PathBuf>,
    /// Sorted by capture date, then path.
    pub items: Vec<Item>,
    /// `.xmp` / `.aae` files found next to the media.
    pub sidecars: Vec<PathBuf>,
    /// Unreadable folders or files.
    pub errors: u64,
}

impl Library {
    pub fn total_bytes(&self) -> u64 {
        self.items.iter().map(|i| i.size).sum()
    }

    pub fn count_by_source(&self, s: DateSource) -> usize {
        self.items.iter().filter(|i| i.source == s).count()
    }
}

/// Canonical roots with any root inside another one dropped (it would be scanned twice).
pub fn normalize_roots(roots: &[PathBuf]) -> io::Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = Vec::new();
    for r in roots {
        let c = fs::canonicalize(r)?;
        if !fs::metadata(&c)?.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotADirectory, c.display().to_string()));
        }
        out.push(c);
    }
    out.sort();
    out.dedup();
    let all = out.clone();
    out.retain(|r| !all.iter().any(|o| o != r && r.starts_with(o)));
    Ok(out)
}

/// Scan every root. Hidden folders, app bundles and photo-library packages are never entered;
/// symlinks are not followed. Cancelling returns `ErrorKind::Interrupted`.
pub fn scan(roots: &[PathBuf], progress: &Progress) -> io::Result<Library> {
    let roots = normalize_roots(roots)?;
    let mut lib = Library { roots: roots.clone(), ..Default::default() };
    let mut found: Vec<(PathBuf, u64, Option<SystemTime>, Format)> = Vec::new();
    for root in &roots {
        walk(root, progress, &mut found, &mut lib.sidecars)?;
    }
    let cancelled = || progress.cancel.load(Relaxed);
    if cancelled() {
        return Err(interrupted());
    }
    let items: Vec<Option<Item>> = found
        .into_par_iter()
        .map(|(path, size, modified, format)| {
            if cancelled() {
                return None;
            }
            let m = meta::read(&path, format);
            let (taken, source) = meta::capture_date(&m, &path, modified);
            progress.read.fetch_add(1, Relaxed);
            Some(Item { path, size, modified, format, taken, source, orientation: m.orientation, previews: m.previews })
        })
        .collect();
    if cancelled() {
        return Err(interrupted());
    }
    lib.items = items.into_iter().flatten().collect();
    lib.items.sort_by(|a, b| a.taken.cmp(&b.taken).then_with(|| a.path.cmp(&b.path)));
    lib.sidecars.sort();
    lib.errors = progress.errors.load(Relaxed);
    Ok(lib)
}

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "cancelled") // i18n-ok: internal marker
}

fn walk(root: &Path, progress: &Progress, out: &mut Vec<(PathBuf, u64, Option<SystemTime>, Format)>, sidecars: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if progress.cancel.load(Relaxed) {
            return Ok(());
        }
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if dir == root => return Err(e),
            Err(_) => {
                progress.errors.fetch_add(1, Relaxed);
                continue;
            }
        };
        progress.dirs.fetch_add(1, Relaxed);
        for entry in rd {
            let Ok(entry) = entry else {
                progress.errors.fetch_add(1, Relaxed);
                continue;
            };
            let Ok(ft) = entry.file_type() else { continue };
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if ft.is_dir() {
                let path = entry.path();
                if !media::skip_dir(name) && !media::is_package(&path) {
                    stack.push(path);
                }
            } else if ft.is_file() && !name.starts_with('.') {
                let path = entry.path();
                if let Some(format) = Format::from_path(&path) {
                    let Ok(md) = entry.metadata() else {
                        progress.errors.fetch_add(1, Relaxed);
                        continue;
                    };
                    progress.found.fetch_add(1, Relaxed);
                    progress.bytes.fetch_add(md.len(), Relaxed);
                    out.push((path, md.len(), md.modified().ok(), format));
                } else if media::is_sidecar(&path) {
                    sidecars.push(path);
                }
            }
        }
    }
    Ok(())
}
