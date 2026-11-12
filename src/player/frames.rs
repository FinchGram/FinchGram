//! Does a video's frame come out right in the window? The app's own player and the app's own
//! window, drawn by Slint's Skia renderer over Metal as on the screen, but into a texture, which is
//! read back: pixel values, not eyes (0.2.0 showed videos upside down, and a picture looked at did
//! not tell). docs/skia.md has what was measured before the app was changed.
//!
//! ```sh
//! cargo test video_frames -- --ignored --nocapture
//! ```
//!
//! It needs a GPU (Metal, and OpenGL for mpv) but no display, and takes over Slint's platform for
//! the whole test process, so it only runs when asked for.
//!
//! The videos (fixtures/frames-ffv1.mkv, lossless, decoded by FFmpeg; fixtures/frames-h264.mp4,
//! 1280×720, decoded by VideoToolbox) are twelve frames, ten a second: red top left, green top
//! right, blue bottom left, white bottom right, and in the middle of the red a grey square that
//! says which frame it is (10 + 20 × its number). A flipped picture or swapped colours put the
//! wrong colour in a corner; a frame that never shows leaves the grey behind. They were made from
//! raw frames written by a few lines of Python, with ffmpeg:
//!
//! ```sh
//! ffmpeg -f rawvideo -pix_fmt rgb24 -s 320x180 -r 10 -i frames.rgb -c:v ffv1 -pix_fmt bgr0 frames-ffv1.mkv
//! ffmpeg -f rawvideo -pix_fmt rgb24 -s 320x180 -r 10 -i frames.rgb \
//!   -vf "scale=1280:720:flags=neighbor:out_color_matrix=bt709:out_range=tv,format=yuv420p" \
//!   -c:v libx264 -crf 10 -g 12 -colorspace bt709 -color_primaries bt709 -color_trc bt709 -color_range tv frames-h264.mp4
//! ```

use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use slint::platform::{EventLoopProxy, Platform, WindowAdapter};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::offscreen;
use crate::{AppState, MainWindow, Page, Viewer, ViewerItem};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
/// Frames in each video.
const FRAMES: usize = 12;

type Asked = Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>;

/// Slint's platform with one window that is never on a screen (src/offscreen.rs), and an event
/// loop that is the test's own: what mpv's threads ask of the UI thread waits in `asked` for the
/// next turn.
struct Offscreen {
    window: Rc<offscreen::Window>,
    asked: Asked,
    started: Instant,
}

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy { asked: self.asked.clone() }))
    }

    fn duration_since_start(&self) -> Duration {
        self.started.elapsed()
    }
}

struct Proxy {
    asked: Asked,
}

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
        Ok(())
    }

    fn invoke_from_event_loop(&self, event: Box<dyn FnOnce() + Send>) -> Result<(), slint::EventLoopError> {
        self.asked.lock().unwrap().push(event);
        Ok(())
    }
}

/// The window and the test's event loop.
struct Stage {
    window: Rc<offscreen::Window>,
    asked: Asked,
}

impl Stage {
    /// One turn of the event loop: what was asked of the UI thread runs, the timers fire, the
    /// window renders (the player draws its frame first, as on the screen), and here is the
    /// picture: red, green, blue, alpha.
    fn turn(&self) -> Vec<u8> {
        let asked: Vec<_> = std::mem::take(&mut *self.asked.lock().unwrap());
        for event in asked {
            event();
        }
        slint::platform::update_timers_and_animations();
        self.window.draw()
    }
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let offset = ((y * WIDTH + x) * 4) as usize;
    [pixels[offset], pixels[offset + 1], pixels[offset + 2]]
}

fn near(pixel: [u8; 3], wanted: [u8; 3], tolerance: u8) -> bool {
    (0..3).all(|channel| pixel[channel].abs_diff(wanted[channel]) <= tolerance)
}

/// Where the video is in the window (left, top, right, bottom): what its red, green and blue
/// reach, which the pages around it have none of. None while no frame shows.
fn find_video(pixels: &[u8], tolerance: u8) -> Option<(u32, u32, u32, u32)> {
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let here = pixel(pixels, x, y);
            if [[255, 0, 0], [0, 255, 0], [0, 0, 255]].iter().any(|colour| near(here, *colour, tolerance)) {
                found = Some(match found {
                    None => (x, y, x, y),
                    Some((left, top, right, bottom)) => (left.min(x), top.min(y), right.max(x), bottom.max(y)),
                });
            }
        }
    }
    found.filter(|(left, top, right, bottom)| right - left > 100 && bottom - top > 50)
}

