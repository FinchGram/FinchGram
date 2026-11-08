//! Video through libmpv (vendor/mpv; docs/architecture.md, "Media playback"): the media viewer's
//! player, and the live stream's (`live`, docs/live-streams.md), each an mpv of its own. Voice
//! messages play through an mpv of their own, with no video (`audio`). mpv decodes, with
//! VideoToolbox where it can, and each time the window renders it draws the current frame into an
//! OpenGL texture of ours, inside Slint's own context (FemtoVG); the page shows that texture as an
//! image (Viewer.frame, Live.frame). What mpv reports goes to that page's global: the position,
//! playing or not, the end.
//!
//! Everything runs on the UI thread; mpv's own threads only ask the event loop to look, and read
//! a file played as it arrives (`stream`) or a live stream as it comes (`pipe`).

pub mod audio;
pub mod decode;
pub mod live;
pub mod popout;
mod mpv;
pub mod music;
pub mod pipe;
pub mod probe;
pub mod remux;
pub mod stream;

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::num::NonZeroU32;
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use glow::HasContext;
use slint::{BorrowedOpenGLTextureBuilder, BorrowedOpenGLTextureOrigin, ComponentHandle, GraphicsAPI, Image, RenderingState};

use crate::{Live, MainWindow, Viewer};

/// A video larger than this on its longer side is drawn this large: the viewer shows it smaller.
const LARGEST: i64 = 2560;

/// What FFmpeg may open for a player of files on the Mac (Telegram's, the music cache's): nothing
/// over the network, not even a playlist inside a file pointing at one. mpv's own streams of the
/// app ("finchgram://") are no FFmpeg protocol and stay open.
pub(crate) const LOCAL_PROTOCOLS: [(&str, &str); 2] =
    [("stream-lavf-o", "protocol_whitelist=[file,crypto,data]"), ("demuxer-lavf-o", "protocol_whitelist=[file,crypto,data]")];

/// What FFmpeg may open for the Live view's streams (src/live/watch.rs): HTTP(S) for HLS, DASH and
/// Icecast, RTSP over its transports. The certificates are checked against the Keychain's roots.
const NETWORK_PROTOCOLS: [(&str, &str); 2] = [
    ("stream-lavf-o", "protocol_whitelist=[https,http,tls,tcp,crypto,data,rtsp,rtp,udp]"),
    ("demuxer-lavf-o", "protocol_whitelist=[https,http,tls,tcp,crypto,data,rtsp,rtp,udp]"),
];

/// What mpv reports, by the number given when observing it.
const POSITION: u64 = 1;
const PAUSED: u64 = 2;
const ENDED: u64 = 3;
const WIDTH: u64 = 4;
const HEIGHT: u64 = 5;
/// mpv waits for bytes of a file played as it arrives (paused-for-cache): the viewer's spinner.
const BUFFERING: u64 = 6;

/// Which page a player draws into: the media viewer, or the live stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Viewer,
    Live,
    /// The Live view's stream in a window of its own, on top of every app (popout.rs).
    Popout,
}

impl Screen {
    /// The screens of the main window.
    const ALL: [Screen; 2] = [Screen::Viewer, Screen::Live];

    fn index(self) -> usize {
        match self {
            Screen::Viewer => 0,
            Screen::Live => 1,
            Screen::Popout => 2,
        }
    }

    /// The screen a wake-up from mpv is for: the number given with the callback.
    fn from_data(data: *mut c_void) -> Screen {
        match data as usize {
            1 => Screen::Live,
            2 => Screen::Popout,
            _ => Screen::Viewer,
        }
    }

    /// A screen that plays the network's streams (the Live view's), not files on the Mac.
    fn networked(self) -> bool {
        matches!(self, Screen::Live | Screen::Popout)
    }
}

/// What a player tells its page.
enum Shown {
    Frame(Image),
    Position(f32),
    Loading(bool),
    Playing(bool),
    Ended(bool),
}

