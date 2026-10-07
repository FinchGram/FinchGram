//! What a video file holds before it is sent (telegram/attachments.rs): how long it is, how big
//! its picture is, and a still of it for its message. A libmpv of its own finds out on another
//! thread, decoding one frame, which it writes out as a JPEG (vo=image) at most 320 pixels wide,
//! the size Telegram wants of a thumbnail.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::{Duration, Instant};

use super::mpv;

/// How long a file may take to load and give its first frame.
const PATIENCE: Duration = Duration::from_secs(20);

pub struct VideoInfo {
    pub duration: i32,
    pub width: i32,
    pub height: i32,
    /// A JPEG of one frame, at most 320 pixels wide; none when mpv could not write one.
    pub thumbnail: Option<PathBuf>,
}

/// Look into the video at `path`, writing its still into `out_dir` under `name`.jpg; `ready` gets
/// what was found on the UI thread, or None when it is not a video mpv can read.
type Ready = Box<dyn FnOnce(Option<VideoInfo>)>;

pub fn probe(path: PathBuf, out_dir: PathBuf, name: String, ready: impl FnOnce(Option<VideoInfo>) + 'static) {
    let ready = std::cell::RefCell::new(Some(Box::new(ready) as Ready));
    let key = KEYS.with(|keys| {
        let mut keys = keys.borrow_mut();
        let key = keys.0;
        keys.0 += 1;
        keys.1.insert(key, ready.into_inner().expect("the callback"));
        key
    });
    let spawned = std::thread::Builder::new().name("video-probe".into()).spawn(move || {
        let info = look(&path, &out_dir, &name);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ready) = KEYS.with(|keys| keys.borrow_mut().1.remove(&key)) {
                ready(info);
            }
        });
    });
    if let Err(err) = spawned {
        eprintln!("video probe: cannot start a thread: {err}");
        if let Some(ready) = KEYS.with(|keys| keys.borrow_mut().1.remove(&key)) {
            ready(None);
        }
    }
}

thread_local! {
    /// What waits for each probe, by its number.
    static KEYS: std::cell::RefCell<(u64, std::collections::HashMap<u64, Ready>)> =
        std::cell::RefCell::new((1, std::collections::HashMap::new()));
}

fn look(path: &Path, out_dir: &Path, name: &str) -> Option<VideoInfo> {
    if std::fs::create_dir_all(out_dir).is_err() {
        return None;
    }
    // SAFETY: a libmpv handle of our own, used on this thread only and destroyed before leaving.
    unsafe {
        let handle = mpv::mpv_create();
        if handle.is_null() {
            return None;
        }
        let options = [
            ("vo", "image"),
            ("vo-image-format", "jpg"),
            ("vo-image-outdir", &out_dir.to_string_lossy()),
            ("vf", "scale=320:-2"),
            ("frames", "1"),
            ("ao", "null"),
            ("hwdec", "no"),
            ("audio", "no"),
            ("terminal", "no"),
            ("msg-level", "all=no"),
        ];
        for (name, value) in options {
            let (name, value) = (CString::new(name).ok()?, CString::new(value).ok()?);
            if mpv::mpv_set_option_string(handle, name.as_ptr(), value.as_ptr()) < 0 {
                eprintln!("video probe: mpv refused the option {}", name.to_string_lossy());
            }
        }
        if mpv::mpv_initialize(handle) < 0 {
            mpv::mpv_terminate_destroy(handle);
            return None;
        }
        let loadfile = CString::new("loadfile").ok()?;
        let file = CString::new(path.to_string_lossy().as_bytes()).ok()?;
        let mut args: [*const c_char; 3] = [loadfile.as_ptr(), file.as_ptr(), ptr::null()];
        if mpv::mpv_command(handle, args.as_mut_ptr()) < 0 {
            mpv::mpv_terminate_destroy(handle);
            return None;
        }
        let started = Instant::now();
        let mut info = None;
        loop {
            if started.elapsed() > PATIENCE {
                break;
            }
            let event = &*mpv::mpv_wait_event(handle, 0.25);
            match event.event_id {
                mpv::MPV_EVENT_NONE => continue,
                mpv::MPV_EVENT_FILE_LOADED => {
                    let duration = double(handle, "duration").unwrap_or(0.0);
                    let rotate = int(handle, "video-params/rotate").unwrap_or(0);
                    let (mut width, mut height) = (int(handle, "width").unwrap_or(0), int(handle, "height").unwrap_or(0));
                    if rotate == 90 || rotate == 270 {
                        std::mem::swap(&mut width, &mut height);
                    }
                    info = Some(VideoInfo {
                        duration: duration.round() as i32,
                        width: width as i32,
                        height: height as i32,
                        thumbnail: None,
                    });
                }
                mpv::MPV_EVENT_END_FILE | mpv::MPV_EVENT_SHUTDOWN => break,
                _ => {}
            }
        }
        mpv::mpv_terminate_destroy(handle);
        let mut info = info?;
        // vo=image names the frame by its number.
        let frame = out_dir.join("00000001.jpg");
        let still = out_dir.join(format!("{name}.jpg"));
        if frame.is_file() && std::fs::rename(&frame, &still).is_ok() {
            info.thumbnail = Some(still);
        }
        Some(info)
    }
}

unsafe fn double(handle: *mut mpv::mpv_handle, name: &str) -> Option<f64> {
    let name = CString::new(name).ok()?;
    let mut value: f64 = 0.0;
    // SAFETY: the format asked for is a double, which `value` holds.
    let result = unsafe { mpv::mpv_get_property(handle, name.as_ptr(), mpv::MPV_FORMAT_DOUBLE, &mut value as *mut f64 as *mut c_void) };
    (result >= 0).then_some(value)
}

unsafe fn int(handle: *mut mpv::mpv_handle, name: &str) -> Option<i64> {
    let name = CString::new(name).ok()?;
    let mut value: i64 = 0;
    // SAFETY: the format asked for is an int64, which `value` holds.
    let result = unsafe { mpv::mpv_get_property(handle, name.as_ptr(), mpv::MPV_FORMAT_INT64, &mut value as *mut i64 as *mut c_void) };
    (result >= 0).then_some(value)
}

/// mpv's words for an error code.
#[allow(dead_code)]
fn error_text(code: c_int) -> String {
    // SAFETY: mpv_error_string returns a static string.
    unsafe { CStr::from_ptr(mpv::mpv_error_string(code)).to_string_lossy().into_owned() }
}
