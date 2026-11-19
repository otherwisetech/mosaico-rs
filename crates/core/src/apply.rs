//! Carry out a plan, and undo it from its journal.
//!
//! Guarantees:
//! - nothing is ever overwritten: files are written to a temporary name and then linked into
//!   place with an operation that fails if the name exists;
//! - copies are verified (size + blake3 re-read from disk) before they count;
//! - a move inside one filesystem is a hard link + unlink of the source; across filesystems it
//!   is copy → fsync → verify → remove the source. Nothing else is ever deleted;
//! - every step is journaled before it happens, so undo works after a crash or a cancel.
use crate::hash::{self, BUF};
use crate::journal::{self, Record, Writer};
use crate::plan::{Mode, Op, Plan};
use std::collections::BTreeMap;
use std::fs::{self, File, FileTimes, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU64};

#[derive(Debug, Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub bytes: AtomicU64,
    pub cancel: AtomicBool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The destination name appeared after the plan was made; nothing was overwritten.
    DestinationExists,
    /// The written copy did not match the source.
    VerifyFailed,
    Io(String),
}

#[derive(Debug, Default)]
pub struct Report {
    pub journal: PathBuf,
    /// Files now at their destination.
    pub done: usize,
    pub bytes: u64,
    /// Identical file found at the destination at apply time.
    pub identical: usize,
    pub failed: Vec<(PathBuf, Failure)>,
    /// Moves whose copy is in place but whose source could not be removed.
    pub source_kept: Vec<(PathBuf, String)>,
    pub cancelled: bool,
}

const PART: &str = ".mosaico-part";

/// Apply `plan`, journaling into a new file in `journal_dir`.
pub fn apply(plan: &Plan, journal_dir: &Path, progress: &Progress) -> io::Result<Report> {
    let mut j = Writer::create(journal_dir)?;
    j.write(&Record::Header { version: journal::VERSION, mode: plan.mode, dest_root: plan.dest_root.clone(), created: chrono::Utc::now().timestamp(), ops: plan.ops.len() })?;
    let mut report = Report { journal: j.path.clone(), ..Default::default() };
    for (i, op) in plan.ops.iter().enumerate() {
        if progress.cancel.load(Relaxed) {
            report.cancelled = true;
            break;
        }
        match apply_one(i, op, plan.mode, &mut j) {
            Ok(Outcome::Placed { source_kept }) => {
                report.done += 1;
                report.bytes += op.size;
                if let Some(e) = source_kept {
                    report.source_kept.push((op.src.clone(), e));
                }
            }
            Ok(Outcome::Identical) => report.identical += 1,
            Err(OpError::Journal(e)) => return Err(e),
            Err(OpError::Op(f)) => report.failed.push((op.src.clone(), f)),
        }
        progress.files.fetch_add(1, Relaxed);
        progress.bytes.fetch_add(op.size, Relaxed);
    }
    Ok(report)
}

enum Outcome {
    Placed { source_kept: Option<String> },
    Identical,
}

enum OpError {
    /// The journal could not be written: stop everything.
    Journal(io::Error),
    Op(Failure),
}

fn io_fail(e: io::Error) -> OpError {
    OpError::Op(Failure::Io(e.to_string()))
}

fn apply_one(i: usize, op: &Op, mode: Mode, j: &mut Writer) -> Result<Outcome, OpError> {
    let parent = op.dst.parent().ok_or_else(|| io_fail(io::Error::from(io::ErrorKind::InvalidInput)))?;
    make_dirs(parent, j)?;
    if fs::symlink_metadata(&op.dst).is_ok() {
        return if hash::same_content(&op.src, &op.dst).unwrap_or(false) { Ok(Outcome::Identical) } else { Err(OpError::Op(Failure::DestinationExists)) };
    }
    let tmp = part_path(&op.dst);
    j.write(&Record::Begin { i, src: op.src.clone(), dst: op.dst.clone(), tmp: tmp.clone() }).map_err(OpError::Journal)?;
    if mode == Mode::Move {
        match fs::hard_link(&op.src, &op.dst) {
            Ok(()) => {
                let size = fs::metadata(&op.dst).map(|m| m.len()).unwrap_or(op.size);
                j.write(&Record::Placed { i, size, hash: None, linked: true }).map_err(OpError::Journal)?;
                return finish_move(i, op, j);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(OpError::Op(Failure::DestinationExists)),
            Err(_) => {} // other filesystem, or no hard links (exFAT): copy instead
        }
    }
    let h = copy_verified(&op.src, &tmp).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        if e.kind() == io::ErrorKind::InvalidData { OpError::Op(Failure::VerifyFailed) } else { io_fail(e) }
    })?;
    match place(&tmp, &op.dst) {
        Ok(()) => {}
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(if e.kind() == io::ErrorKind::AlreadyExists { OpError::Op(Failure::DestinationExists) } else { io_fail(e) });
        }
    }
    j.write(&Record::Placed { i, size: op.size, hash: Some(h.to_hex().to_string()), linked: false }).map_err(OpError::Journal)?;
    match mode {
        Mode::Copy => {
            j.write(&Record::Done { i }).map_err(OpError::Journal)?;
            Ok(Outcome::Placed { source_kept: None })
        }
        Mode::Move => finish_move(i, op, j),
    }
}