struct Player {
    screen: Screen,
    ui: slint::Weak<MainWindow>,
    /// The pop-out's window, for Screen::Popout.
    popout: Option<slint::Weak<crate::LivePopOut>>,
    /// Created when the first video plays.
    mpv: *mut mpv::mpv_handle,
    /// Created in the window's OpenGL context when the first frame is drawn.
    render: *mut mpv::mpv_render_context,
    gl: Option<Rc<glow::Context>>,
    target: Option<Target>,
    /// The viewer has the texture (Viewer.frame). stop() takes it from the viewer and keeps the
    /// texture: the next video of the same size draws into it, and the viewer has to be given it
    /// again. (Up to v0.3.14 it was given only with a new texture: a video the size of the one
    /// before stayed on its still while it played.)
    handed: bool,
    /// The video's size as shown (mpv's dwidth, dheight).
    width: i64,
    height: i64,
    loaded: bool,
    paused: bool,
    ended: bool,
    /// A file to play once the renderer exists: mpv's video output needs it before a file loads.
    waiting: Option<Load>,
}

struct Load {
    path: String,
    looping: bool,
    speed: f64,
    /// 0 … 100.
    volume: i32,
    muted: bool,
}

/// The texture mpv draws into, and the framebuffer that draws into it.
struct Target {
    texture: glow::Texture,
    framebuffer: glow::Framebuffer,
    width: i32,
    height: i32,
}

thread_local! {
    /// The players, by their screen's index.
    static PLAYERS: RefCell<[Option<Player>; 3]> = const { RefCell::new([None, None, None]) };
}

/// Set while the UI thread has been asked to look and has not yet: mpv wakes it once (by screen).
static EVENTS_ASKED: [AtomicBool; 3] = [AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false)];
static FRAME_ASKED: [AtomicBool; 3] = [AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false)];

/// Hook the players into `ui`'s rendering. Call once.
pub fn install(ui: &MainWindow) {
    PLAYERS.with(|players| {
        let mut players = players.borrow_mut();
        for screen in Screen::ALL {
            players[screen.index()] = Some(Player {
                screen,
                ui: ui.as_weak(),
                popout: None,
                mpv: ptr::null_mut(),
                render: ptr::null_mut(),
                gl: None,
                target: None,
                handed: false,
                width: 0,
                height: 0,
                loaded: false,
                paused: false,
                ended: false,
                waiting: None,
            });
        }
    });
    let hooked = ui.window().set_rendering_notifier(|state, api| {
        let GraphicsAPI::NativeOpenGL { get_proc_address } = api else { return };
        // SAFETY: the function pointers come from the context the window renders with.
        let gl = matches!(state, RenderingState::RenderingSetup)
            .then(|| Rc::new(unsafe { glow::Context::from_loader_function_cstr(|name| get_proc_address(name)) }));
        for screen in Screen::ALL {
            with_screen(screen, |player| match state {
                RenderingState::RenderingSetup => player.gl = gl.clone(),
                RenderingState::BeforeRendering => player.draw(get_proc_address),
                RenderingState::RenderingTeardown => player.release_gl(),
                _ => {}
            });
        }
    });
    if let Err(err) = hooked {
        eprintln!("player: video cannot be drawn in this window: {err}");
    }
}

/// Hook the pop-out's player into `window`'s rendering: a window of its own has an OpenGL context
/// of its own, which the main window's texture cannot be drawn in. The player (its mpv) is kept
/// for the next pop-out; its renderer goes with the window.
pub fn install_popout(window: &crate::LivePopOut) {
    let main = PLAYERS.with(|players| players.borrow()[Screen::Live.index()].as_ref().map(|player| player.ui.clone()));
    let Some(main) = main else { return };
    PLAYERS.with(|players| {
        let mut players = players.borrow_mut();
        match players[Screen::Popout.index()].as_mut() {
            Some(player) => player.popout = Some(window.as_weak()),
            None => {
                players[Screen::Popout.index()] = Some(Player {
                    screen: Screen::Popout,
                    ui: main,
                    popout: Some(window.as_weak()),
                    mpv: ptr::null_mut(),
                    render: ptr::null_mut(),
                    gl: None,
                    target: None,
                    handed: false,
                    width: 0,
                    height: 0,
                    loaded: false,
                    paused: false,
                    ended: false,
                    waiting: None,
                })
            }
        }
    });
    let hooked = window.window().set_rendering_notifier(|state, api| {
        let GraphicsAPI::NativeOpenGL { get_proc_address } = api else { return };
        // SAFETY: the function pointers come from the context the window renders with.
        let gl = matches!(state, RenderingState::RenderingSetup)
            .then(|| Rc::new(unsafe { glow::Context::from_loader_function_cstr(|name| get_proc_address(name)) }));
        with_screen(Screen::Popout, |player| match state {
            RenderingState::RenderingSetup => player.gl = gl.clone(),
            RenderingState::BeforeRendering => player.draw(get_proc_address),
            RenderingState::RenderingTeardown => player.release_gl(),
            _ => {}
        });
    });
    if let Err(err) = hooked {
        eprintln!("player: video cannot be drawn in the pop-out: {err}");
    }
}

