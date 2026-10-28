//! Pictures for the pages: files TDLib has downloaded, and the tiny previews messages carry. A file
//! is decoded off the UI thread, by one worker in the order asked for, into pixels that become a
//! `slint::Image` on the UI thread (docs/architecture.md, Threads). A cache keeps the recent ones.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};

use base64::Engine;
use image::ImageDecoder as _;
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
    match read(path, LARGEST) {
        Ok(picture) => Some(pixels(picture)),
        Err(err) => {
            eprintln!("images: cannot decode {}: {err}", path.display());
            None
        }
    }
}

/// The picture in the file at `path`, turned the way it was taken (the EXIF orientation a phone's
/// photo carries), no larger than `largest` pixels on its longer side. What the image crate cannot
/// read (HEIC, TIFF, GIF, AVIF, …) the system reads (src/platform/).
pub fn read(path: &Path, largest: u32) -> Result<image::DynamicImage, String> {
    let picture = match read_oriented(path) {
        Ok(picture) => picture,
        Err(err) => crate::platform::read_picture(path, largest).map(image::DynamicImage::ImageRgba8).ok_or(err)?,
    };
    Ok(if picture.width().max(picture.height()) > largest { picture.thumbnail(largest, largest) } else { picture })
}

fn read_oriented(path: &Path) -> Result<image::DynamicImage, String> {
    let mut decoder = image::ImageReader::open(path)
        .map_err(|err| err.to_string())?
        .with_guessed_format()
        .map_err(|err| err.to_string())?
        .into_decoder()
        .map_err(|err| err.to_string())?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut picture = image::DynamicImage::from_decoder(decoder).map_err(|err| err.to_string())?;
    picture.apply_orientation(orientation);
    Ok(picture)
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

    /// A JPEG whose EXIF orientation is `orientation`, as a phone writes it (6: the phone was held
    /// upright, the picture is to be turned 90° clockwise to show).
    fn jpeg_turned(width: u32, height: u32, orientation: u16) -> Vec<u8> {
        use image::ImageEncoder as _;
        let picture = image::RgbImage::from_pixel(width, height, image::Rgb([74, 95, 193]));
        // A TIFF header (little end) and one entry: Orientation (0x0112), a SHORT, the value.
        let mut exif = vec![0x49, 0x49, 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0];
        exif.extend_from_slice(&orientation.to_le_bytes());
        exif.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut bytes);
        encoder.set_exif_metadata(exif).expect("EXIF");
        encoder.write_image(picture.as_raw(), width, height, image::ExtendedColorType::Rgb8).expect("a JPEG");
        bytes
    }

    /// An uncompressed RGB TIFF, a kind the image crate is built without here.
    fn tiff(width: u16, height: u16, colour: [u8; 3]) -> Vec<u8> {
        let pixels = usize::from(width) * usize::from(height);
        // The header, then the directory at 8: nine entries, then the bits per sample and the pixels.
        let entries: [(u16, u16, u32, u32); 9] = [
            (256, 3, 1, u32::from(width)),
            (257, 3, 1, u32::from(height)),
            (258, 3, 3, 122),
            (259, 3, 1, 1),
            (262, 3, 1, 2),
            (273, 4, 1, 128),
            (277, 3, 1, 3),
            (278, 3, 1, u32::from(height)),
            (279, 4, 1, (pixels * 3) as u32),
        ];
        let mut bytes = vec![0x49, 0x49, 42, 0, 8, 0, 0, 0];
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, kind, count, value) in entries {
            bytes.extend_from_slice(&tag.to_le_bytes());
            bytes.extend_from_slice(&kind.to_le_bytes());
            bytes.extend_from_slice(&count.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(&[8, 0, 8, 0, 8, 0]);
        assert_eq!(bytes.len(), 128);
        for _ in 0..pixels {
            bytes.extend_from_slice(&colour);
        }
        bytes
    }

    #[test]
    fn a_phone_photo_is_read_the_way_it_was_taken() {
        let directory = std::env::temp_dir().join(format!("finchgram-turned-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("a directory");
        let turned = directory.join("turned.jpg");
        std::fs::write(&turned, jpeg_turned(40, 20, 6)).expect("written");
        let picture = read(&turned, 4096).expect("read");
        assert_eq!((picture.width(), picture.height()), (20, 40));
        let upright = directory.join("upright.jpg");
        std::fs::write(&upright, jpeg_turned(40, 20, 1)).expect("written");
        let picture = read(&upright, 4096).expect("read");
        assert_eq!((picture.width(), picture.height()), (40, 20));
        // No larger than asked.
        assert_eq!(read(&turned, 10).map(|picture| (picture.width(), picture.height())), Ok((5, 10)));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_picture_the_image_crate_cannot_read_is_read_by_the_system() {
        let directory = std::env::temp_dir().join(format!("finchgram-tiff-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("a directory");
        let path = directory.join("picture.tiff");
        std::fs::write(&path, tiff(6, 4, [200, 40, 40])).expect("written");
        assert!(image::open(&path).is_err(), "the image crate reads TIFF now: this test no longer tests the system's reader");
        let picture = read(&path, 2048).expect("read by the system").to_rgba8();
        assert_eq!(picture.dimensions(), (6, 4));
        let [red, green, blue, alpha] = picture.get_pixel(2, 2).0;
        assert!(red > 150 && green < 90 && blue < 90 && alpha == 255, "{red} {green} {blue} {alpha}");
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
