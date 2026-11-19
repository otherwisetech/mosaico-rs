//! The organization plan: where every file would go. Building a plan only reads the disk; it
//! never creates, copies or moves anything (see `apply`).
//!
//! Files travel in *units*: a photo plus its RAW/JPG twin, Live Photo video and `.xmp` / `.aae`
//! sidecars (same folder, same name stem). A unit always lands in one folder, and when a name is
//! taken the whole unit gets the same ` (2)`, ` (3)`… suffix so the pair stays matched.
use crate::date::DateSource;
use crate::hash;
use crate::scan::Library;
use chrono::{Datelike, NaiveDateTime};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU64};

pub const DEFAULT_TEMPLATE: &str = "{year}/{year}-{month} {month_name}";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Copy,
    Move,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub dest_root: PathBuf,
    pub template: String,
    /// Localized month names, January first (filled into `{month_name}`).
    pub month_names: [String; 12],
    pub mode: Mode,
    /// Organize items whose date only comes from the file's modified time.
    pub include_uncertain: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TemplateError {
    Empty,
    UnknownToken,
    Unbalanced,
}

/// `{year}/{year}-{month} {month_name}` → `2023/2023-05 May`. Components are sanitized for
/// every filesystem; `..`, absolute parts and empty segments are dropped.
pub fn render_template(tpl: &str, d: NaiveDateTime, month_names: &[String; 12]) -> Result<PathBuf, TemplateError> {
    let mut s = String::new();
    let mut rest = tpl;
    while let Some(open) = rest.find(['{', '}']) {
        if rest.as_bytes()[open] == b'}' {
            return Err(TemplateError::Unbalanced);
        }
        s.push_str(&rest[..open]);
        let close = rest[open..].find('}').ok_or(TemplateError::Unbalanced)? + open;
        match &rest[open + 1..close] {
            "year" => s.push_str(&format!("{:04}", d.year())),
            "month" => s.push_str(&format!("{:02}", d.month())),
            "month_name" => s.push_str(&month_names[d.month0() as usize]),
            "day" => s.push_str(&format!("{:02}", d.day())),
            _ => return Err(TemplateError::UnknownToken),
        }
        rest = &rest[close + 1..];
    }
    s.push_str(rest);
    let mut out = PathBuf::new();
    for part in s.split(['/', '\\']) {
        let clean: String = part.chars().map(|c| if matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '-' } else { c }).collect();
        let clean = clean.trim().trim_end_matches('.').trim();
        if clean.is_empty() || clean == "." || clean == ".." {
            continue;
        }
        out.push(clean);
    }
    if out.as_os_str().is_empty() { Err(TemplateError::Empty) } else { Ok(out) }
}

/// One file of a unit; its name is `head` + optional ` (n)` + `tail`.
#[derive(Debug, Clone)]
pub struct Member {
    pub src: PathBuf,
    pub size: u64,
    head: String,
    tail: String,
}

impl Member {
    fn name(&self, n: u32) -> String {
        if n <= 1 { format!("{}{}", self.head, self.tail) } else { format!("{} ({n}){}", self.head, self.tail) }
    }
}

#[derive(Debug, Clone)]
pub struct Unit {
    pub members: Vec<Member>,
    pub taken: NaiveDateTime,
    pub source: DateSource,
}

fn split_name(p: &Path) -> Option<(String, String)> {
    let name = p.file_name()?.to_str()?;
    Some(match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.to_string(), String::new()),
    })
}

