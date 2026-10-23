//! Slow mode (the design's fifth round): in a group with slow mode, a member who sent a message
//! waits before the next: the composer counts the wait down (supergroupFullInfo.
//! slow_mode_delay_expires_in, and SLOWMODE_WAIT_N when Telegram refuses a message), and
//! administrators are not held. Setting slow mode, and the rest of a group's administration, is
//! the management page's (manage.rs).

use std::cell::RefCell;
use std::time::{Duration, Instant};

use slint::{Timer, TimerMode};

use super::api::ChatType;
use super::store;

thread_local! {
    /// Counts the slow-mode wait down, a second at a time, while there is one.
    static TICK: RefCell<Option<Timer>> = const { RefCell::new(None) };
}

fn open_chat() -> Option<i64> {
    store::with(|store| store.open).flatten()
}

// ---- slow mode ----------------------------------------------------------------------------

/// A chat came to the front, or its full info changed: the wait is what TDLib says.
pub fn slow_mode_changed() {
    let Some(chat_id) = open_chat() else { return };
    store::with(|store| {
        let Some(chat) = store.chats.get(&chat_id) else { return };
        let ChatType::Supergroup { supergroup_id, .. } = chat.kind else { return };
        let (_, limited) = store.slow_mode(chat);
        let expires_in = store.supergroup_full.get(&supergroup_id).map_or(0.0, |info| info.slow_mode_delay_expires_in);
        if limited && expires_in > 0.0 {
            store.slow_until.insert(chat_id, Instant::now() + Duration::from_secs_f64(expires_in));
        } else if !limited {
            store.slow_until.remove(&chat_id);
        }
        store.dirty.header = true;
    });
    tick();
}

/// We sent a message: the wait starts, when slow mode holds us. (TDLib says the same soon after,
/// slow_mode_changed; this is for the composer not to lag.)
pub fn message_sent(chat_id: i64) {
    store::with(|store| {
        let Some(chat) = store.chats.get(&chat_id) else { return };
        let (delay, limited) = store.slow_mode(chat);
        if limited {
            store.slow_until.insert(chat_id, Instant::now() + Duration::from_secs(u64::try_from(delay).unwrap_or(0)));
            store.dirty.header = true;
        }
    });
    tick();
}

/// Telegram refused a message for the wait (SLOWMODE_WAIT_N): the composer counts those seconds.
pub fn wait_refused(chat_id: i64, seconds: i32) {
    store::with(|store| {
        store.slow_until.insert(chat_id, Instant::now() + Duration::from_secs(u64::try_from(seconds).unwrap_or(0)));
        store.dirty.header = true;
    });
    tick();
}

/// Whether slow mode holds us from sending in the chat now.
pub fn blocked(chat_id: i64) -> bool {
    store::with(|store| {
        let chat = store.chats.get(&chat_id)?;
        let (_, limited) = store.slow_mode(chat);
        Some(limited && store.slow_until.get(&chat_id).is_some_and(|until| *until > Instant::now()))
    })
    .flatten()
    .unwrap_or(false)
}

/// The wait shown goes down every second until it is over.
fn tick() {
    let waiting = store::with(|store| store.open.and_then(|id| store.slow_until.get(&id)).is_some_and(|until| *until > Instant::now())).unwrap_or(false);
    store::refresh();
    TICK.with(|tick| {
        let mut tick = tick.borrow_mut();
        if !waiting {
            *tick = None;
            return;
        }
        if tick.is_none() {
            let timer = Timer::default();
            timer.start(TimerMode::Repeated, Duration::from_secs(1), || {
                store::with(|store| store.dirty.header = true);
                let over = store::with(|store| store.open.and_then(|id| store.slow_until.get(&id)).is_none_or(|until| *until <= Instant::now())).unwrap_or(true);
                store::refresh();
                if over {
                    TICK.with(|tick| *tick.borrow_mut() = None);
                }
            });
            *tick = Some(timer);
        }
    });
}

pub fn forget() {
    TICK.with(|tick| *tick.borrow_mut() = None);
}
