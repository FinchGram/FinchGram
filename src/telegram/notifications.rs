//! New messages outside the window (Settings → Notifications & sounds): the system's notifications
//! and their sound, the Dock icon's bounce, and the unread count on it.
//!
//! TDLib decides what is worth a notification (its notification API, switched on in [`start`]): it
//! knows which chats are muted, one by one or by kind of chat, waits a moment while another of the
//! account's devices is in use, and takes a notification back once its message is read, here or
//! anywhere else. FinchGram shows what TDLib adds and takes back what TDLib takes back; it only
//! leaves out what the user is looking at already, the open chat while the window is in front.
//! Whether each kind of chat notifies is Telegram's setting for the account (By chat type, kept by
//! the store); the rest is FinchGram's own (AppState, saved in settings.toml).

use std::cell::Cell;

use serde_json::json;
use slint::ComponentHandle;

use super::api::{self, ChatList, NotificationSettingsScope, NotificationType};
use super::store::{self, Names};
use super::{Error, send, with_ui};
use crate::platform::{self, Notification};
use crate::{AppState, ChatKind, Fmt, MainWindow, NotificationScope, NotificationScopes, Page, PlatformWords, TelegramState, Viewer};

/// How many chats' notifications TDLib keeps shown at most, its largest number; of each chat it
/// keeps 10 (its default).
const SHOWN_CHATS: i32 = 25;
/// Muting a kind of chat "forever": TDLib takes any time longer than a year as that.
const MUTE_FOREVER: i32 = i32::MAX;

thread_local! {
    /// Unread messages in the main chat list: all of them, and those in chats that are not muted.
    static UNREAD: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
    /// A chat that a click on a notification asked for before TDLib had said what it is (the
    /// click started FinchGram).
    static WANTED: Cell<Option<i64>> = const { Cell::new(None) };
}

pub fn connect(ui: &MainWindow) {
    ui.global::<NotificationScopes>().on_toggle(|scope| {
        toggle_scope(match scope {
            NotificationScope::PrivateChats => NotificationSettingsScope::Private,
            NotificationScope::Groups => NotificationSettingsScope::Group,
            NotificationScope::Channels => NotificationSettingsScope::Channel,
        })
    });
    platform::handle_notification_clicks(clicked);
}

/// The account is ready: TDLib makes notifications from now on (the option is kept in its
/// database), and the system is asked whether FinchGram may show them.
pub fn start() {
    let request = json!({
        "@type": "setOption", "name": "notification_group_count_max",
        "value": { "@type": "optionValueInteger", "value": SHOWN_CHATS.to_string() },
    });
    send(request, |answer| log_error("turn notifications on", answer));
    if read_state(|app| app.get_desktop_notifications()) {
        platform::ask_to_notify();
    }
}

/// The account is gone (a log out): its notifications go, and its unread count.
pub fn forget() {
    UNREAD.set((0, 0));
    WANTED.set(None);
    platform::remove_all_notifications();
    platform::set_badge(0);
}

/// Settings → Notifications & sounds changed: the unread count follows, and the system is asked
/// whether FinchGram may show notifications when they were just turned on (it asks the user only
/// once).
pub fn settings_changed(desktop_turned_on: bool) {
    show_unread_count();
    if desktop_turned_on {
        platform::ask_to_notify();
    }
}

/// TDLib changed the notifications of a chat (one group of them): show those it added, unless they
/// only show again, and take back those it removed.
pub fn group_changed(
    group_id: i32,
    chat_id: i64,
    settings_chat_id: i64,
    sound_id: i64,
    added: Vec<api::Notification>,
    removed: Vec<i32>,
) {
    if !removed.is_empty() {
        let ids: Vec<String> = removed.iter().map(|id| identifier(group_id, *id)).collect();
        platform::remove_notifications(&ids);
    }
    // A group that had made room for others shows again: nothing new in it.
    if settings_chat_id == 0 || added.is_empty() {
        return;
    }
    // The user is looking at the chat: TDLib takes the notification back once its message is seen.
    if super::window_is_in_front() && store::with(|store| store.open) == Some(Some(chat_id)) {
        return;
    }
    let Some((desktop, previews, sounds, flash)) = read_state(|app| {
        Some((app.get_desktop_notifications(), app.get_message_previews(), app.get_notification_sounds(), app.get_flash_taskbar()))
    }) else {
        return;
    };
    if flash {
        platform::flash_icon();
    }
    if !desktop {
        return;
    }
    let audible = sounds && sound_id != 0 && added.iter().any(|notification| !notification.is_silent);
    let mut shown: Vec<Notification> = added
        .iter()
        .filter_map(|notification| {
            let NotificationType::NewMessage { message, show_preview } = &notification.kind else { return None };
            let (title, subtitle, body) = words(message, previews && *show_preview)?;
            Some(Notification { id: identifier(group_id, notification.id), chat_id, title, subtitle, body, sound: false })
        })
        .collect();
    // Several at once sound once.
    if let Some(last) = shown.last_mut() {
        last.sound = audible;
    }
    for notification in &shown {
        platform::show_notification(notification);
    }
}