/// Play the file at `path` from its start; a GIF loops. `volume` is 0 … 100.
pub fn play(path: &str, looping: bool, speed: f64, volume: i32, muted: bool) {
    if !path.starts_with("finchgram://") {
        stream::withdraw();
    }
    with_player(|player| {
        if player.mpv.is_null()
            && let Err(err) = player.start()
        {
            eprintln!("player: cannot start mpv: {err}");
            return;
        }
        let load = Load { path: path.to_string(), looping, speed, volume, muted };
        if player.render.is_null() {
            // The renderer is made when the window next renders; the file loads then.
            player.waiting = Some(load);
            player.redraw();
        } else {
            player.load(load);
        }
    });
}

/// Play a file as it arrives, through `stream` (stream.rs), from its start; as `play` otherwise.
/// The viewer's spinner stays until mpv has opened it (`loading`), and comes back while mpv
/// waits for bytes.
pub fn play_stream(stream: std::sync::Arc<stream::Stream>, looping: bool, speed: f64, volume: i32, muted: bool) {
    let url = stream::offer(stream);
    play(&url, looping, speed, volume, muted);
}

pub fn is_loaded() -> bool {
    let mut loaded = false;
    with_player(|player| loaded = player.loaded);
    loaded
}

pub fn set_paused(paused: bool) {
    with_player(|player| {
        if player.ended && !paused {
            // Played to the end: from the start again.
            player.command(&["seek", "0", "absolute"]);
        }
        player.command(&["set", "pause", if paused { "yes" } else { "no" }]);
    });
}

pub fn seek(seconds: f64) {
    with_player(|player| {
        player.command(&["seek", &format!("{seconds:.3}"), "absolute"]);
    });
}

pub fn set_speed(speed: f64) {
    with_player(|player| player.command(&["set", "speed", &speed.to_string()]));
}

/// `volume` is 0 … 100; the mute is a setting of its own.
pub fn set_volume(volume: i32) {
    with_player(|player| player.command(&["set", "volume", &volume.to_string()]));
}

pub fn set_muted(muted: bool) {
    with_player(|player| player.command(&["set", "mute", if muted { "yes" } else { "no" }]));
}

/// Nothing plays: the file is let go and its frame no longer shown; a stream is over.
pub fn stop() {
    stream::withdraw();
    with_player(|player| {
        player.waiting = None;
        if player.loaded {
            player.command(&["stop"]);
        }
        player.loaded = false;
        player.paused = false;
        player.ended = false;
        player.width = 0;
        player.height = 0;
        player.handed = false;
        player.tell(Shown::Frame(Image::default()));
        player.tell(Shown::Playing(false));
        player.tell(Shown::Ended(false));
    });
}

/// The media viewer's player.
fn with_player(change: impl FnOnce(&mut Player)) {
    with_screen(Screen::Viewer, change);
}

fn with_screen(screen: Screen, change: impl FnOnce(&mut Player)) {
    PLAYERS.with(|players| {
        if let Some(player) = players.borrow_mut()[screen.index()].as_mut() {
            change(player);
        }
    });
}