fn finish_move(i: usize, op: &Op, j: &mut Writer) -> Result<Outcome, OpError> {
    match fs::remove_file(&op.src) {
        Ok(()) => {
            j.write(&Record::Done { i }).map_err(OpError::Journal)?;
            Ok(Outcome::Placed { source_kept: None })
        }
        Err(e) => Ok(Outcome::Placed { source_kept: Some(e.to_string()) }),
    }
}

/// Create the missing folders of `dir`, top down, journaling each before creating it.
fn make_dirs(dir: &Path, j: &mut Writer) -> Result<(), OpError> {
    let mut missing = Vec::new();
    let mut d = Some(dir);
    while let Some(p) = d {
        if p.exists() {
            break;
        }
        missing.push(p.to_path_buf());
        d = p.parent();
    }
    for p in missing.into_iter().rev() {
        j.write(&Record::Mkdir { dir: p.clone() }).map_err(OpError::Journal)?;
        match fs::create_dir(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io_fail(e)),
        }
    }
    Ok(())
}

/// A temporary name next to `dst` that does not exist yet.
fn part_path(dst: &Path) -> PathBuf {
    let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = dst.parent().unwrap_or(Path::new(""));
    (0..)
        .map(|n| dir.join(if n == 0 { format!(".{name}{PART}") } else { format!(".{name}.{n}{PART}") }))
        .find(|p| fs::symlink_metadata(p).is_err())
        .unwrap()
}

/// Copy `src` to the new file `tmp` (never an existing one), keep the modified time, fsync,
/// then re-read it and check size and blake3 against the source. Returns the hash.
fn copy_verified(src: &Path, tmp: &Path) -> io::Result<blake3::Hash> {
    let mut input = File::open(src)?;
    let meta = input.metadata()?;
    let mut out = OpenOptions::new().write(true).create_new(true).open(tmp)?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0; BUF];
    let mut total = 0u64;
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        out.write_all(&buf[..n])?;
        total += n as u64;
    }
    if let Ok(m) = meta.modified() {
        let _ = out.set_times(FileTimes::new().set_modified(m));
    }
    out.sync_all()?;
    drop(out);
    let want = h.finalize();
    if total != meta.len() || fs::metadata(tmp)?.len() != total || hash::file_hash(tmp)? != want {
        return Err(io::Error::new(io::ErrorKind::InvalidData, tmp.display().to_string()));
    }
    Ok(want)
}

