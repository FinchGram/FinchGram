//! Video through libmpv (vendor/mpv; docs/architecture.md, "Media playback"): the media viewer's
//! player, and the live stream's (`live`, docs/live-streams.md), each an mpv of its own. Voice
//! messages play through an mpv of their own, with no video (`audio`). mpv decodes, with
//! VideoToolbox where it can, and draws with OpenGL, the one way libmpv has of drawing on the GPU:
//! in a context of the player's own with no window, into a surface it shares with the window
//! (src/platform/video.rs). The window is drawn by Slint's Skia renderer over Metal, which reads
//! that surface as a texture, and the page shows it as an image (Viewer.frame, Live.frame). Each
//! time the window is about to render, the player has mpv draw its newest frame (docs/skia.md).
//! What mpv reports goes to that page's global: the position, playing or not, the end.
//!
//! Everything runs on the UI thread; mpv's own threads only ask the event loop to look, and read
//! a file played as it arrives (`stream`) or a live stream as it comes (`pipe`).

pub mod audio;
pub mod decode;
#[cfg(test)]
mod frames;
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
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use slint::wgpu_30::wgpu;
use slint::{ComponentHandle, GraphicsAPI, Image, RenderingState};

use crate::platform::video;
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
    /// Created when the first video plays, with its renderer and the OpenGL context that draws in:
    /// the player's own, no window's (src/platform/video.rs).
    mpv: *mut mpv::mpv_handle,
    render: *mut mpv::mpv_render_context,
    context: Option<video::Context>,
    /// What mpv draws into and the page shows: made when a frame comes, anew when the video's size
    /// changes, and gone with the window's renderer, whose device its texture is of.
    surface: Option<video::Surface>,
    /// The page has the surface's texture (Viewer.frame). stop() takes it from the page and keeps
    /// the surface: the next video of the same size draws into it, and the page has to be given it
    /// again. (Up to v0.3.14 it was given only with a new texture: a video the size of the one
    /// before stayed on its still while it played.)
    handed: bool,
    /// mpv has drawn a frame of the file that plays: it can draw it once more, into a new surface.
    drawn: bool,
    /// The video's size as shown (mpv's dwidth, dheight).
    width: i64,
    height: i64,
    loaded: bool,
    paused: bool,
    ended: bool,
}

struct Load {
    path: String,
    looping: bool,
    speed: f64,
    /// 0 … 100.
    volume: i32,
    muted: bool,
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
            players[screen.index()] = Some(Player::idle(screen, ui.as_weak(), None));
        }
    });
    let hooked = ui.window().set_rendering_notifier(|state, api| {
        for screen in Screen::ALL {
            with_screen(screen, |player| player.follow(&state, api));
        }
    });
    if let Err(err) = hooked {
        eprintln!("player: video cannot be drawn in this window: {err}");
    }
}

/// Hook the pop-out's player into `window`'s rendering: it draws when the pop-out renders. The
/// player (its mpv) is kept for the next pop-out; its surface goes with the window's renderer.
pub fn install_popout(window: &crate::LivePopOut) {
    let main = PLAYERS.with(|players| players.borrow()[Screen::Live.index()].as_ref().map(|player| player.ui.clone()));
    let Some(main) = main else { return };
    PLAYERS.with(|players| {
        let mut players = players.borrow_mut();
        match players[Screen::Popout.index()].as_mut() {
            Some(player) => player.popout = Some(window.as_weak()),
            None => players[Screen::Popout.index()] = Some(Player::idle(Screen::Popout, main, Some(window.as_weak()))),
        }
    });
    let hooked = window.window().set_rendering_notifier(|state, api| with_screen(Screen::Popout, |player| player.follow(&state, api)));
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
        player.load(Load { path: path.to_string(), looping, speed, volume, muted });
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
        if player.loaded {
            player.command(&["stop"]);
        }
        player.loaded = false;
        player.paused = false;
        player.ended = false;
        player.width = 0;
        player.height = 0;
        player.handed = false;
        player.drawn = false;
        player.tell(Shown::Frame(Image::default()));
        player.tell(Shown::Playing(false));
        player.tell(Shown::Ended(false));
    });
}