impl Player {
    fn start(&mut self) -> Result<(), String> {
        // SAFETY: plain libmpv calls on a handle we own; strings outlive the calls.
        unsafe {
            let mpv = mpv::mpv_create();
            if mpv.is_null() {
                return Err("mpv_create failed".into());
            }
            // Nothing of the user's own mpv (configuration, scripts) is read, and nothing but what
            // the app asks for is shown or listened to. A video stays on its last frame at its end;
            // a live stream's end is the end.
            let keep_open = if self.screen == Screen::Viewer { "yes" } else { "no" };
            let roots = if self.screen.networked() { crate::live::roots::file() } else { None };
            let roots = roots.map(|path| path.to_string_lossy().to_string()).unwrap_or_default();
            let reach: Vec<(&str, &str)> = match self.screen {
                Screen::Viewer => LOCAL_PROTOCOLS.to_vec(),
                Screen::Live | Screen::Popout if roots.is_empty() => NETWORK_PROTOCOLS.to_vec(),
                Screen::Live | Screen::Popout => {
                    let mut options = NETWORK_PROTOCOLS.to_vec();
                    options.push(("tls-verify", "yes"));
                    options.push(("tls-ca-file", roots.as_str()));
                    options
                }
            };
            for (name, value) in reach.into_iter().chain([
                ("config", "no"),
                ("vo", "libmpv"),
                ("hwdec", "auto-safe"),
                ("keep-open", keep_open),
                ("idle", "yes"),
                ("terminal", "no"),
                ("input-default-bindings", "no"),
                ("input-vo-keyboard", "no"),
                ("osd-level", "0"),
                ("audio-display", "no"),
                ("sub-auto", "no"),
            ]) {
                let (name, value) = (CString::new(name).unwrap_or_default(), CString::new(value).unwrap_or_default());
                let status = mpv::mpv_set_option_string(mpv, name.as_ptr(), value.as_ptr());
                if status < 0 {
                    eprintln!("player: option {name:?}: {}", error_string(status));
                }
            }
            let status = mpv::mpv_initialize(mpv);
            if status < 0 {
                mpv::mpv_terminate_destroy(mpv);
                return Err(error_string(status));
            }
            for (id, name, format) in [
                (POSITION, c"time-pos", mpv::MPV_FORMAT_DOUBLE),
                (PAUSED, c"pause", mpv::MPV_FORMAT_FLAG),
                (ENDED, c"eof-reached", mpv::MPV_FORMAT_FLAG),
                (WIDTH, c"dwidth", mpv::MPV_FORMAT_INT64),
                (HEIGHT, c"dheight", mpv::MPV_FORMAT_INT64),
                (BUFFERING, c"paused-for-cache", mpv::MPV_FORMAT_FLAG),
            ] {
                mpv::mpv_observe_property(mpv, id, name.as_ptr(), format);
            }
            // Files played as they arrive ("finchgram://" URLs, read through stream.rs), or a live
            // stream as it comes ("finchgram-live://", pipe.rs).
            let status = match self.screen {
                Screen::Viewer => mpv::mpv_stream_cb_add_ro(mpv, c"finchgram".as_ptr(), ptr::null_mut(), stream::open),
                Screen::Live => mpv::mpv_stream_cb_add_ro(mpv, pipe::PROTOCOL.as_ptr(), ptr::null_mut(), pipe::open),
                // The pop-out plays the directory's streams by their addresses only.
                Screen::Popout => 0,
            };
            if status < 0 {
                eprintln!("player: cannot play streams of our own: {}", error_string(status));
            }
            mpv::mpv_set_wakeup_callback(mpv, wake_for_events, self.screen.index() as *mut c_void);
            self.mpv = mpv;
        }
        Ok(())
    }

    fn property_i64(&self, name: &CStr) -> i64 {
        let mut value: i64 = 0;
        // SAFETY: an int64 property read into a local.
        let status = unsafe { mpv::mpv_get_property(self.mpv, name.as_ptr(), mpv::MPV_FORMAT_INT64, (&mut value as *mut i64).cast()) };
        if status < 0 { 0 } else { value }
    }

    /// A property by name, as a number; None when mpv has none (no stream, no such track).
    fn property_number(&self, name: &str) -> Option<i64> {
        if self.mpv.is_null() {
            return None;
        }
        let name = CString::new(name).ok()?;
        let mut value: i64 = 0;
        // SAFETY: an int64 property read into a local.
        let status = unsafe { mpv::mpv_get_property(self.mpv, name.as_ptr(), mpv::MPV_FORMAT_INT64, (&mut value as *mut i64).cast()) };
        (status >= 0).then_some(value)
    }

    /// A property by name, as mpv writes it; None when it has none.
    fn property_text(&self, name: &str) -> Option<String> {
        if self.mpv.is_null() {
            return None;
        }
        let name = CString::new(name).ok()?;
        // SAFETY: mpv returns a string it allocated, or null; it is copied, then freed by mpv.
        unsafe {
            let text = mpv::mpv_get_property_string(self.mpv, name.as_ptr());
            if text.is_null() {
                return None;
            }
            let copied = CStr::from_ptr(text).to_string_lossy().to_string();
            mpv::mpv_free(text.cast());
            Some(copied)
        }
    }

    fn load(&mut self, load: Load) {
        self.command(&["set", "loop-file", if load.looping { "inf" } else { "no" }]);
        self.command(&["set", "speed", &load.speed.to_string()]);
        self.command(&["set", "volume", &load.volume.to_string()]);
        self.command(&["set", "mute", if load.muted { "yes" } else { "no" }]);
        self.command(&["loadfile", &load.path, "replace"]);
        self.command(&["set", "pause", "no"]);
        self.loaded = true;
        self.paused = false;
        self.ended = false;
        self.report();
    }

