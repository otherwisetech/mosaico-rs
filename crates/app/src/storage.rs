//! Where mosaico keeps its own files: `<data root>/mosaico/` (`~/.otherwise/mosaico/` unless the
//! user moved the root in Otherwise Hub) holds the undo journals, `people.json`, settings and
//! recent folders. Before 0.3 they were in the platform folders (`dirs::legacy_*`); [`init`]
//! moves them over once. Photos are never touched here.
//!
//! Journals are what undo needs, so the move never loses one: each file is renamed, or copied,
//! verified and only then deleted (otherwise-link's `storage::move_any`); a journal whose name is
//! already taken at the new place gets a free name instead of being left behind; nothing is
//! ever overwritten.
use mosaico_core::dirs;
use otherwise_link::storage;
use std::io;
use std::path::{Path, PathBuf};

/// Left in the old folder once its files moved (names the new folder).
pub const MARKER: &str = ".migrated-to-otherwise";

/// Points mosaico-core at `<data root>/mosaico/` and moves the old folders' files there (not
/// when `OTHERWISE_DATA_DIR` is set: tests, portable setups). Call before anything reads them.
pub fn init() {
    let new = otherwise_link::paths::app_dir(crate::otherwise::APP_ID);
    let skip = std::env::var_os(otherwise_link::paths::ENV).is_some_and(|v| !v.is_empty());
    if !skip {
        // config first: on Linux it's its own folder; elsewhere the same one (then AlreadyDone)
        for old in [dirs::legacy_data_dir(), dirs::legacy_config_dir()].into_iter().flatten() {
            match migrate(&old, &new) {
                Ok(Migrated::Done { kept, .. }) if !kept.is_empty() => eprintln!("mosaico: left in {}: {}", old.display(), kept.join(", ")), // i18n-ok: log only
                Ok(_) => {}
                Err(e) => eprintln!("mosaico: moving {} failed: {e}", old.display()), // i18n-ok: log only
            }
        }
    }
    dirs::set_app_dir(new);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Migrated {
    Nothing,
    AlreadyDone,
    /// Moved (relative paths, as named at the new place); `kept` stayed in the old folder
    /// because the new one already had a different file of that name.
    Done { moved: Vec<String>, kept: Vec<String> },
}

/// Moves every file of `old` into `new` once, leaving [`MARKER`] behind. A file already at the
/// new place with the same bytes is a duplicate (the old one is removed); a different one is
/// never overwritten: a journal moves under a free name, anything else stays in `old`.
pub fn migrate(old: &Path, new: &Path) -> io::Result<Migrated> {
    if !old.is_dir() || old == new {
        return Ok(Migrated::Nothing);
    }
    if old.join(MARKER).exists() {
        return Ok(Migrated::AlreadyDone);
    }
    let (mut moved, mut kept) = (Vec::new(), Vec::new());
    move_files(old, new, "", &mut moved, &mut kept)?;
    otherwise_link::fsutil::write_atomic(&old.join(MARKER), format!("{}\n", new.display()).as_bytes())?;
    Ok(Migrated::Done { moved, kept })
}

fn same_bytes(a: &Path, b: &Path) -> io::Result<bool> {
    Ok(std::fs::read(a)? == std::fs::read(b)?)
}

/// `name-2.jsonl`, `name-3.jsonl`… the first that doesn't exist in `dir`.
fn free_name(dir: &Path, name: &str) -> String {
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (2..).map(|n| if ext.is_empty() { format!("{stem}-{n}") } else { format!("{stem}-{n}.{ext}") }).find(|c| !dir.join(c).exists()).unwrap_or_default()
}

fn move_files(from: &Path, to: &Path, rel: &str, moved: &mut Vec<String>, kept: &mut Vec<String>) -> io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(from)?.collect::<io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let join = |n: &str| if rel.is_empty() { n.to_string() } else { format!("{rel}/{n}") };
        let t = e.file_type()?;
        if t.is_dir() {
            move_files(&e.path(), &to.join(&name), &join(&name), moved, kept)?;
            let _ = std::fs::remove_dir(e.path());
        } else if t.is_file() && name != MARKER && !storage::skipped_in_copy(&name) {
            let mut dest_name = name.clone();
            if to.join(&name).exists() {
                if same_bytes(&e.path(), &to.join(&name))? {
                    std::fs::remove_file(e.path())?;
                    continue;
                }
                if !name.ends_with(".jsonl") {
                    kept.push(join(&name));
                    continue;
                }
                dest_name = free_name(to, &name);
            }
            storage::move_any(&e.path(), &to.join(&dest_name))?;
            moved.push(join(&dest_name));
        }
    }
    Ok(())
}

/// Temp folders are never remembered as a destination.
pub fn is_temp(p: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let (p, tmp) = (canon(p), canon(&std::env::temp_dir()));
    p.starts_with(&tmp) || ["/tmp", "/private/tmp", "/var/folders", "/private/var/folders"].iter().any(|t| p.starts_with(t)) // i18n-ok: paths
}

fn last_dest_file() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("last-destination.txt"))
}