/// What the window shows of the video: which frame its grey square stands for, or what is wrong.
fn frame_shown(pixels: &[u8], tolerance: u8) -> Option<Result<usize, String>> {
    let (left, top, right, bottom) = find_video(pixels, tolerance)?;
    let at = |x: f32, y: f32| pixel(pixels, left + ((right - left) as f32 * x) as u32, top + ((bottom - top) as f32 * y) as u32);
    // Beside the middle, clear of the corners' rounding, of the grey square and of what the viewer
    // draws over a video that does not play.
    let corners = [at(0.12, 0.40), at(0.88, 0.40), at(0.12, 0.60), at(0.88, 0.60)];
    let wanted = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]];
    if !(0..4).all(|corner| near(corners[corner], wanted[corner], tolerance)) {
        return Some(Err(format!("red, green, blue, white came out as {corners:?} in {left},{top} … {right},{bottom}")));
    }
    let [red, green, blue] = at(0.25, 0.25).map(i32::from);
    let index = ((green - 10) as f32 / 20.0).round() as i32;
    let grey = (red - green).abs() <= 8 && (green - blue).abs() <= 8 && (green - (10 + 20 * index)).abs() <= 8;
    if !grey || !(0..FRAMES as i32).contains(&index) {
        return Some(Err(format!("the grey square came out as {:?}", [red, green, blue])));
    }
    Some(Ok(index as usize))
}

#[test]
#[ignore = "needs a GPU and takes over Slint's platform; run with: cargo test video_frames -- --ignored --nocapture"]
fn video_frames() {
    let stage = Stage { window: offscreen::Window::new(&offscreen::Graphics::new()), asked: Asked::default() };
    slint::platform::set_platform(Box::new(Offscreen { window: stage.window.clone(), asked: stage.asked.clone(), started: Instant::now() }))
        .expect("platform");
    crate::fonts::register();
    stage.window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    let ui = MainWindow::new().expect("window");
    ui.show().expect("show");
    super::install(&ui);
    // The media viewer opens over the Downloads page too, which needs no account.
    ui.global::<AppState>().set_page(Page::Downloads);
    let viewer = ui.global::<Viewer>();
    let folder = concat!(env!("CARGO_MANIFEST_DIR"), "/src/player/fixtures");

    for (name, width, height, tolerance, decoder) in [("frames-ffv1.mkv", 320, 180, 2, "no"), ("frames-h264.mp4", 1280, 720, 16, "videotoolbox")] {
        let item = ViewerItem { id: name.into(), width, height, video: true, duration: 1, name: name.into(), ..ViewerItem::default() };
        viewer.set_items(ModelRc::new(VecModel::from(vec![item])));
        viewer.set_index(0);
        viewer.set_open(true);
        // The viewer is drawn before the video starts, as in the app: what Skia does once for a
        // page (its shaders, the glyphs) is not time taken from the video's first frames.
        let warmed = Instant::now();
        for _ in 0..3 {
            stage.turn();
        }
        println!("{name}: the viewer's first three pictures took {:?}", warmed.elapsed());
        super::play(&format!("{folder}/{name}"), false, 1.0, 0, true);

        // Every picture on the way is looked at: the frames come in their order, up to the last.
        let mut seen: Vec<usize> = Vec::new();
        let mut longest = Duration::ZERO;
        let until = Instant::now() + Duration::from_secs(10);
        while seen.last() != Some(&(FRAMES - 1)) && Instant::now() < until {
            let turned = Instant::now();
            let pixels = stage.turn();
            longest = longest.max(turned.elapsed());
            match frame_shown(&pixels, tolerance) {
                Some(Ok(index)) if seen.last() != Some(&index) => seen.push(index),
                Some(Err(wrong)) => panic!("{name}, after frames {seen:?}: {wrong}"),
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(4));
        }
        println!("{name}: the window showed frames {seen:?}; its slowest picture took {longest:?}");
        assert_eq!(seen.last(), Some(&(FRAMES - 1)), "{name}: the last frame never showed");
        assert!(seen.windows(2).all(|pair| pair[0] < pair[1]), "{name}: frames out of their order: {seen:?}");
        assert!(seen.len() >= FRAMES - 3, "{name}: too few of the frames showed: {seen:?}");
        let mut decoded = String::new();
        super::with_player(|player| decoded = player.property_text("hwdec-current").unwrap_or_default());
        assert_eq!(decoded, decoder, "{name}: decoded another way");

        // The window's renderer goes (the window was hidden) and comes back: the frame there was
        // shows again, in a new surface, though mpv has no new one.
        super::with_player(super::Player::release_surface);
        assert_eq!(viewer.get_frame().size().width, 0, "{name}: the page kept the frame of a surface that went");
        assert_eq!(frame_shown(&stage.turn(), tolerance), Some(Ok(FRAMES - 1)), "{name}: the frame did not come back in a new surface");

        super::stop();
        viewer.set_open(false);
        assert!(frame_shown(&stage.turn(), tolerance).is_none(), "{name}: the video still shows after it stopped");
    }
    super::shut_down();
}
