//! mosaico's folders. Since 0.3 the app keeps its data (undo journals, `people.json`,
//! settings, recent folders) in the Otherwise data folder, `<data root>/mosaico/`; the app sets
//! it with [`set_app_dir`] at startup (the core stays free of otherwise-link). Without it (core
//! tests, the `dryrun` example) everything falls back to the platform folders of earlier
//! versions, the `legacy_*` ones. Thumbnails stay in the machine-local cache folder: they can
//! be gigabytes and are rebuilt on demand, so they don't belong in a synced folder.
use std::path::PathBuf;
use std::sync::OnceLock;

static APP_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Points data and config at `dir` (`<data root>/mosaico`). First call wins.
pub fn set_app_dir(dir: PathBuf) {
    let _ = APP_DIR.set(dir);
}

pub fn app_dir() -> Option<PathBuf> {
    APP_DIR.get().cloned()
}

const APP: &str = "mosaico";

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn home() -> Option<PathBuf> {
    env_dir("HOME").or_else(|| env_dir("USERPROFILE"))
}

/// `~/Library/Caches/mosaico`, `$XDG_CACHE_HOME/mosaico` (`~/.cache/mosaico`) or
/// `%LOCALAPPDATA%\mosaico\cache`.
pub fn cache_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return env_dir("LOCALAPPDATA").map(|d| d.join(APP).join("cache"));
    }
    if cfg!(target_os = "macos") {
        return home().map(|h| h.join("Library/Caches").join(APP));
    }
    env_dir("XDG_CACHE_HOME").or_else(|| home().map(|h| h.join(".cache"))).map(|d| d.join(APP))
}

/// Where undo journals and `people.json` live: [`app_dir`], else [`legacy_data_dir`].
pub fn data_dir() -> Option<PathBuf> {
    app_dir().or_else(legacy_data_dir)
}

/// Settings and recent folders: [`app_dir`], else [`legacy_config_dir`].
pub fn config_dir() -> Option<PathBuf> {
    app_dir().or_else(legacy_config_dir)
}

/// Before 0.3: `~/Library/Application Support/mosaico`, `$XDG_DATA_HOME/mosaico`
/// (`~/.local/share/mosaico`) or `%APPDATA%\mosaico`.
pub fn legacy_data_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return env_dir("APPDATA").map(|d| d.join(APP));
    }
    if cfg!(target_os = "macos") {
        return home().map(|h| h.join("Library/Application Support").join(APP));
    }
    env_dir("XDG_DATA_HOME").or_else(|| home().map(|h| h.join(".local/share"))).map(|d| d.join(APP))
}

/// Before 0.3: same as [`legacy_data_dir`] on macOS/Windows, XDG config on Linux.
pub fn legacy_config_dir() -> Option<PathBuf> {
    if cfg!(windows) || cfg!(target_os = "macos") {
        return legacy_data_dir();
    }
    env_dir("XDG_CONFIG_HOME").or_else(|| home().map(|h| h.join(".config"))).map(|d| d.join(APP))
}

pub fn journal_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("journals"))
}

pub fn thumb_dir() -> Option<PathBuf> {
    cache_dir().map(|d| d.join("thumbs"))
}