/// The app ends: every surface goes now, while the windows' renderer still lives. What holds a
/// texture of the renderer's device must not be left for the end of the thread: a Metal device
/// torn down there ends the process badly (docs/skia.md).
pub fn shut_down() {
    for screen in [Screen::Viewer, Screen::Live, Screen::Popout] {
        with_screen(screen, Player::release_surface);
    }
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
    /// A player that plays nothing yet: mpv starts with the first video.
    fn idle(screen: Screen, ui: slint::Weak<MainWindow>, popout: Option<slint::Weak<crate::LivePopOut>>) -> Player {
        Player {
            screen,
            ui,
            popout,
            mpv: ptr::null_mut(),
            render: ptr::null_mut(),
            context: None,
            surface: None,
            handed: false,
            drawn: false,
            width: 0,
            height: 0,
            loaded: false,
            paused: false,
            ended: false,
        }
    }

    /// What the window's renderer says (its rendering notifier): about to render, or going away.
    fn follow(&mut self, state: &RenderingState, api: &GraphicsAPI) {
        match state {
            RenderingState::BeforeRendering => {
                // The window is drawn over Metal, through wgpu (src/platform/, select_backend).
                if let GraphicsAPI::WGPU30 { device, .. } = api {
                    self.draw(device);
                }
            }
            RenderingState::RenderingTeardown => self.release_surface(),
            _ => {}
        }
    }

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
            // mpv's renderer, in an OpenGL context of the player's own (mpv's video output needs
            // it before a file loads): mpv looks its OpenGL functions up now, and sets the hardware
            // decoder's hand-over up for this context.
            let context = match video::Context::new() {
                Ok(context) => context,
                Err(err) => {
                    mpv::mpv_terminate_destroy(mpv);
                    return Err(err);
                }
            };
            let mut render: *mut mpv::mpv_render_context = ptr::null_mut();
            let status = {
                let _entered = context.enter();
                let mut init = mpv::mpv_opengl_init_params { get_proc_address: video::lookup, get_proc_address_ctx: ptr::null_mut() };
                let api = c"opengl";
                let mut params = [
                    mpv::mpv_render_param { kind: mpv::MPV_RENDER_PARAM_API_TYPE, data: api.as_ptr().cast_mut().cast() },
                    mpv::mpv_render_param {
                        kind: mpv::MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                        data: (&mut init as *mut mpv::mpv_opengl_init_params).cast(),
                    },
                    mpv::mpv_render_param { kind: mpv::MPV_RENDER_PARAM_INVALID, data: ptr::null_mut() },
                ];
                mpv::mpv_render_context_create(&mut render, mpv, params.as_mut_ptr())
            };
            if status < 0 {
                mpv::mpv_terminate_destroy(mpv);
                return Err(format!("mpv's renderer: {}", error_string(status)));
            }
            mpv::mpv_render_context_set_update_callback(render, wake_for_frame, self.screen.index() as *mut c_void);
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
            self.render = render;
            self.context = Some(context);
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
        self.drawn = false;
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

    /// The window is about to render: when mpv has a new frame, it draws it into the surface.
    /// `device` is the one the window is drawn with: the texture the page is given is of it.
    fn draw(&mut self, device: &wgpu::Device) {
        if self.render.is_null() || !self.loaded {
            return;
        }
        let Some(entered) = self.context.as_ref().map(video::Context::enter) else { return };
        // SAFETY: the render context is ours and its OpenGL context is entered.
        let flags = unsafe { mpv::mpv_render_context_update(self.render) };
        // A new frame; or the frame there is, once more, when its surface went with the window's
        // renderer (the window was hidden while the video was held).
        let again = self.drawn && self.surface.is_none();
        if flags & mpv::MPV_RENDER_UPDATE_FRAME == 0 && !again {
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
        let replaced = match self.prepare_surface(device, width, height) {
            Ok(replaced) => replaced,
            Err(err) => {
                eprintln!("player: cannot draw video: {err}");
                self.command(&["stop"]);
                self.loaded = false;
                self.tell(Shown::Loading(false));
                self.report();
                return;
            }
        };
        let Some(surface) = self.surface.as_ref() else { return };
        let mut fbo = mpv::mpv_opengl_fbo { fbo: surface.framebuffer as c_int, w: width, h: height, internal_format: video::FRAMEBUFFER_FORMAT };
        let mut flip: c_int = 0;
        let mut params = [
            mpv::mpv_render_param { kind: mpv::MPV_RENDER_PARAM_OPENGL_FBO, data: (&mut fbo as *mut mpv::mpv_opengl_fbo).cast() },
            mpv::mpv_render_param { kind: mpv::MPV_RENDER_PARAM_FLIP_Y, data: (&mut flip as *mut c_int).cast() },
            mpv::mpv_render_param { kind: mpv::MPV_RENDER_PARAM_INVALID, data: ptr::null_mut() },
        ];
        // SAFETY: the parameters live until the call returns. The flush hands OpenGL's drawing to
        // the GPU before the window's own, which is sent after this, reads the surface.
        unsafe { mpv::mpv_render_context_render(self.render, params.as_mut_ptr()) };
        video::flush();
        drop(entered);
        self.drawn = true;
        if replaced || !self.handed {
            // Unflipped, mpv writes the top of the picture to the surface's first row, and that is
            // the row Metal has at the top. (Checked with pixel values, docs/skia.md: 0.2.0 showed
            // videos upside down.)
            match Image::try_from(surface.shown.clone()) {
                Ok(frame) => {
                    self.tell(Shown::Frame(frame));
                    self.handed = true;
                }
                Err(err) => eprintln!("player: the window does not take the video's texture: {err}"),
            }
        }
    }

    /// A surface of the video's size, with the player's context entered; true when it was made
    /// anew.
    fn prepare_surface(&mut self, device: &wgpu::Device, width: i32, height: i32) -> Result<bool, String> {
        if let Some(surface) = &self.surface
            && (surface.width, surface.height) == (width, height)
        {
            return Ok(false);
        }
        if let Some(old) = self.surface.take() {
            self.tell(Shown::Frame(Image::default()));
            self.handed = false;
            old.release();
        }
        let context = self.context.as_ref().ok_or("the player has no OpenGL context")?;
        self.surface = Some(video::Surface::new(context, device, width, height)?);
        Ok(true)
    }

    /// The window's renderer goes away (the window was hidden, or the app ends): the surface goes
    /// with it, since its texture is of that renderer's device. mpv and its own renderer stay, and
    /// draw into a new surface when the window renders again.
    fn release_surface(&mut self) {
        let Some(surface) = self.surface.take() else { return };
        self.tell(Shown::Frame(Image::default()));
        self.handed = false;
        if let Some(context) = self.context.as_ref() {
            let _entered = context.enter();
            surface.release();
        }
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