    fn command(&self, arguments: &[&str]) {
        if self.mpv.is_null() {
            return;
        }
        let owned: Vec<CString> = arguments.iter().filter_map(|argument| CString::new(*argument).ok()).collect();
        let mut pointers: Vec<*const c_char> = owned.iter().map(|argument| argument.as_ptr()).collect();
        pointers.push(ptr::null());
        // SAFETY: a null-terminated array of strings that live until the call returns.
        let status = unsafe { mpv::mpv_command(self.mpv, pointers.as_mut_ptr()) };
        if status < 0 {
            eprintln!("player: {arguments:?}: {}", error_string(status));
        }
    }

    /// What mpv has to say since it last woke us.
    fn drain_events(&mut self) {
        if self.mpv.is_null() {
            return;
        }
        loop {
            // SAFETY: mpv_wait_event returns a valid event until the next call.
            let event = unsafe { &*mpv::mpv_wait_event(self.mpv, 0.0) };
            match event.event_id {
                mpv::MPV_EVENT_NONE | mpv::MPV_EVENT_SHUTDOWN => break,
                mpv::MPV_EVENT_PROPERTY_CHANGE => {
                    // SAFETY: a property change carries an mpv_event_property in the format we asked for.
                    let property = unsafe { &*(event.data as *const mpv::mpv_event_property) };
                    if property.data.is_null() {
                        continue;
                    }
                    unsafe {
                        match event.reply_userdata {
                            POSITION => {
                                let position = *(property.data as *const f64);
                                self.tell(Shown::Position(position as f32));
                            }
                            PAUSED => self.paused = *(property.data as *const c_int) != 0,
                            ENDED => self.ended = *(property.data as *const c_int) != 0,
                            WIDTH => self.width = *(property.data as *const i64),
                            HEIGHT => self.height = *(property.data as *const i64),
                            BUFFERING => {
                                let waiting = *(property.data as *const c_int) != 0 && self.loaded;
                                self.tell(Shown::Loading(waiting));
                            }
                            _ => {}
                        }
                    }
                    self.report();
                }
                // Open, and playing: a file that arrives as it plays no longer waits to start.
                mpv::MPV_EVENT_FILE_LOADED => {
                    self.tell(Shown::Loading(false));
                    if self.screen == Screen::Live {
                        let _ = slint::invoke_from_event_loop(live::loaded);
                    }
                    if self.screen == Screen::Popout {
                        let _ = slint::invoke_from_event_loop(crate::live::popout::loaded);
                    }
                }
                mpv::MPV_EVENT_END_FILE => {
                    // SAFETY: an end-of-file event carries an mpv_event_end_file.
                    let end = unsafe { &*(event.data as *const mpv::mpv_event_end_file) };
                    let failed = end.reason == mpv::MPV_END_FILE_REASON_ERROR;
                    if failed {
                        eprintln!("player: cannot play the file: {}", error_string(end.error));
                        self.loaded = false;
                        self.tell(Shown::Loading(false));
                        self.report();
                    }
                    // The live stream's end, or its failure: the live side decides what follows.
                    if self.screen == Screen::Live && (failed || end.reason == mpv::MPV_END_FILE_REASON_EOF) {
                        self.loaded = false;
                        self.report();
                        let _ = slint::invoke_from_event_loop(move || live::ended(failed));
                    }
                    if self.screen == Screen::Popout && (failed || end.reason == mpv::MPV_END_FILE_REASON_EOF) {
                        self.loaded = false;
                        self.report();
                        let _ = slint::invoke_from_event_loop(move || crate::live::popout::ended(failed));
                    }
                }
                _ => {}
            }
        }
    }

    /// Playing, paused or at the end: the page's controls follow.
    fn report(&self) {
        let (playing, ended) = (self.loaded && !self.paused && !self.ended, self.loaded && self.ended);
        self.tell(Shown::Playing(playing));
        self.tell(Shown::Ended(ended));
    }

    /// The window this player draws into renders again.
    fn redraw(&self) {
        match self.screen {
            Screen::Popout => {
                if let Some(window) = self.popout.as_ref().and_then(slint::Weak::upgrade) {
                    window.window().request_redraw();
                }
            }
            _ => {
                if let Some(ui) = self.ui.upgrade() {
                    ui.window().request_redraw();
                }
            }
        }
    }

