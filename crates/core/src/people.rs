//! Person tags: which people appear in which photos or folders. A person is a card in the
//! shared Otherwise entity registry (otherwise-link), referenced by its id; mosaico only reads
//! the registry (in the app) and keeps the tags here, in its own data folder (`people.json`).
//! Nothing is written into the photos or into the registry. A folder's tag applies to
//! everything under it.
//!
//! Tags are keyed by absolute path, so photos moved by "Organize → move" lose theirs.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

pub const FILE: &str = "people.json";
const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tags {
    #[serde(default = "schema")]
    pub schema: u32,
    /// Path (file or folder) → entity ids, in the order they were added.
    #[serde(default)]
    pub tags: BTreeMap<PathBuf, Vec<String>>,
    /// Last known name per entity id, so tags still read well while the registry is off.
    #[serde(default)]
    pub names: BTreeMap<String, String>,
}

fn schema() -> u32 {
    SCHEMA
}

/// `<data dir>/people.json`
pub fn default_file() -> Option<PathBuf> {
    crate::dirs::data_dir().map(|d| d.join(FILE))
}

impl Tags {
    /// Missing file: no tags. A file that doesn't parse (or a newer schema) is an error, so it
    /// never gets overwritten by an empty set.
    pub fn load(file: &Path) -> io::Result<Tags> {
        let text = match std::fs::read_to_string(file) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Tags { schema: SCHEMA, ..Default::default() }),
            Err(e) => return Err(e),
        };
        let tags: Tags = serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if tags.schema > SCHEMA {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("people.json schema {}", tags.schema))); // i18n-ok: developer diagnostic
        }
        Ok(tags)
    }

    /// Written whole to a temp file next to it, then renamed over it.
    pub fn save(&self, file: &Path) -> io::Result<()> {
        if let Some(d) = file.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = file.with_extension("json.tmp");
        let mut out = self.clone();
        out.schema = SCHEMA;
        std::fs::write(&tmp, serde_json::to_vec_pretty(&out).map_err(io::Error::other)?)?;
        std::fs::rename(&tmp, file)
    }

    pub fn add(&mut self, path: &Path, id: &str, name: &str) -> bool {
        self.names.insert(id.to_string(), name.to_string());
        let ids = self.tags.entry(path.to_path_buf()).or_default();
        if ids.iter().any(|i| i == id) {
            return false;
        }
        ids.push(id.to_string());
        true
    }

    pub fn remove(&mut self, path: &Path, id: &str) -> bool {
        let Some(ids) = self.tags.get_mut(path) else { return false };
        let before = ids.len();
        ids.retain(|i| i != id);
        let changed = ids.len() != before;
        if ids.is_empty() {
            self.tags.remove(path);
        }
        changed
    }

    /// Tags set on exactly this path.
    pub fn direct(&self, path: &Path) -> &[String] {
        self.tags.get(path).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Everyone in `item`: its own tags, then its folders' (nearest first), each id once, with
    /// the path the tag sits on.
    pub fn of_item(&self, item: &Path) -> Vec<(String, PathBuf)> {
        let mut out: Vec<(String, PathBuf)> = Vec::new();
        for p in item.ancestors() {
            for id in self.direct(p) {
                if !out.iter().any(|(i, _)| i == id) {
                    out.push((id.clone(), p.to_path_buf()));
                }
            }
        }
        out
    }

    pub fn has(&self, item: &Path, id: &str) -> bool {
        item.ancestors().any(|p| self.direct(p).iter().any(|i| i == id))
    }

    pub fn name(&self, id: &str) -> Option<&str> {
        self.names.get(id).map(String::as_str)
    }

    /// People with at least one tag, by name.
    pub fn people(&self) -> Vec<(String, String)> {
        let mut ids: Vec<&String> = self.tags.values().flatten().collect();
        ids.sort();
        ids.dedup();
        let mut out: Vec<(String, String)> = ids.into_iter().map(|id| (id.clone(), self.name(id).unwrap_or(id).to_string())).collect();
        out.sort_by_key(|(_, n)| n.to_lowercase());
        out
    }

    /// The registry merged `from` into `to` (or renamed it): tags follow.
    pub fn repoint(&mut self, from: &str, to: &str, name: Option<&str>) -> bool {
        let mut changed = false;
        for ids in self.tags.values_mut().filter(|_| from != to) {
            if let Some(i) = ids.iter().position(|x| x == from) {
                changed = true;
                if ids.iter().any(|x| x == to) {
                    ids.remove(i);
                } else {
                    ids[i] = to.to_string();
                }
            }
        }
        if let Some(n) = name.or(self.names.get(from).map(String::as_str)).map(str::to_string)
            && self.names.get(to) != Some(&n)
        {
            self.names.insert(to.to_string(), n);
            changed = true;
        }
        if from != to && self.names.remove(from).is_some() {
            changed = true;
        }
        changed
    }
}
