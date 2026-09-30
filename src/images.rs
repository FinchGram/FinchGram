//! Pictures for the pages: files TDLib has downloaded, and the tiny previews messages carry. A file
//! is decoded off the UI thread, by one worker in the order asked for, into pixels that become a
//! `slint::Image` on the UI thread (docs/architecture.md, Threads). A cache keeps the recent ones.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};

use base64::Engine;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// A picture larger than this on its longer side is scaled down: the pages show it far smaller.
const LARGEST: u32 = 2048;

type Ready = Box<dyn FnOnce(Option<Image>)>;

struct Job {
    id: u64,
    path: PathBuf,
}

thread_local! {
    static WORKER: RefCell<Option<Sender<Job>>> = const { RefCell::new(None) };
    /// By job: what waits for the picture.
    static WAITING: RefCell<HashMap<u64, Ready>> = RefCell::new(HashMap::new());
    static NEXT: Cell<u64> = const { Cell::new(1) };
}

/// Decode the picture in the file at `path` off the UI thread. `ready` gets it on the UI thread,
/// or None when the file is not a picture that can be read.
pub fn load(path: impl Into<PathBuf>, ready: impl FnOnce(Option<Image>) + 'static) {
    let id = NEXT.with(|next| next.replace(next.get() + 1));
    WAITING.with(|waiting| waiting.borrow_mut().insert(id, Box::new(ready)));
    let job = Job { id, path: path.into() };
    WORKER.with(|worker| {
        let mut worker = worker.borrow_mut();
        if let Err(mpsc::SendError(job)) = worker.get_or_insert_with(start).send(job) {
            // The worker is gone (it panicked on a file): a new one takes the job.
            *worker = Some(start());
            if let Some(sender) = worker.as_ref() {
                let _ = sender.send(job);
            }
        }
    });
}

/// A message's tiny preview: a base64 JPEG of about 40 pixels, small enough to decode at once.
pub fn preview(base64_jpeg: &str) -> Option<Image> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(base64_jpeg).ok()?;
    let picture = image::load_from_memory(&bytes).ok()?;
    Some(Image::from_rgba8(pixels(picture)))
}

fn start() -> Sender<Job> {
    let (sender, jobs) = mpsc::channel::<Job>();
    let started = std::thread::Builder::new().name("images".into()).spawn(move || {
        for job in jobs {
            let pixels = decode(&job.path);
            let id = job.id;
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ready) = WAITING.with(|waiting| waiting.borrow_mut().remove(&id)) {
                    ready(pixels.map(Image::from_rgba8));
                }
            });
        }
    });
    if let Err(err) = started {
        eprintln!("images: cannot start the decoding thread: {err}");
    }
    sender
}

fn decode(path: &Path) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let picture = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode();
    match picture {
        Ok(picture) => Some(pixels(picture)),
        Err(err) => {
            eprintln!("images: cannot decode {}: {err}", path.display());
            None
        }
    }
}

fn pixels(picture: image::DynamicImage) -> SharedPixelBuffer<Rgba8Pixel> {
    let picture = if picture.width().max(picture.height()) > LARGEST { picture.thumbnail(LARGEST, LARGEST) } else { picture };
    let rgba = picture.into_rgba8();
    SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height())
}

/// The pictures shown lately, by key; the oldest go first when there are too many.
pub struct Cache<K> {
    pictures: HashMap<K, Image>,
    order: VecDeque<K>,
    capacity: usize,
}

impl<K: Clone + Eq + Hash> Cache<K> {
    pub fn new(capacity: usize) -> Cache<K> {
        Cache { pictures: HashMap::new(), order: VecDeque::new(), capacity }
    }

    pub fn get(&self, key: &K) -> Option<Image> {
        self.pictures.get(key).cloned()
    }

    pub fn contains(&self, key: &K) -> bool {
        self.pictures.contains_key(key)
    }

    pub fn insert(&mut self, key: K, picture: Image) {
        if self.pictures.insert(key.clone(), picture).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.pictures.remove(&oldest);
            }
        }
    }

    pub fn clear(&mut self) {
        self.pictures.clear();
        self.order.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let picture = image::RgbImage::from_pixel(width, height, image::Rgb([74, 95, 193]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        picture.write_to(&mut bytes, image::ImageFormat::Jpeg).expect("a JPEG");
        bytes.into_inner()
    }

    #[test]
    fn a_preview_is_decoded_from_base64() {
        let data = base64::engine::general_purpose::STANDARD.encode(jpeg(40, 30));
        let picture = preview(&data).expect("a picture");
        assert_eq!((picture.size().width, picture.size().height), (40, 30));
        assert!(preview("not base64!").is_none());
    }

    #[test]
    fn a_large_file_is_scaled_down_and_a_broken_one_is_none() {
        let directory = std::env::temp_dir().join(format!("finchgram-images-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("a directory");
        let large = directory.join("large.jpg");
        std::fs::write(&large, jpeg(4096, 1024)).expect("written");
        let pixels = decode(&large).expect("decoded");
        assert_eq!((pixels.width(), pixels.height()), (2048, 512));
        let broken = directory.join("broken.jpg");
        std::fs::write(&broken, b"not a picture").expect("written");
        assert!(decode(&broken).is_none());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_cache_drops_the_oldest() {
        let picture = Image::from_rgba8(SharedPixelBuffer::new(1, 1));
        let mut cache = Cache::new(2);
        cache.insert(1, picture.clone());
        cache.insert(2, picture.clone());
        cache.insert(3, picture);
        assert!(!cache.contains(&1) && cache.contains(&2) && cache.contains(&3));
    }
}