    /// Tell the page this player draws into.
    fn tell(&self, shown: Shown) {
        if self.screen == Screen::Popout {
            let Some(window) = self.popout.as_ref().and_then(slint::Weak::upgrade) else { return };
            match shown {
                Shown::Frame(frame) => window.set_frame(frame),
                Shown::Loading(loading) => window.set_buffering(loading),
                Shown::Playing(_) | Shown::Position(_) | Shown::Ended(_) => {}
            }
            return;
        }
        let Some(ui) = self.ui.upgrade() else { return };
        match self.screen {
            Screen::Viewer => {
                let viewer = ui.global::<Viewer>();
                match shown {
                    Shown::Frame(frame) => viewer.set_frame(frame),
                    Shown::Position(position) => viewer.set_position(position),
                    Shown::Loading(loading) => viewer.set_loading(loading),
                    Shown::Playing(playing) => viewer.set_playing(playing),
                    Shown::Ended(ended) => viewer.set_ended(ended),
                }
            }
            Screen::Live => {
                let live = ui.global::<Live>();
                match shown {
                    Shown::Frame(frame) => live.set_frame(frame),
                    Shown::Loading(loading) => live.set_buffering(loading),
                    Shown::Playing(playing) => live.set_playing(playing),
                    // A live stream has no position to seek to, and no end to stay on.
                    Shown::Position(_) | Shown::Ended(_) => {}
                }
            }
            Screen::Popout => {}
        }
    }

    /// The window is about to render: when mpv has a new frame, draw it into our texture.
    fn draw(&mut self, get_proc_address: &dyn Fn(&CStr) -> *const c_void) {
        if self.mpv.is_null() || (!self.loaded && self.waiting.is_none()) {
            return;
        }
        let Some(gl) = self.gl.clone() else { return };
        if self.render.is_null() {
            let saved = GlState::save(&gl);
            let made = self.create_render(get_proc_address);
            saved.restore(&gl);
            if let Err(err) = made {
                eprintln!("player: cannot draw video: {err}");
                self.waiting = None;
                self.loaded = false;
                self.report();
                return;
            }
        }
        if let Some(load) = self.waiting.take() {
            self.load(load);
            return;
        }
        // SAFETY: the render context is ours and the window's context is current.
        let flags = unsafe { mpv::mpv_render_context_update(self.render) };
        if flags & mpv::MPV_RENDER_UPDATE_FRAME == 0 {
            return;
        }
        // A frame may come before its size is reported: mpv waits until it is drawn, so ask.
        if self.width <= 0 || self.height <= 0 {
            self.width = self.property_i64(c"dwidth");
            self.height = self.property_i64(c"dheight");
        }
        let (width, height) = fit(self.width, self.height);
        if width <= 0 || height <= 0 {
            return;
        }
        let saved = GlState::save(&gl);
        let replaced = self.prepare_target(&gl, width, height);
        let Some(target) = self.target.as_ref() else {
            saved.restore(&gl);
            return;
        };
        let mut fbo = mpv::mpv_opengl_fbo { fbo: target.framebuffer.0.get() as c_int, w: width, h: height, internal_format: glow::RGBA8 as c_int };
        let mut flip: c_int = 0;
        let mut params = [
            mpv::mpv_render_param { kind: mpv::MPV_RENDER_PARAM_OPENGL_FBO, data: (&mut fbo as *mut mpv::mpv_opengl_fbo).cast() },
            mpv::mpv_render_param { kind: mpv::MPV_RENDER_PARAM_FLIP_Y, data: (&mut flip as *mut c_int).cast() },
            mpv::mpv_render_param { kind: mpv::MPV_RENDER_PARAM_INVALID, data: ptr::null_mut() },
        ];
        // SAFETY: the parameters live until the call returns.
        unsafe { mpv::mpv_render_context_render(self.render, params.as_mut_ptr()) };
        saved.restore(&gl);
        if replaced || !self.handed {
            // Unflipped, mpv writes the top of the picture to the texture's first row, and that is
            // the row Slint draws at the top with TopLeft. (BottomLeft showed videos upside down.)
            // SAFETY: the texture stays alive until it is replaced, and then the image with it.
            let frame = unsafe {
                BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture(target.texture.0, [width as u32, height as u32].into())
                    .origin(BorrowedOpenGLTextureOrigin::TopLeft)
                    .build()
            };
            self.tell(Shown::Frame(frame));
            self.handed = true;
        }
    }