/// Give `tmp` its final name `dst` without ever replacing an existing `dst`: hard link (which
/// fails if `dst` exists) then drop the temporary name. Filesystems without hard links fall
/// back to an existence check + rename.
fn place(tmp: &Path, dst: &Path) -> io::Result<()> {
    match fs::hard_link(tmp, dst) {
        Ok(()) => {
            fs::remove_file(tmp)?;
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(e),
        Err(_) => {
            if fs::symlink_metadata(dst).is_ok() {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            fs::rename(tmp, dst)?;
        }
    }
    sync_dir(dst.parent());
    Ok(())
}

fn sync_dir(dir: Option<&Path>) {
    #[cfg(unix)]
    if let Some(d) = dir
        && let Ok(f) = File::open(d)
    {
        let _ = f.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

// ── undo ───────────────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kept {
    /// The file at the destination changed since it was placed: left alone.
    Changed,
    /// Neither the source nor the destination exists any more.
    Missing,
    /// Copy mode: the original is gone, so the copy is the only one left and is kept.
    SourceMissing,
    Io(String),
}

#[derive(Debug, Default)]
pub struct UndoReport {
    /// Copies removed from the destination (copy mode) or files moved back (move mode).
    pub restored: usize,
    pub kept: Vec<(PathBuf, Kept)>,
    /// Destination folders created by the apply that are now gone again.
    pub dirs_removed: usize,
    /// …and those left because something else is in them.
    pub dirs_left: usize,
}

#[derive(Default)]
struct OpLog {
    src: PathBuf,
    dst: PathBuf,
    tmp: PathBuf,
    placed: Option<(u64, Option<String>, bool)>,
}

/// Undo the apply recorded in `path`: remove the copies it made (only when their content still
/// matches what was written) or move moved files back. Re-running is safe.
pub fn undo(path: &Path, progress: &Progress) -> io::Result<UndoReport> {
    let recs = journal::read(path)?;
    let Some(Record::Header { mode, .. }) = recs.first().cloned() else { return Err(io::Error::new(io::ErrorKind::InvalidData, path.display().to_string())) };
    let mut ops: BTreeMap<usize, OpLog> = BTreeMap::new();
    let mut dirs = Vec::new();
    for r in &recs {
        match r {
            Record::Mkdir { dir } => dirs.push(dir.clone()),
            Record::Begin { i, src, dst, tmp } => {
                ops.insert(*i, OpLog { src: src.clone(), dst: dst.clone(), tmp: tmp.clone(), placed: None });
            }
            Record::Placed { i, size, hash, linked } => {
                if let Some(o) = ops.get_mut(i) {
                    o.placed = Some((*size, hash.clone(), *linked));
                }
            }
            _ => {}
        }
    }
    let mut report = UndoReport::default();
    for o in ops.values().rev() {
        progress.files.fetch_add(1, Relaxed);
        // a temporary copy left by an interrupted operation (always named by us)
        if o.tmp.to_string_lossy().ends_with(PART) && o.tmp.is_file() {
            let _ = fs::remove_file(&o.tmp);
        }
        match undo_one(mode, o) {
            Ok(true) => report.restored += 1,
            Ok(false) => {}
            Err(k) => report.kept.push((o.dst.clone(), k)),
        }
    }
    for d in dirs.iter().rev() {
        if !d.exists() {
            continue;
        }
        match fs::remove_dir(d) {
            Ok(()) => report.dirs_removed += 1,
            Err(_) => report.dirs_left += 1,
        }
    }
    Writer::open_append(path)?.write(&Record::Undone { at: chrono::Utc::now().timestamp() })?;
    Ok(report)
}

/// Whether the destination still holds exactly what this apply put there.
fn ours(o: &OpLog) -> bool {
    match &o.placed {
        Some((_, Some(h), _)) => hash::file_hash(&o.dst).is_ok_and(|d| d.to_hex().as_str() == h),
        Some((size, None, true)) => fs::metadata(&o.dst).is_ok_and(|m| m.len() == *size) && (!o.src.exists() || same_file(&o.src, &o.dst)),
        // crashed between writing and journaling the result: compare with the source
        _ => o.src.exists() && hash::same_content(&o.src, &o.dst).unwrap_or(false),
    }
}

#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_file(a: &Path, b: &Path) -> bool {
    hash::same_content(a, b).unwrap_or(false)
}

fn undo_one(mode: Mode, o: &OpLog) -> Result<bool, Kept> {
    let (src, dst) = (o.src.exists(), fs::symlink_metadata(&o.dst).is_ok());
    if !dst {
        return if src { Ok(false) } else { Err(Kept::Missing) };
    }
    if !ours(o) {
        return Err(Kept::Changed);
    }
    if mode == Mode::Copy {
        // never remove the only copy left: the original must still be there, unchanged
        if !src {
            return Err(Kept::SourceMissing);
        }
        if !hash::same_content(&o.src, &o.dst).unwrap_or(false) {
            return Err(Kept::Changed);
        }
        fs::remove_file(&o.dst).map_err(|e| Kept::Io(e.to_string()))?;
        return Ok(true);
    }
    if src {
        // interrupted move: the source is still there, so the destination is a second copy/link
        if !(same_file(&o.src, &o.dst) || hash::same_content(&o.src, &o.dst).unwrap_or(false)) {
            return Err(Kept::Changed);
        }
        fs::remove_file(&o.dst).map_err(|e| Kept::Io(e.to_string()))?;
        return Ok(true);
    }
    // move back: same procedure as a move, in the other direction
    if let Some(p) = o.src.parent() {
        fs::create_dir_all(p).map_err(|e| Kept::Io(e.to_string()))?;
    }
    match fs::hard_link(&o.dst, &o.src) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(Kept::Io(e.to_string())),
        Err(_) => {
            let tmp = part_path(&o.src);
            copy_verified(&o.dst, &tmp).map_err(|e| {
                let _ = fs::remove_file(&tmp);
                Kept::Io(e.to_string())
            })?;
            place(&tmp, &o.src).map_err(|e| {
                let _ = fs::remove_file(&tmp);
                Kept::Io(e.to_string())
            })?;
        }
    }
    fs::remove_file(&o.dst).map_err(|e| Kept::Io(e.to_string()))?;
    Ok(true)
}