/// The destination chosen last time, if it still exists (and isn't a temp folder).
pub fn last_dest() -> Option<PathBuf> {
    read_last_dest(&last_dest_file()?)
}

pub fn read_last_dest(file: &Path) -> Option<PathBuf> {
    let p = PathBuf::from(std::fs::read_to_string(file).ok()?.trim());
    (p.is_absolute() && p.is_dir() && !is_temp(&p)).then_some(p)
}

pub fn remember_dest(dest: &Path) {
    if let Some(f) = last_dest_file() {
        write_last_dest(&f, dest);
    }
}

pub fn write_last_dest(file: &Path, dest: &Path) {
    if is_temp(dest) {
        return;
    }
    if let Err(e) = otherwise_link::fsutil::write_atomic(file, dest.display().to_string().as_bytes()) {
        eprintln!("mosaico: {}: {e}", file.display()); // i18n-ok: log only
    }
}

/// Copies of `people.json` a sync client left next to it.
pub fn conflicted_copies(file: &Path) -> Vec<PathBuf> {
    otherwise_link::cloud::conflicted_copies(file)
}

/// «Show in Finder» / «Show in Explorer» / «Show in file manager».
pub fn reveal_label() -> String {
    if cfg!(target_os = "macos") {
        i18n::t!("app.file.reveal_macos")
    } else if cfg!(windows) {
        i18n::t!("app.file.reveal_windows")
    } else {
        i18n::t!("app.file.reveal_linux")
    }
}

/// Shows mosaico's data folder (journals, people.json) in the file manager.
pub fn reveal_data_dir(cx: &mut gpui::App) {
    if let Some(d) = dirs::data_dir() {
        let _ = std::fs::create_dir_all(&d);
        cx.reveal_path(&d);
    }
}

#[cfg(test)]
mod tests {
    use super::{MARKER, Migrated, migrate, read_last_dest, write_last_dest};

    #[test]
    fn migration_keeps_every_journal_and_never_overwrites() {
        let t = tempfile::tempdir().unwrap();
        let (old, new) = (t.path().join("old"), t.path().join("root").join("mosaico"));
        assert_eq!(migrate(&old, &new).unwrap(), Migrated::Nothing);
        std::fs::create_dir_all(old.join("journals")).unwrap();
        std::fs::create_dir_all(new.join("journals")).unwrap();
        std::fs::write(old.join("journals").join("20260101-120000.jsonl"), b"{\"a\":1}\n").unwrap();
        std::fs::write(old.join("journals").join("20260102-120000.jsonl"), b"{\"old\":1}\n").unwrap();
        std::fs::write(old.join("journals").join("20260103-120000.jsonl"), b"same\n").unwrap();
        std::fs::write(old.join("people.json"), b"{\"schema\":1,\"old\":true}").unwrap();
        std::fs::write(old.join("settings.json"), b"{}").unwrap();
        // the new place already has: a different journal of one name, an identical one, people.json
        std::fs::write(new.join("journals").join("20260102-120000.jsonl"), b"{\"new\":1}\n").unwrap();
        std::fs::write(new.join("journals").join("20260103-120000.jsonl"), b"same\n").unwrap();
        std::fs::write(new.join("people.json"), b"{\"schema\":1,\"new\":true}").unwrap();
        let m = migrate(&old, &new).unwrap();
        assert_eq!(
            m,
            Migrated::Done { moved: vec!["journals/20260101-120000.jsonl".into(), "journals/20260102-120000-2.jsonl".into(), "settings.json".into()], kept: vec!["people.json".into()] }
        );
        let j = new.join("journals");
        assert_eq!(std::fs::read(j.join("20260101-120000.jsonl")).unwrap(), b"{\"a\":1}\n");
        assert_eq!(std::fs::read(j.join("20260102-120000.jsonl")).unwrap(), b"{\"new\":1}\n");
        assert_eq!(std::fs::read(j.join("20260102-120000-2.jsonl")).unwrap(), b"{\"old\":1}\n");
        assert_eq!(std::fs::read(j.join("20260103-120000.jsonl")).unwrap(), b"same\n");
        // people.json at the new place wins and the old one is kept, not deleted
        assert_eq!(std::fs::read(new.join("people.json")).unwrap(), b"{\"schema\":1,\"new\":true}");
        assert_eq!(std::fs::read(old.join("people.json")).unwrap(), b"{\"schema\":1,\"old\":true}");
        assert!(!old.join("journals").exists());
        assert!(old.join(MARKER).exists());
        assert_eq!(migrate(&old, &new).unwrap(), Migrated::AlreadyDone);
    }

    #[test]
    fn last_destination_skips_temp_and_missing() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("last-destination.txt");
        assert_eq!(read_last_dest(&f), None);
        // a temp folder is never remembered
        write_last_dest(&f, t.path());
        assert!(!f.exists());
        std::fs::write(&f, "/definitely/not/here").unwrap();
        assert_eq!(read_last_dest(&f), None);
        if let Some(h) = std::env::var_os("HOME").map(std::path::PathBuf::from).filter(|h| h.is_dir() && !super::is_temp(h)) {
            write_last_dest(&f, &h);
            assert_eq!(read_last_dest(&f), Some(h));
        }
    }
}