/// Group the library into units (media sharing folder + stem, plus matching sidecars).
pub fn units(lib: &Library) -> Vec<Unit> {
    let mut by_key: HashMap<(PathBuf, String), usize> = HashMap::new();
    let mut units: Vec<Unit> = Vec::new();
    for it in &lib.items {
        let Some((head, tail)) = split_name(&it.path) else { continue };
        let key = (it.path.parent().map(Path::to_path_buf).unwrap_or_default(), head.to_lowercase());
        let m = Member { src: it.path.clone(), size: it.size, head, tail };
        match by_key.get(&key) {
            Some(&u) => {
                let unit = &mut units[u];
                unit.members.push(m);
                // the unit's date is its best-sourced member's
                if it.source < unit.source {
                    unit.source = it.source;
                    unit.taken = it.taken;
                }
            }
            None => {
                by_key.insert(key, units.len());
                units.push(Unit { members: vec![m], taken: it.taken, source: it.source });
            }
        }
    }
    for sc in &lib.sidecars {
        let (Some((head, tail)), Some(parent)) = (split_name(sc), sc.parent()) else { continue };
        let size = std::fs::metadata(sc).map(|m| m.len()).unwrap_or(0);
        // IMG_1.xmp → stem IMG_1; IMG_1.CR2.xmp → stem IMG_1 when a member is named IMG_1.CR2
        if let Some(&u) = by_key.get(&(parent.to_path_buf(), head.to_lowercase())) {
            units[u].members.push(Member { src: sc.clone(), size, head, tail });
        } else if let Some((inner_head, inner_tail)) = split_name(Path::new(&head))
            && let Some(&u) = by_key.get(&(parent.to_path_buf(), inner_head.to_lowercase()))
            && units[u].members.iter().any(|m| m.name(1).eq_ignore_ascii_case(&head))
        {
            units[u].members.push(Member { src: sc.clone(), size, head: inner_head, tail: format!("{inner_tail}{tail}") });
        }
    }
    units.sort_by(|a, b| a.taken.cmp(&b.taken).then_with(|| a.members[0].src.cmp(&b.members[0].src)));
    units
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SkipReason {
    /// The same content (size + blake3) is already at the destination.
    Identical,
    /// Another source file with the same content is already planned for that destination.
    DuplicateInSources,
    /// The file already is where the plan would put it.
    AlreadyInPlace,
    /// Only the modified time dates it, and uncertain dates were excluded.
    UncertainDate,
}

#[derive(Debug, Clone)]
pub struct Op {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub size: u64,
    /// Got a ` (n)` suffix because the name was taken.
    pub renamed: bool,
    pub uncertain: bool,
}

#[derive(Debug, Clone)]
pub struct Skipped {
    pub src: PathBuf,
    pub dst: Option<PathBuf>,
    pub reason: SkipReason,
}

/// Destination folder (relative to the root) and the ops landing in it.
#[derive(Debug, Clone)]
pub struct Folder {
    pub dir: PathBuf,
    pub ops: Vec<usize>,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub mode: Mode,
    pub dest_root: PathBuf,
    /// Sorted by destination.
    pub ops: Vec<Op>,
    pub skipped: Vec<Skipped>,
    pub folders: Vec<Folder>,
}

impl Plan {
    pub fn bytes(&self) -> u64 {
        self.ops.iter().map(|o| o.size).sum()
    }

    pub fn renamed(&self) -> usize {
        self.ops.iter().filter(|o| o.renamed).count()
    }

    pub fn skipped_for(&self, r: SkipReason) -> usize {
        self.skipped.iter().filter(|s| s.reason == r).count()
    }
}

#[derive(Debug, Default)]
pub struct Progress {
    pub units: AtomicU64,
    pub cancel: AtomicBool,
}

enum State {
    Free,
    Identical,
    DuplicatePlanned,
    InPlace,
    Conflict,
}

struct Ctx {
    planned: HashMap<String, (PathBuf, u64)>,
    hashes: HashMap<PathBuf, Option<blake3::Hash>>,
}

impl Ctx {
    fn hash(&mut self, p: &Path) -> Option<blake3::Hash> {
        *self.hashes.entry(p.to_path_buf()).or_insert_with(|| hash::file_hash(p).ok())
    }

    fn same(&mut self, a: &Path, b: &Path) -> bool {
        match (self.hash(a), self.hash(b)) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        }
    }

    fn state(&mut self, target: &Path, m: &Member) -> State {
        if target == m.src {
            return State::InPlace;
        }
        let key = target.to_string_lossy().to_lowercase();
        if let Some((other, size)) = self.planned.get(&key).cloned() {
            return if size == m.size && self.same(&other, &m.src) { State::DuplicatePlanned } else { State::Conflict };
        }
        match std::fs::symlink_metadata(target) {
            Ok(md) if md.is_file() && md.len() == m.size && self.same(target, &m.src) => State::Identical,
            Ok(_) => State::Conflict,
            Err(e) if e.kind() == io::ErrorKind::NotFound => State::Free,
            Err(_) => State::Conflict,
        }
    }
}

