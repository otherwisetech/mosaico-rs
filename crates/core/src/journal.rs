//! Undo journals: one JSON-lines file per apply, in the app's data folder. Every record is
//! flushed to disk *before* the file operation it announces, so a crash at any point leaves
//! enough information to put things back.
use crate::plan::Mode;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Record {
    Header { version: u32, mode: Mode, dest_root: PathBuf, created: i64, ops: usize },
    /// A destination folder about to be created by this apply.
    Mkdir { dir: PathBuf },
    /// About to copy/move `src` to `dst`, writing through `tmp` first.
    Begin { i: usize, src: PathBuf, dst: PathBuf, tmp: PathBuf },
    /// `dst` now holds the file: verified blake3 `hash` (copies) or a hard link (`linked`).
    Placed { i: usize, size: u64, hash: Option<String>, linked: bool },
    /// Finished (for a move: the source was removed).
    Done { i: usize },
    /// This journal was undone.
    Undone { at: i64 },
}

pub struct Writer {
    file: File,
    pub path: PathBuf,
}

impl Writer {
    /// A new journal file in `dir` named after the current local time.
    pub fn create(dir: &Path) -> io::Result<Writer> {
        fs::create_dir_all(dir)?;
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f").to_string();
        for n in 0..1000 {
            let path = dir.join(if n == 0 { format!("{stamp}.jsonl") } else { format!("{stamp}-{n}.jsonl") });
            match OpenOptions::new().append(true).create_new(true).open(&path) {
                Ok(file) => return Ok(Writer { file, path }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, dir.display().to_string()))
    }

    pub fn open_append(path: &Path) -> io::Result<Writer> {
        Ok(Writer { file: OpenOptions::new().append(true).open(path)?, path: path.to_path_buf() })
    }

    /// Append one record and flush it to disk.
    pub fn write(&mut self, r: &Record) -> io::Result<()> {
        let mut line = serde_json::to_string(r).map_err(io::Error::other)?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.file.sync_data()
    }
}

/// All readable records. A torn last line (crash while writing it) is ignored.
pub fn read(path: &Path) -> io::Result<Vec<Record>> {
    let mut out = Vec::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(&line) {
            Ok(r) => out.push(r),
            Err(_) => break,
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub path: PathBuf,
    pub mode: Mode,
    pub dest_root: PathBuf,
    /// Unix seconds.
    pub created: i64,
    /// Files placed at the destination.
    pub placed: usize,
    pub undone: bool,
}

pub fn summary(path: &Path) -> Option<Summary> {
    let recs = read(path).ok()?;
    let Some(Record::Header { mode, dest_root, created, .. }) = recs.first().cloned() else { return None };
    let placed = recs.iter().filter(|r| matches!(r, Record::Placed { .. })).count();
    let undone = recs.iter().any(|r| matches!(r, Record::Undone { .. }));
    Some(Summary { path: path.to_path_buf(), mode, dest_root, created, placed, undone })
}

/// The newest journal in `dir` that placed at least one file, unless it was already undone.
pub fn latest_undoable(dir: &Path) -> Option<Summary> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "jsonl")).collect();
    files.sort();
    files.iter().rev().filter_map(|p| summary(p)).find(|s| s.placed > 0).filter(|s| !s.undone)
}
