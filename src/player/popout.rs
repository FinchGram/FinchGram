//! The pop-out's player (src/live/popout.rs): the Live view's stream of the directory, played in a
//! window of its own on top of every app, by its address, over the network the Live screens alone
//! may reach (mod.rs), its certificates checked against the Keychain's roots.

use super::{Load, Screen, Shown, with_screen};

/// Play a stream by its address, from the live edge; `volume` is 0 … 100.
pub fn play_url(url: &str, volume: i32, muted: bool) {
    with_screen(Screen::Popout, |player| {
        if player.mpv.is_null()
            && let Err(err) = player.start()
        {
            eprintln!("pop-out player: cannot start mpv: {err}");
            return;
        }
        let load = Load { path: url.to_string(), looping: false, speed: 1.0, volume, muted };
        player.tell(Shown::Loading(true));
        if player.render.is_null() {
            // The renderer is made when the pop-out first renders; the stream loads then.
            player.waiting = Some(load);
            player.redraw();
        } else {
            player.load(load);
        }
    });
}

/// Nothing plays in the pop-out.
pub fn stop() {
    with_screen(Screen::Popout, |player| {
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
        player.tell(Shown::Loading(false));
    });
}

pub fn set_paused(paused: bool) {
    with_screen(Screen::Popout, |player| {
        player.command(&["set", "pause", if paused { "yes" } else { "no" }]);
        player.paused = paused;
    });
}

pub fn set_muted(muted: bool) {
    with_screen(Screen::Popout, |player| player.command(&["set", "mute", if muted { "yes" } else { "no" }]));
}
