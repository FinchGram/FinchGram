//! The pop-out player (the canvas "FinchGram Desktop 直播", round 12: "Pop out · stays on top of
//! other apps"): the stream left playing in a window of its own, above every app, while the user
//! does something else; the main window's Watch closes meanwhile. A window has globals and an
//! OpenGL context of its own, so it has a player of its own (src/player/popout.rs) and is handed
//! what it shows. Its stream is a directory's, by its address; Telegram's play in the Live view.

use std::cell::RefCell;

use slint::ComponentHandle;

use crate::{LiveChannel, LiveDirectoryWords, LivePopOut};

thread_local! {
    /// The window, made at the first pop-out and kept (hidden) for the next.
    static WINDOW: RefCell<Option<LivePopOut>> = const { RefCell::new(None) };
    /// The stream it plays: its address.
    static PLAYING: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn window() -> Option<LivePopOut> {
    WINDOW.with(|kept| {
        if kept.borrow().is_none() {
            let window = LivePopOut::new().map_err(|err| eprintln!("live: cannot make the pop-out window: {err}")).ok()?;
            crate::player::install_popout(&window);
            window.on_toggle_pause(|| {
                with_window(|window| {
                    let paused = !window.get_paused();
                    window.set_paused(paused);
                    crate::player::popout::set_paused(paused);
                });
            });
            window.on_toggle_mute(|| {
                with_window(|window| {
                    let muted = !window.get_muted();
                    window.set_muted(muted);
                    crate::player::popout::set_muted(muted);
                });
            });
            window.on_pop_in(super::watch::pop_in);
            window.on_stop(super::watch::stop);
            window.window().on_close_requested(|| {
                super::watch::stop();
                slint::CloseRequestResponse::HideWindow
            });
            *kept.borrow_mut() = Some(window);
        }
        kept.borrow().as_ref().map(ComponentHandle::clone_strong)
    })
}

fn with_window(change: impl FnOnce(&LivePopOut)) {
    WINDOW.with(|kept| {
        if let Some(window) = kept.borrow().as_ref() {
            change(window);
        }
    });
}

/// Play `address` (the stream of `card`) in the pop-out, shown on top of every app.
pub fn open(address: &str, card: &LiveChannel, muted: bool, words: &LiveDirectoryWords<'_>) {
    let Some(window) = window() else { return };
    window.set_name(card.name.clone());
    let viewers = words.invoke_viewers(card.clone());
    let source = words.invoke_source(card.source);
    window.set_meta(if viewers.is_empty() { source } else { format!("{source} · {viewers}").into() });
    window.set_audio(card.audio);
    window.set_paused(false);
    window.set_muted(muted);
    PLAYING.with(|playing| *playing.borrow_mut() = Some(address.to_string()));
    let (volume, _) = super::sound();
    crate::player::popout::play_url(address, volume, muted);
    if let Err(err) = window.show() {
        eprintln!("live: cannot show the pop-out window: {err}");
    }
}

/// The pop-out closes and its stream stops; its address, to go on with in the main window.
pub fn close() -> Option<String> {
    let address = PLAYING.with(|playing| playing.borrow_mut().take());
    if address.is_some() {
        crate::player::popout::stop();
    }
    with_window(|window| {
        let _ = window.hide();
    });
    address
}

pub fn is_open() -> bool {
    PLAYING.with(|playing| playing.borrow().is_some())
}

/// mpv opened the pop-out's stream.
pub fn loaded() {
    with_window(|window| window.set_buffering(false));
}

/// The pop-out's stream ended, or could not be played: the window goes.
pub fn ended(_failed: bool) {
    if is_open() {
        super::watch::stop();
    }
}
