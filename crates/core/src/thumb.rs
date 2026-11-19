//! Thumbnails: the embedded JPEG preview when there is one (instant), else a full decode with
//! the `image` crate. Cached on disk as small JPEGs keyed by path + size + modified time.
//! Formats with neither (HEIC without an EXIF preview, videos) get a placeholder tile.
use crate::media::Format;
use crate::scan::Item;
use image::{DynamicImage, ImageFormat};
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Longest side of a cached thumbnail.
pub const THUMB_PX: u32 = 256;
/// Embedded previews smaller than this are a last resort (160×120 EXIF thumbnails are blurry).
const GOOD_PREVIEW: u64 = 20_000;
const MAX_PREVIEW: u64 = 32 << 20;

#[derive(Debug, Clone)]
pub struct Job {
    /// Caller's id, returned with the result.
    pub id: usize,
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub format: Format,
    pub orientation: u16,
    pub previews: Vec<(u64, u64)>,
}

impl Job {
    pub fn for_item(id: usize, it: &Item) -> Job {
        Job { id, path: it.path.clone(), size: it.size, modified: it.modified, format: it.format, orientation: it.orientation, previews: it.previews.clone() }
    }
}

/// Cache file name for a source file (changes whenever the file does).
pub fn cache_key(path: &Path, size: u64, modified: Option<SystemTime>) -> String {
    let nanos = modified.and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
    let mut h = blake3::Hasher::new();
    h.update(path.as_os_str().as_encoded_bytes());
    h.update(&size.to_le_bytes());
    h.update(&nanos.to_le_bytes());
    h.finalize().to_hex()[..32].to_string()
}

pub fn cache_path(cache: &Path, key: &str) -> PathBuf {
    cache.join(&key[..2]).join(format!("{key}.jpg"))
}

/// The cached thumbnail for `job`, creating it if needed. `Ok(None)` = show a placeholder.
pub fn make(job: &Job, cache: &Path) -> io::Result<Option<PathBuf>> {
    let out = cache_path(cache, &cache_key(&job.path, job.size, job.modified));
    if out.exists() {
        return Ok(Some(out));
    }
    let Some(img) = render(job) else { return Ok(None) };
    let dir = out.parent().unwrap();
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}-{}.tmp", std::process::id(), job.id));
    {
        let mut w = BufWriter::new(File::create(&tmp)?);
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, 82).encode_image(&img.to_rgb8()).map_err(io::Error::other)?;
    }
    fs::rename(&tmp, &out)?;
    Ok(Some(out))
}

/// Decode the best available picture and shrink it to THUMB_PX, upright.
pub fn render(job: &Job) -> Option<DynamicImage> {
    let mut previews = job.previews.clone();
    previews.retain(|(_, l)| *l <= MAX_PREVIEW);
    previews.sort_by_key(|(_, l)| *l);
    // smallest good-sized preview first, then the larger ones, then the tiny ones
    let split = previews.iter().position(|(_, l)| *l >= GOOD_PREVIEW).unwrap_or(previews.len());
    let order: Vec<(u64, u64)> = previews[split..].iter().chain(previews[..split].iter().rev()).copied().collect();
    let mut img = order.iter().find_map(|&(o, l)| decode_preview(&job.path, o, l)).or_else(|| {
        if job.format.decodable() { image::ImageReader::open(&job.path).ok()?.with_guessed_format().ok()?.decode().ok() } else { None }
    })?;
    if img.width() > THUMB_PX || img.height() > THUMB_PX {
        img = img.thumbnail(THUMB_PX, THUMB_PX);
    }
    Some(orient(img, job.orientation))
}

fn decode_preview(path: &Path, off: u64, len: u64) -> Option<DynamicImage> {
    let mut f = File::open(path).ok()?;
    f.seek(SeekFrom::Start(off)).ok()?;
    let mut buf = vec![0; len as usize];
    f.read_exact(&mut buf).ok()?;
    image::load_from_memory_with_format(&buf, ImageFormat::Jpeg).ok()
}

/// Apply an EXIF orientation (1–8).
pub fn orient(img: DynamicImage, o: u16) -> DynamicImage {
    match o {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

type Queue = Arc<(Mutex<Vec<Job>>, Condvar)>;

/// A pool of worker threads producing thumbnails. Requests are served newest first, so the
/// tiles currently on screen win over ones the user already scrolled past.
pub struct Thumbnailer {
    queue: Queue,
    stop: Arc<AtomicBool>,
    rx: Receiver<(usize, Option<PathBuf>)>,
}

impl Thumbnailer {
    pub fn new(cache: PathBuf, threads: usize) -> Thumbnailer {
        let queue: Queue = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        for _ in 0..threads.max(1) {
            let (queue, stop, tx, cache): (Queue, _, Sender<_>, _) = (queue.clone(), stop.clone(), tx.clone(), cache.clone());
            std::thread::spawn(move || {
                loop {
                    let job = {
                        let (lock, cv) = &*queue;
                        let mut q = lock.lock().unwrap();
                        while q.is_empty() && !stop.load(Relaxed) {
                            q = cv.wait(q).unwrap();
                        }
                        if stop.load(Relaxed) {
                            return;
                        }
                        q.pop().unwrap()
                    };
                    let res = make(&job, &cache).ok().flatten();
                    if tx.send((job.id, res)).is_err() {
                        return;
                    }
                }
            });
        }
        Thumbnailer { queue, stop, rx }
    }

    pub fn request(&self, job: Job) {
        let (lock, cv) = &*self.queue;
        lock.lock().unwrap().push(job);
        cv.notify_one();
    }

    /// Forget pending requests (e.g. after a new scan).
    pub fn clear(&self) {
        self.queue.0.lock().unwrap().clear();
    }

    pub fn results(&self) -> Vec<(usize, Option<PathBuf>)> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Thumbnailer {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        self.queue.1.notify_all();
    }
}
