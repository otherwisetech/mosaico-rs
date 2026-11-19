//! Thumbnail state for the grid: requests go to the core's worker pool, finished ones are
//! picked up by a short polling timer; decoded images live in a bounded (LRU) image cache.
use gpui::*;
use mosaico_core::scan::Item;
use mosaico_core::thumb::{self, Job, Thumbnailer};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum Thumb {
    Pending,
    Ready(Arc<Path>),
    /// No preview could be made (HEIC without an embedded JPEG, videos): show the format.
    Placeholder,
}

pub struct Thumbs {
    states: HashMap<usize, Thumb>,
    worker: Option<Thumbnailer>,
    cache: Option<PathBuf>,
    /// Bumped on every new library so late results from the previous one are dropped.
    generation: usize,
}

const GEN_SHIFT: u32 = 40;

impl Thumbs {
    pub fn new() -> Thumbs {
        let cache = mosaico_core::dirs::thumb_dir();
        let threads = std::thread::available_parallelism().map(|n| n.get().saturating_sub(1)).unwrap_or(2).clamp(1, 6);
        Thumbs { worker: cache.clone().map(|c| Thumbnailer::new(c, threads)), cache, states: HashMap::new(), generation: 0 }
    }

    /// Forget everything (a new library was loaded).
    pub fn reset(&mut self) {
        self.states.clear();
        self.generation += 1;
        if let Some(w) = &self.worker {
            w.clear();
        }
    }

    /// The thumbnail of item `ix`, requesting it the first time it is asked for.
    pub fn get(&mut self, ix: usize, it: &Item) -> Thumb {
        if let Some(s) = self.states.get(&ix) {
            return s.clone();
        }
        let state = match (&self.worker, &self.cache) {
            (Some(w), Some(cache)) => {
                let cached = thumb::cache_path(cache, &thumb::cache_key(&it.path, it.size, it.modified));
                if cached.is_file() {
                    Thumb::Ready(cached.into())
                } else if it.previews.is_empty() && !it.format.decodable() {
                    Thumb::Placeholder
                } else {
                    w.request(Job::for_item((self.generation << GEN_SHIFT) | ix, it));
                    Thumb::Pending
                }
            }
            _ => Thumb::Placeholder,
        };
        self.states.insert(ix, state.clone());
        state
    }

    /// Collect finished thumbnails; true if any arrived.
    pub fn poll(&mut self) -> bool {
        let Some(w) = &self.worker else { return false };
        let mut changed = false;
        for (id, res) in w.results() {
            if id >> GEN_SHIFT != self.generation {
                continue;
            }
            let ix = id & ((1 << GEN_SHIFT) - 1);
            self.states.insert(ix, res.map(|p| Thumb::Ready(p.into())).unwrap_or(Thumb::Placeholder));
            changed = true;
        }
        changed
    }
}

/// Image cache that keeps at most `cap` decoded thumbnails, dropping the least recently drawn.
pub struct LruImages {
    inner: Entity<RetainAllImageCache>,
    used: HashMap<Resource, u64>,
    tick: u64,
    cap: usize,
}

impl LruImages {
    pub fn new(cap: usize, cx: &mut App) -> Entity<LruImages> {
        let inner = RetainAllImageCache::new(cx);
        cx.new(|_| LruImages { inner, used: HashMap::new(), tick: 0, cap })
    }
}

impl ImageCache for LruImages {
    fn load(&mut self, resource: &Resource, window: &mut Window, cx: &mut App) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        self.tick += 1;
        self.used.insert(resource.clone(), self.tick);
        if self.used.len() > self.cap {
            let mut by_age: Vec<(u64, Resource)> = self.used.iter().map(|(r, t)| (*t, r.clone())).collect();
            by_age.sort_by_key(|(t, _)| *t);
            let drop = self.used.len() - self.cap * 9 / 10;
            for (_, r) in by_age.into_iter().take(drop) {
                self.used.remove(&r);
                self.inner.update(cx, |c, cx| c.remove(&r, window, cx));
            }
        }
        self.inner.update(cx, |c, cx| c.load(resource, window, cx))
    }
}