/// The notifications still shown from before TDLib started: the system takes back the others
/// (their messages were read meanwhile, or the user logged out).
pub fn active(groups: Vec<api::NotificationGroup>) {
    let ids = groups
        .iter()
        .flat_map(|group| group.notifications.iter().map(|notification| identifier(group.id, notification.id)))
        .collect();
    platform::keep_only_notifications(ids);
}

/// The number of unread messages changed. The main chat list's is the one on the Dock icon.
pub fn unread_changed(list: ChatList, unread: i32, unmuted: i32) {
    if list == ChatList::Main {
        UNREAD.set((unread, unmuted));
        show_unread_count();
    }
}

/// After each batch from TDLib: a chat a click on a notification asked for, once it is known.
pub fn open_wanted() {
    if let Some(chat_id) = WANTED.get()
        && can_open(chat_id)
    {
        WANTED.set(None);
        open(chat_id);
    }
}

/// A click on a notification, the window already shown: its chat.
fn clicked(chat_id: i64) {
    if can_open(chat_id) {
        open(chat_id);
    } else {
        WANTED.set(Some(chat_id));
    }
}

fn can_open(chat_id: i64) -> bool {
    read_state(|app| app.get_telegram_state() == TelegramState::Ready)
        && store::with(|store| store.chats.contains_key(&chat_id)).unwrap_or(false)
}

fn open(chat_id: i64) {
    with_ui(|ui| {
        let viewer = ui.global::<Viewer>();
        if viewer.get_open() {
            viewer.invoke_close();
        }
        ui.global::<AppState>().set_page(Page::Chats);
    });
    super::conversation::open(chat_id);
}

fn show_unread_count() {
    let (unread, unmuted) = UNREAD.get();
    platform::set_badge(if read_state(|app| app.get_count_muted_chats()) { unread } else { unmuted });
}

/// What a notification says: the chat's name; who wrote, in a group; and the message, or only that
/// one came.
fn words(message: &api::Message, show_message: bool) -> Option<(String, String, String)> {
    let mut words = None;
    with_ui(|ui| {
        let names = Names::from(ui);
        words = store::with(|store| {
            let chat = store.chats.get(&message.chat_id)?;
            let sender = store.sender_name(&message.sender_id, &names);
            let subtitle = if store.kind(chat) == ChatKind::Group { sender.clone() } else { String::new() };
            let body = if show_message {
                let (content, text, detail) = store.content(message, &names);
                let fmt = ui.global::<Fmt>();
                if store::is_service(content) {
                    fmt.invoke_service(content, sender.into(), detail.into())
                } else {
                    fmt.invoke_body(content, text.into(), detail.into())
                }
            } else {
                ui.global::<PlatformWords>().get_new_message()
            };
            Some((store.title(chat, &names), subtitle, body.to_string()))
        })
        .flatten();
    });
    words
}

/// Switch a kind of chat's notifications off, or on again: Telegram's setting for the account. Its
/// chats that say otherwise keep what they say.
fn toggle_scope(scope: NotificationSettingsScope) {
    let Some(settings) = store::with(|store| {
        let mute_for = if store.scope_muted(scope) { 0 } else { MUTE_FOREVER };
        Some(store.scope_settings.get(&scope)?.with_mute_for(mute_for))
    })
    .flatten() else {
        return;
    };
    let request = json!({ "@type": "setScopeNotificationSettings", "scope": scope.to_json(), "notification_settings": settings });
    send(request, |answer| log_error("change the notifications of a kind of chat", answer));
}

/// The name a notification has in the system: TDLib's notification ids are unique only within
/// their group.
fn identifier(group_id: i32, notification_id: i32) -> String {
    format!("{group_id}.{notification_id}")
}

/// Read the app's state (Settings → Notifications & sounds, how far Telegram is), when the window
/// still exists.
fn read_state<T: Default>(read: impl FnOnce(&AppState) -> T) -> T {
    let mut value = T::default();
    with_ui(|ui| value = read(&ui.global::<AppState>()));
    value
}

