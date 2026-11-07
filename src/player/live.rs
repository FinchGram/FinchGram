//! The live stream's player (docs/live-streams.md): an mpv of its own, drawn into Live.frame as
//! the media viewer's is into Viewer.frame, reading the stream from a pipe (pipe.rs) that the live
//! side (src/telegram/live.rs) fills a second at a time, each second remuxed (remux.rs).

use std::sync::Arc;

use slint::ComponentHandle;

use super::{Load, Screen, Shown, pipe, with_screen};

/// Play the stream that comes through `pipe`, from its start; `volume` is 0 … 100. A stream
/// played before is given up.
pub fn play(pipe: Arc<pipe::Pipe>, volume: i32, muted: bool) {
    let url = pipe::offer(pipe);
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

/// `volume` is 0 … 100; the mute is a setting of its own.
pub fn set_volume(volume: i32) {
    with_screen(Screen::Live, |player| player.command(&["set", "volume", &volume.to_string()]));
}

pub fn set_muted(muted: bool) {
    with_screen(Screen::Live, |player| player.command(&["set", "mute", if muted { "yes" } else { "no" }]));
}

/// Tell `ended` when mpv has come to the end of the stream (false) or could not play it (true).
pub fn on_ended(ended: impl Fn(bool) + 'static) {
    super::LIVE_ENDED.with(|kept| *kept.borrow_mut() = Some(Box::new(ended)));
}