/// The destination root as a canonical path when it exists (so files already in place are
/// recognised), else as given.
fn dest_root(p: &Path) -> io::Result<PathBuf> {
    if p.exists() {
        return std::fs::canonicalize(p);
    }
    if !p.is_absolute() || p.components().any(|c| c == Component::ParentDir) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, p.display().to_string()));
    }
    Ok(p.to_path_buf())
}

/// Work out the plan. Reads metadata (and hashes same-size files at the destination) only.
pub fn build(lib: &Library, opts: &Options, progress: &Progress) -> io::Result<Plan> {
    let root = dest_root(&opts.dest_root)?;
    render_template(&opts.template, NaiveDateTime::default(), &opts.month_names).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("{e:?}")))?;
    let mut ctx = Ctx { planned: HashMap::new(), hashes: HashMap::new() };
    let mut ops = Vec::new();
    let mut skipped = Vec::new();
    for unit in units(lib) {
        if progress.cancel.load(Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled")); // i18n-ok: internal marker
        }
        progress.units.fetch_add(1, Relaxed);
        let uncertain = unit.source.uncertain();
        if uncertain && !opts.include_uncertain {
            skipped.extend(unit.members.iter().map(|m| Skipped { src: m.src.clone(), dst: None, reason: SkipReason::UncertainDate }));
            continue;
        }
        let dir = root.join(render_template(&opts.template, unit.taken, &opts.month_names).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("{e:?}")))?);
        let mut n = 1;
        let states = loop {
            let states: Vec<State> = unit.members.iter().map(|m| ctx.state(&dir.join(m.name(n)), m)).collect();
            if !states.iter().any(|s| matches!(s, State::Conflict)) || n >= 9999 {
                break states;
            }
            n += 1;
        };
        for (m, st) in unit.members.iter().zip(states) {
            let dst = dir.join(m.name(n));
            let reason = match st {
                State::Free => {
                    ctx.planned.insert(dst.to_string_lossy().to_lowercase(), (m.src.clone(), m.size));
                    ops.push(Op { src: m.src.clone(), dst, size: m.size, renamed: n > 1, uncertain });
                    continue;
                }
                State::Identical => SkipReason::Identical,
                State::DuplicatePlanned => SkipReason::DuplicateInSources,
                State::InPlace => SkipReason::AlreadyInPlace,
                State::Conflict => continue, // only after 9999 attempts: leave the file alone
            };
            skipped.push(Skipped { src: m.src.clone(), dst: Some(dst), reason });
        }
    }
    ops.sort_by(|a, b| (a.dst.parent(), a.dst.file_name()).cmp(&(b.dst.parent(), b.dst.file_name())));
    let mut folders: Vec<Folder> = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        let dir = op.dst.parent().and_then(|p| p.strip_prefix(&root).ok()).map(Path::to_path_buf).unwrap_or_default();
        match folders.last_mut() {
            Some(f) if f.dir == dir => {
                f.ops.push(i);
                f.bytes += op.size;
            }
            _ => folders.push(Folder { dir, ops: vec![i], bytes: op.size }),
        }
    }
    Ok(Plan { mode: opts.mode, dest_root: root, ops, skipped, folders })
}