fn log_error(what: &str, answer: Result<serde_json::Value, Error>) {
    match answer {
        Ok(_) | Err(Error::Stopped) => {}
        Err(err) => eprintln!("telegram: cannot {what}: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_message_notification_and_one_that_only_shows_again() {
        let update: api::Update = serde_json::from_str(
            r#"{"@type":"updateNotificationGroup","notification_group_id":7,"type":{"@type":"notificationGroupTypeMessages"},"chat_id":1048576,"notification_settings_chat_id":1048576,"notification_sound_id":"-1","total_count":1,"added_notifications":[{"@type":"notification","id":42,"date":1791387600,"is_silent":false,"type":{"@type":"notificationTypeNewMessage","message":{"@type":"message","id":3145728,"sender_id":{"@type":"messageSenderUser","user_id":1048576},"chat_id":1048576,"is_outgoing":false,"date":1791387600,"edit_date":0,"content":{"@type":"messageText","text":{"@type":"formattedText","text":"see you at nine","entities":[]}}},"show_preview":true}},{"@type":"notification","id":43,"date":1791387601,"is_silent":true,"type":{"@type":"notificationTypeNewSecretChat"}}],"removed_notification_ids":[40,41]}"#,
        )
        .unwrap();
        let api::Update::NotificationGroup {
            notification_group_id,
            chat_id,
            notification_settings_chat_id,
            notification_sound_id,
            added_notifications,
            removed_notification_ids,
        } = update
        else {
            panic!("not a notification group");
        };
        assert_eq!((notification_group_id, chat_id, notification_settings_chat_id), (7, 1_048_576, 1_048_576));
        assert_eq!(notification_sound_id, -1);
        assert_eq!(removed_notification_ids, vec![40, 41]);
        let NotificationType::NewMessage { message, show_preview } = &added_notifications[0].kind else {
            panic!("not a message");
        };
        assert!(*show_preview && message.id == 3_145_728 && !added_notifications[0].is_silent);
        assert!(matches!(added_notifications[1].kind, NotificationType::Other) && added_notifications[1].is_silent);
        assert_eq!(identifier(notification_group_id, added_notifications[0].id), "7.42");

        let update: api::Update = serde_json::from_str(
            r#"{"@type":"updateActiveNotifications","groups":[{"@type":"notificationGroup","id":7,"type":{"@type":"notificationGroupTypeMentions"},"chat_id":1048576,"total_count":2,"notifications":[{"@type":"notification","id":42,"date":1791387600,"is_silent":false,"type":{"@type":"notificationTypeNewCall","call_id":5}}]}]}"#,
        )
        .unwrap();
        let api::Update::ActiveNotifications { groups } = update else { panic!("not the active notifications") };
        assert_eq!((groups[0].id, groups[0].notifications[0].id), (7, 42));

        let update: api::Update = serde_json::from_str(
            r#"{"@type":"updateUnreadMessageCount","chat_list":{"@type":"chatListMain"},"unread_count":120,"unread_unmuted_count":7}"#,
        )
        .unwrap();
        assert!(matches!(
            update,
            api::Update::UnreadMessageCount { chat_list: ChatList::Main, unread_count: 120, unread_unmuted_count: 7 }
        ));
    }

    #[test]
    fn a_kind_of_chat_is_switched_off_with_its_other_settings_unchanged() {
        let update: api::Update = serde_json::from_str(
            r#"{"@type":"updateScopeNotificationSettings","scope":{"@type":"notificationSettingsScopeChannelChats"},"notification_settings":{"@type":"scopeNotificationSettings","mute_for":0,"sound_id":"5037419087648243712","show_preview":true,"use_default_mute_stories":true,"mute_stories":false,"story_sound_id":"-1","show_story_poster":true,"disable_pinned_message_notifications":false,"disable_mention_notifications":true}}"#,
        )
        .unwrap();
        let api::Update::ScopeNotificationSettings { scope, notification_settings } = update else { panic!("not scope settings") };
        assert_eq!(scope, NotificationSettingsScope::Channel);
        assert_eq!(scope.to_json(), json!({ "@type": "notificationSettingsScopeChannelChats" }));
        let muted = notification_settings.with_mute_for(MUTE_FOREVER);
        assert_eq!(muted["mute_for"], json!(i32::MAX));
        assert_eq!(muted["sound_id"], json!("5037419087648243712"));
        assert_eq!(muted["story_sound_id"], json!("-1"));
        assert_eq!((muted["show_preview"].clone(), muted["disable_mention_notifications"].clone()), (json!(true), json!(true)));
    }
}
