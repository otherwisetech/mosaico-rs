//! Platform folders: thumbnail cache, app data (journals) and config.
use std::path::PathBuf;

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

/// Where undo journals live: `~/Library/Application Support/mosaico`,
/// `$XDG_DATA_HOME/mosaico` (`~/.local/share/mosaico`) or `%APPDATA%\mosaico`.
pub fn data_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return env_dir("APPDATA").map(|d| d.join(APP));
    }
    if cfg!(target_os = "macos") {
        return home().map(|h| h.join("Library/Application Support").join(APP));
    }
    env_dir("XDG_DATA_HOME").or_else(|| home().map(|h| h.join(".local/share"))).map(|d| d.join(APP))
}

/// Settings and recent folders: same as [`data_dir`] on macOS/Windows, XDG config on Linux.
pub fn config_dir() -> Option<PathBuf> {
    if cfg!(windows) || cfg!(target_os = "macos") {
        return data_dir();
    }
    env_dir("XDG_CONFIG_HOME").or_else(|| home().map(|h| h.join(".config"))).map(|d| d.join(APP))
}

pub fn journal_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("journals"))
}

pub fn thumb_dir() -> Option<PathBuf> {
    cache_dir().map(|d| d.join("thumbs"))
}