    fn create_render(&mut self, get_proc_address: &dyn Fn(&CStr) -> *const c_void) -> Result<(), String> {
        let mut init = mpv::mpv_opengl_init_params {
            get_proc_address: lookup_gl_function,
            get_proc_address_ctx: (&get_proc_address as *const &dyn Fn(&CStr) -> *const c_void).cast_mut().cast(),
        };
        let api = c"opengl";
        let mut params = [
            mpv::mpv_render_param { kind: mpv::MPV_RENDER_PARAM_API_TYPE, data: api.as_ptr().cast_mut().cast() },
            mpv::mpv_render_param {
                kind: mpv::MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: (&mut init as *mut mpv::mpv_opengl_init_params).cast(),
            },
            mpv::mpv_render_param { kind: mpv::MPV_RENDER_PARAM_INVALID, data: ptr::null_mut() },
        ];
        // SAFETY: mpv looks up the GL functions during this call only, while `get_proc_address`
        // is valid; the window's context is current.
        unsafe {
            let status = mpv::mpv_render_context_create(&mut self.render, self.mpv, params.as_mut_ptr());
            if status < 0 {
                self.render = ptr::null_mut();
                return Err(error_string(status));
            }
            mpv::mpv_render_context_set_update_callback(self.render, wake_for_frame, self.screen.index() as *mut c_void);
        }
        Ok(())
    }

    /// A texture of the video's size, and its framebuffer; true when they were made anew.
    fn prepare_target(&mut self, gl: &glow::Context, width: i32, height: i32) -> bool {
        if let Some(target) = &self.target
            && (target.width, target.height) == (width, height)
        {
            return false;
        }
        self.delete_target(gl);
        // SAFETY: plain GL calls in the current context; the caller restores the state after.
        unsafe {
            let (Ok(texture), Ok(framebuffer)) = (gl.create_texture(), gl.create_framebuffer()) else { return false };
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                width,
                height,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(texture), 0);
            self.target = Some(Target { texture, framebuffer, width, height });
        }
        true
    }

    fn delete_target(&mut self, gl: &glow::Context) {
        if let Some(target) = self.target.take() {
            self.tell(Shown::Frame(Image::default()));
            self.handed = false;
            // SAFETY: our own objects, in the context that made them.
            unsafe {
                gl.delete_framebuffer(target.framebuffer);
                gl.delete_texture(target.texture);
            }
        }
    }

    /// The window's context goes away: so do mpv's renderer and our texture.
    fn release_gl(&mut self) {
        if !self.render.is_null() {
            // SAFETY: freed once, with the context still current.
            unsafe { mpv::mpv_render_context_free(self.render) };
            self.render = ptr::null_mut();
        }
        if let Some(gl) = self.gl.take() {
            self.delete_target(&gl);
        }
    }
}

/// mpv asks for an OpenGL function; `context` is the window's lookup, valid during
/// mpv_render_context_create.
unsafe extern "C" fn lookup_gl_function(context: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: `context` points at the `&dyn Fn` passed in create_render, alive during the call.
    unsafe {
        let lookup = &*(context as *const &dyn Fn(&CStr) -> *const c_void);
        lookup(CStr::from_ptr(name)).cast_mut()
    }
}

/// mpv has events: the UI thread reads them (once for however many wake-ups came meanwhile).
/// `data` is the player's screen.
unsafe extern "C" fn wake_for_events(data: *mut c_void) {
    let screen = Screen::from_data(data);
    if !EVENTS_ASKED[screen.index()].swap(true, Ordering::AcqRel) {
        let _ = slint::invoke_from_event_loop(move || {
            EVENTS_ASKED[screen.index()].store(false, Ordering::Release);
            with_screen(screen, Player::drain_events);
        });
    }
}

/// mpv has a new frame: the window renders, and draw() fetches it. `data` is the player's screen.
unsafe extern "C" fn wake_for_frame(data: *mut c_void) {
    let screen = Screen::from_data(data);
    if !FRAME_ASKED[screen.index()].swap(true, Ordering::AcqRel) {
        let _ = slint::invoke_from_event_loop(move || {
            FRAME_ASKED[screen.index()].store(false, Ordering::Release);
            with_screen(screen, |player| player.redraw());
        });
    }
}

