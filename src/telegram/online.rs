//! Whether the account shows as online (TDLib's "online" option): while FinchGram's window is in
//! front and in use, as in Telegram's own apps; it goes offline a few seconds after another app
//! comes to the front, or after half a minute without a key or the mouse.
//!
//! Telegram goes by it beyond what others see: while the account is online here, its other devices
//! hold their notifications back, and TDLib shows FinchGram's at once; while it is not, TDLib waits
//! with a notification as long as another device is in use, in case the message is read there
//! (notifications.rs).

use std::cell::Cell;
use std::time::{Duration, Instant};

use serde_json::json;

use super::{Error, send};

/// Another app in front this long: offline.
const AWAY_AFTER: Duration = Duration::from_secs(5);
/// No key pressed and the mouse not used in the window this long: offline.
const IDLE_AFTER: Duration = Duration::from_secs(30);
/// While online, how often it looks whether it still is.
const CHECK_EVERY: Duration = Duration::from_secs(5);

thread_local! {
    /// TDLib has an account to show as online.
    static READY: Cell<bool> = const { Cell::new(false) };
    static IN_FRONT: Cell<bool> = const { Cell::new(false) };
    /// When the window last stopped being in front.
    static LEFT: Cell<Option<Instant>> = const { Cell::new(None) };
    static LAST_INPUT: Cell<Option<Instant>> = const { Cell::new(None) };
    /// What TDLib was last told; None: nothing since it became ready.
    static TOLD: Cell<Option<bool>> = const { Cell::new(None) };
    /// Runs while online, to go offline in time.
    static CHECK: slint::Timer = slint::Timer::default();
}

/// TDLib has an account (authorizationStateReady), or no longer has one: a log out, or a new start
/// of finchgram-tdlib, which begins offline.
pub fn set_ready(ready: bool) {
    READY.set(ready);
    TOLD.set(None);
    if !ready {
        CHECK.with(slint::Timer::stop);
    }
    update();
}

/// The window came to the front, or another app did (or the window was closed).
pub fn window_in_front(in_front: bool) {
    IN_FRONT.set(in_front);
    if in_front {
        LAST_INPUT.set(Some(Instant::now()));
    } else {
        LEFT.set(Some(Instant::now()));
    }
    update();
}

/// A key or the mouse was used in the window. Called for every move of the mouse, so it only
/// notes the time unless the account is offline.
pub fn input() {
    LAST_INPUT.set(Some(Instant::now()));
    if TOLD.get() != Some(true) {
        update();
    }
}

/// Tell TDLib when the account goes online or offline.
fn update() {
    if !READY.get() {
        return;
    }
    let online = in_use(IN_FRONT.get(), LEFT.get(), LAST_INPUT.get(), Instant::now());
    if TOLD.get() == Some(online) {
        return;
    }
    TOLD.set(Some(online));
    let request = json!({ "@type": "setOption", "name": "online", "value": { "@type": "optionValueBoolean", "value": online } });
    send(request, |answer| match answer {
        Ok(_) | Err(Error::Stopped) => {}
        Err(err) => eprintln!("telegram: cannot change the online status: {err}"),
    });
    CHECK.with(|check| {
        if online {
            check.start(slint::TimerMode::Repeated, CHECK_EVERY, update);
        } else {
            check.stop();
        }
    });
}

/// Whether the window is in use: in front (or until a moment ago), and a key or the mouse used
/// lately.
fn in_use(in_front: bool, left: Option<Instant>, last_input: Option<Instant>, now: Instant) -> bool {
    let in_front = in_front || left.is_some_and(|left| now.duration_since(left) < AWAY_AFTER);
    let used = last_input.is_some_and(|input| now.duration_since(input) < IDLE_AFTER);
    in_front && used
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn online_while_in_front_and_used() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        assert!(in_use(true, None, Some(start), at(10)));
        assert!(!in_use(true, None, Some(start), at(31)), "idle");
        assert!(!in_use(true, None, None, at(1)), "never used");
        assert!(in_use(false, Some(at(10)), Some(at(9)), at(12)), "only just left");
        assert!(!in_use(false, Some(at(10)), Some(at(9)), at(16)), "away");
    }
}
