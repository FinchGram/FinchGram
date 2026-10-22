//! Slow mode and the group's administration (the design's fifth round, GroupAdmin). In a group
//! with slow mode, a member who sent a message waits before the next: the composer counts the
//! wait down (supergroupFullInfo.slow_mode_delay_expires_in, and SLOWMODE_WAIT_N when Telegram
//! refuses a message), and administrators are not held. The chat's settings panel (the header's
//! sidebar icon) shows a member the group's rules, and an owner or administrator the controls:
//! slow mode (setChatSlowModeDelay), what members may do (setChatPermissions), joining by
//! request (toggleSupergroupJoinByRequest), protected content (toggleChatHasProtectedContent),
//! history for new members (toggleSupergroupIsAllHistoryAvailable), hidden members
//! (toggleSupergroupHasHiddenMembers, 100 members or more) and aggressive anti-spam
//! (toggleSupergroupHasAggressiveAntiSpamEnabled, 200 or more). What Telegram refuses
//! (CHAT_ADMIN_REQUIRED, …) is shown in the panel.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use serde_json::json;
use slint::{ComponentHandle, Timer, TimerMode};

use super::api::ChatType;
use super::{Error, errors, send, store, with_ui};
use crate::{Conversation, MainWindow};

thread_local! {
    /// Counts the slow-mode wait down, a second at a time, while there is one.
    static TICK: RefCell<Option<Timer>> = const { RefCell::new(None) };
}

pub fn connect(ui: &MainWindow) {
    let conversation = ui.global::<Conversation>();
    conversation.on_toggle_settings(|| {
        with_ui(|ui| {
            let conversation = ui.global::<Conversation>();
            conversation.set_settings_open(!conversation.get_settings_open());
        });
    });
    conversation.on_set_slow_mode(set_slow_mode);
    conversation.on_toggle_permission(|name| toggle_permission(&name));
    conversation.on_toggle_option(|name| toggle_option(&name));
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

// ---- the settings panel -------------------------------------------------------------------

fn busy(name: &str) {
    store::with(|store| {
        store.admin_busy = name.to_string();
        store.admin_error.clear();
        store.dirty.header = true;
    });
    store::refresh();
}

/// Telegram answered a change: the control is free again; a refusal is shown.
fn answered(answer: Result<serde_json::Value, Error>) {
    let error = match answer {
        Ok(_) | Err(Error::Stopped) => String::new(),
        Err(Error::Telegram { message, .. }) => errors::explain(&message),
    };
    store::with(|store| {
        store.admin_busy.clear();
        store.admin_error = error;
        store.dirty.header = true;
    });
    store::refresh();
}

fn set_slow_mode(seconds: i32) {
    let Some(chat_id) = open_chat() else { return };
    busy("slow");
    send(json!({ "@type": "setChatSlowModeDelay", "chat_id": chat_id, "slow_mode_delay": seconds }), answered);
}

/// One of what members may do, turned the other way (the group's default permissions).
fn toggle_permission(name: &str) {
    let Some(chat_id) = open_chat() else { return };
    let Some(permissions) = store::with(|store| {
        let mut permissions = store.chats.get(&chat_id)?.permissions.clone();
        match name {
            "send" => permissions.can_send_basic_messages = !permissions.can_send_basic_messages,
            "media" => {
                let on = !(permissions.can_send_photos || permissions.can_send_videos);
                permissions.can_send_photos = on;
                permissions.can_send_videos = on;
                permissions.can_send_audios = on;
                permissions.can_send_documents = on;
                permissions.can_send_video_notes = on;
                permissions.can_send_voice_notes = on;
            }
            "stickers" => permissions.can_send_other_messages = !permissions.can_send_other_messages,
            "links" => permissions.can_add_link_previews = !permissions.can_add_link_previews,
            "polls" => permissions.can_send_polls = !permissions.can_send_polls,
            "invite" => permissions.can_invite_users = !permissions.can_invite_users,
            "pin" => permissions.can_pin_messages = !permissions.can_pin_messages,
            "info" => permissions.can_change_info = !permissions.can_change_info,
            _ => return None,
        }
        Some(permissions)
    })
    .flatten() else {
        return;
    };
    busy(name);
    send(json!({ "@type": "setChatPermissions", "chat_id": chat_id, "permissions": permissions.to_json() }), answered);
}

/// One of the group's options, turned the other way.
fn toggle_option(name: &str) {
    let Some(chat_id) = open_chat() else { return };
    let Some(request) = store::with(|store| {
        let chat = store.chats.get(&chat_id)?;
        let ChatType::Supergroup { supergroup_id, .. } = chat.kind else { return None };
        let group = store.supergroups.get(&supergroup_id)?;
        let full = store.supergroup_full.get(&supergroup_id);
        Some(match name {
            "join" => json!({
                "@type": "toggleSupergroupJoinByRequest", "supergroup_id": supergroup_id, "join_by_request": !group.join_by_request,
                "guard_bot_user_id": 0, "apply_to_invite_links": false,
            }),
            "protect" => json!({ "@type": "toggleChatHasProtectedContent", "chat_id": chat_id, "has_protected_content": !chat.has_protected_content }),
            "history" => json!({
                "@type": "toggleSupergroupIsAllHistoryAvailable", "supergroup_id": supergroup_id,
                "is_all_history_available": !full.is_none_or(|info| info.is_all_history_available),
            }),
            "hide-members" => json!({
                "@type": "toggleSupergroupHasHiddenMembers", "supergroup_id": supergroup_id,
                "has_hidden_members": !full.is_some_and(|info| info.has_hidden_members),
            }),
            "anti-spam" => json!({
                "@type": "toggleSupergroupHasAggressiveAntiSpamEnabled", "supergroup_id": supergroup_id,
                "has_aggressive_anti_spam_enabled": !full.is_some_and(|info| info.has_aggressive_anti_spam_enabled),
            }),
            _ => return None,
        })
    })
    .flatten() else {
        return;
    };
    busy(name);
    send(request, answered);
}

pub fn forget() {
    TICK.with(|tick| *tick.borrow_mut() = None);
    with_ui(|ui| ui.global::<Conversation>().set_settings_open(false));
}