fn error_string(status: c_int) -> String {
    // SAFETY: mpv_error_string returns a static string for any value.
    unsafe { CStr::from_ptr(mpv::mpv_error_string(status)) }.to_string_lossy().into_owned()
}

/// The video's size, no larger than LARGEST on its longer side.
fn fit(width: i64, height: i64) -> (i32, i32) {
    let longer = width.max(height);
    if longer <= 0 {
        return (0, 0);
    }
    let scale = (LARGEST as f64 / longer as f64).min(1.0);
    ((width as f64 * scale).round() as i32, (height as f64 * scale).round() as i32)
}

/// The OpenGL state Slint's renderer relies on, which mpv (and making our texture) may change.
struct GlState {
    framebuffer: i32,
    viewport: [i32; 4],
    scissor: bool,
    scissor_box: [i32; 4],
    blend: bool,
    blend_func: [i32; 4],
    clear_color: [f32; 4],
    program: i32,
    vertex_array: i32,
    array_buffer: i32,
    active_texture: i32,
    texture: i32,
    unpack_alignment: i32,
}

impl GlState {
    fn save(gl: &glow::Context) -> GlState {
        // SAFETY: queries in the current context.
        unsafe {
            let mut viewport = [0; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            let mut scissor_box = [0; 4];
            gl.get_parameter_i32_slice(glow::SCISSOR_BOX, &mut scissor_box);
            let mut clear_color = [0.0; 4];
            gl.get_parameter_f32_slice(glow::COLOR_CLEAR_VALUE, &mut clear_color);
            GlState {
                framebuffer: gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING),
                viewport,
                scissor: gl.is_enabled(glow::SCISSOR_TEST),
                scissor_box,
                blend: gl.is_enabled(glow::BLEND),
                blend_func: [
                    gl.get_parameter_i32(glow::BLEND_SRC_RGB),
                    gl.get_parameter_i32(glow::BLEND_DST_RGB),
                    gl.get_parameter_i32(glow::BLEND_SRC_ALPHA),
                    gl.get_parameter_i32(glow::BLEND_DST_ALPHA),
                ],
                clear_color,
                program: gl.get_parameter_i32(glow::CURRENT_PROGRAM),
                vertex_array: gl.get_parameter_i32(glow::VERTEX_ARRAY_BINDING),
                array_buffer: gl.get_parameter_i32(glow::ARRAY_BUFFER_BINDING),
                active_texture: gl.get_parameter_i32(glow::ACTIVE_TEXTURE),
                texture: gl.get_parameter_i32(glow::TEXTURE_BINDING_2D),
                unpack_alignment: gl.get_parameter_i32(glow::UNPACK_ALIGNMENT),
            }
        }
    }

    fn restore(&self, gl: &glow::Context) {
        let name = |value: i32| NonZeroU32::new(value as u32);
        // SAFETY: setting back what save() read, in the same context.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, name(self.framebuffer).map(glow::NativeFramebuffer));
            gl.viewport(self.viewport[0], self.viewport[1], self.viewport[2], self.viewport[3]);
            if self.scissor {
                gl.enable(glow::SCISSOR_TEST);
            } else {
                gl.disable(glow::SCISSOR_TEST);
            }
            gl.scissor(self.scissor_box[0], self.scissor_box[1], self.scissor_box[2], self.scissor_box[3]);
            if self.blend {
                gl.enable(glow::BLEND);
            } else {
                gl.disable(glow::BLEND);
            }
            let [src_rgb, dst_rgb, src_alpha, dst_alpha] = self.blend_func.map(|value| value as u32);
            gl.blend_func_separate(src_rgb, dst_rgb, src_alpha, dst_alpha);
            let [red, green, blue, alpha] = self.clear_color;
            gl.clear_color(red, green, blue, alpha);
            gl.use_program(name(self.program).map(glow::NativeProgram));
            gl.bind_vertex_array(name(self.vertex_array).map(glow::NativeVertexArray));
            gl.bind_buffer(glow::ARRAY_BUFFER, name(self.array_buffer).map(glow::NativeBuffer));
            gl.active_texture(self.active_texture as u32);
            gl.bind_texture(glow::TEXTURE_2D, name(self.texture).map(glow::NativeTexture));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, self.unpack_alignment);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_large_video_is_drawn_no_larger_than_it_needs() {
        assert_eq!(fit(3840, 2160), (2560, 1440));
        assert_eq!(fit(1280, 720), (1280, 720));
        assert_eq!(fit(0, 0), (0, 0));
    }
}
