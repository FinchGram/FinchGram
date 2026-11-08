//! The live streams' player (docs/live-streams.md): an mpv of its own, drawn into Live.frame as
//! the media viewer's is into Viewer.frame. It plays either side of the Live view, one at a time:
//! Telegram's stream, read from a pipe (pipe.rs) that src/telegram/live.rs fills a second at a
//! time, each second remuxed (remux.rs); or a stream of the live directory, by its address
//! (src/live/watch.rs: HLS, DASH, Icecast), over the network that player alone may reach
//! (mod.rs's NETWORK_PROTOCOLS), its certificates checked against the Keychain's roots.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use slint::ComponentHandle;

use super::{Load, Screen, Shown, pipe, with_screen};

/// Which side of the Live view the player plays for: what it tells of a start and an end goes
/// to that side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Telegram,
    Directory,
}

type Ended = Box<dyn Fn(bool)>;
type Loaded = Box<dyn Fn()>;

thread_local! {
    static OWNER: Cell<Owner> = const { Cell::new(Owner::Telegram) };
    static ENDED: RefCell<[Option<Ended>; 2]> = const { RefCell::new([None, None]) };
    static LOADED: RefCell<[Option<Loaded>; 2]> = const { RefCell::new([None, None]) };
}

/// Which side plays now (or played last).
pub fn owner() -> Owner {
    OWNER.get()
}

fn index(owner: Owner) -> usize {
    match owner {
        Owner::Telegram => 0,
        Owner::Directory => 1,
    }
}

/// Play the stream that comes through `pipe`, from its start; `volume` is 0 … 100. A stream
/// played before is given up.
pub fn play(pipe: Arc<pipe::Pipe>, volume: i32, muted: bool) {
    let url = pipe::offer(pipe);
    start(Owner::Telegram, url, volume, muted);
}

/// Play a stream of the live directory by its address (https://…/stream.m3u8, an Icecast
/// station's), from the live edge.
pub fn play_url(url: &str, volume: i32, muted: bool) {
    pipe::withdraw();
    start(Owner::Directory, url.to_string(), volume, muted);
}

fn start(owner: Owner, url: String, volume: i32, muted: bool) {
    OWNER.set(owner);
    with_screen(Screen::Live, |player| {
        if player.mpv.is_null()
            && let Err(err) = player.start()
        {
            eprintln!("live player: cannot start mpv: {err}");
            return;
        }
        let load = Load { path: url, looping: false, speed: 1.0, volume, muted };
        player.tell(Shown::Loading(true));
        if player.render.is_null() {
            // The renderer is made when the window next renders; the stream loads then.
            player.waiting = Some(load);
            if let Some(ui) = player.ui.upgrade() {
                ui.window().request_redraw();
            }
        } else {
            player.load(load);
        }
    });
}

/// Nothing plays: the stream is given up and its picture no longer shown.
pub fn stop() {
    pipe::withdraw();
    with_screen(Screen::Live, |player| {
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
        player.tell(Shown::Frame(slint::Image::default()));
        player.tell(Shown::Playing(false));
        player.tell(Shown::Loading(false));
    });
}

/// Held, or going on from where it was held (mpv keeps what came meanwhile, as far as its cache
/// goes); a directory stream only.
pub fn set_paused(paused: bool) {
    with_screen(Screen::Live, |player| {
        player.command(&["set", "pause", if paused { "yes" } else { "no" }]);
        player.paused = paused;
        player.report();
    });
}

/// The stream's variants, by mpv's video track ids, with their heights and bitrates: what the
/// quality menu lists.
pub fn variants() -> Vec<(i64, i64, i64)> {
    let mut found = Vec::new();
    with_screen(Screen::Live, |player| {
        let count = player.property_number("track-list/count").unwrap_or(0);
        for track in 0..count {
            if player.property_text(&format!("track-list/{track}/type")).as_deref() != Some("video") {
                continue;
            }
            let id = player.property_number(&format!("track-list/{track}/id")).unwrap_or(0);
            let height = player.property_number(&format!("track-list/{track}/demux-h")).unwrap_or(0);
            let bitrate = player.property_number(&format!("track-list/{track}/demux-bitrate")).unwrap_or(0);
            found.push((id, height, bitrate));
        }
    });
    found
}

/// Play this variant (a video track id), or mpv's choice (None: the highest).
pub fn choose_variant(id: Option<i64>) {
    with_screen(Screen::Live, |player| match id {
        Some(id) => player.command(&["set", "vid", &id.to_string()]),
        None => player.command(&["set", "vid", "auto"]),
    });
}

/// What plays now: the video's height and the bitrate mpv measures (bits a second), the format.
pub fn playing_facts() -> (i64, i64, String) {
    let mut facts = (0, 0, String::new());
    with_screen(Screen::Live, |player| {
        facts = (
            player.property_number("video-params/h").unwrap_or(0),
            player.property_number("video-bitrate").or_else(|| player.property_number("audio-bitrate")).unwrap_or(0),
            player.property_text("file-format").unwrap_or_default(),
        );
    });
    facts
}

/// `volume` is 0 … 100; the mute is a setting of its own.
pub fn set_volume(volume: i32) {
    with_screen(Screen::Live, |player| player.command(&["set", "volume", &volume.to_string()]));
}

pub fn set_muted(muted: bool) {
    with_screen(Screen::Live, |player| player.command(&["set", "mute", if muted { "yes" } else { "no" }]));
}

/// Tell `ended` when mpv has come to the end of the `owner`'s stream (false) or could not play it
/// (true).
pub fn on_ended(owner: Owner, ended: impl Fn(bool) + 'static) {
    ENDED.with(|kept| kept.borrow_mut()[index(owner)] = Some(Box::new(ended)));
}

/// Tell `loaded` when mpv has opened the `owner`'s stream.
pub fn on_loaded(owner: Owner, loaded: impl Fn() + 'static) {
    LOADED.with(|kept| kept.borrow_mut()[index(owner)] = Some(Box::new(loaded)));
}

pub(super) fn ended(failed: bool) {
    let owner = OWNER.get();
    ENDED.with(|kept| {
        if let Some(ended) = &kept.borrow()[index(owner)] {
            ended(failed);
        }
    });
}

pub(super) fn loaded() {
    let owner = OWNER.get();
    LOADED.with(|kept| {
        if let Some(loaded) = &kept.borrow()[index(owner)] {
            loaded();
        }
    });
}
