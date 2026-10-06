//! The open chat: its tabs (Workbench), its messages, writing, and what the chat menu does (mute,
//! pin, mark as read).
//!
//! TDLib is told which chat is open (openChat, closeChat): supergroups and channels send their
//! updates only while open. The messages of the open chat are marked as seen as they arrive
//! (viewMessages), which is what marks them read, and while the user writes, the chat sees that we
//! are typing (sendChatAction); that is what Telegram's own apps do, and FinchGram does not do
//! otherwise (Telegram's API terms). Channels show Telegram's sponsored messages, marked as seen
//! once shown. The pictures of photos, videos and GIFs are downloaded when a page shows their
//! message.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use serde_json::json;
use slint::ComponentHandle;

use super::api::{self, ChatList, ChatType, Messages, SponsoredMessages, SupergroupFullInfo};
use super::store::{self, History};
use super::{Error, actions, files, send};
use crate::images;
use crate::{Conversation, MainWindow};

thread_local! {
    /// Messages that arrived in the open chat while the window was in the background.
    static UNSEEN: RefCell<Vec<(i64, Vec<i64>)>> = const { RefCell::new(Vec::new()) };
    /// The chat last told that we are typing, and when.
    static TYPING: Cell<Option<(i64, Instant)>> = const { Cell::new(None) };
}

/// The window came to the front: what arrived in the open chat meanwhile is seen now. Messages of
/// a chat that is no longer open stay unread.
pub fn window_came_to_front() {
    let unseen = UNSEEN.with(|unseen| std::mem::take(&mut *unseen.borrow_mut()));
    let open = open_chat();
    for (chat_id, message_ids) in unseen {
        if Some(chat_id) == open {
            view(chat_id, message_ids);
        }
    }
}

/// How many messages one getChatHistory asks for, and how many a chat shows at least when opened.
const PAGE: i32 = 50;
const ENOUGH: usize = 30;
/// Chats kept open in tabs at most; the oldest tab closes to make room.
const TABS: usize = 12;
/// Muting "forever": TDLib takes any time longer than a year as that.
const MUTE_FOREVER: i32 = i32::MAX;
/// Others see that we are typing for a few seconds after they are told (store::TYPING_LASTS);
/// while the user goes on writing, they are told again after this long, as Telegram's own apps do.
const TYPING_SAID_EVERY: Duration = Duration::from_secs(5);

pub fn connect(ui: &MainWindow) {
    let conversation = ui.global::<Conversation>();
    conversation.on_open(|id| {
        if let Ok(id) = id.parse() {
            open(id);
        }
    });
    conversation.on_close(|id| {
        if let Ok(id) = id.parse() {
            close(id);
        }
    });
    conversation.on_close_others(|id| {
        if let Ok(id) = id.parse() {
            close_others(id);
        }
    });
    conversation.on_close_all(close_all);
    conversation.on_send(|text| write(&text));
    conversation.on_edited(|words| edited(&words));
    conversation.on_load_older(load_older);
    conversation.on_retry(|id| {
        if let (Ok(id), Some(chat_id)) = (id.parse::<i64>(), open_chat()) {
            let request = json!({ "@type": "resendMessages", "chat_id": chat_id, "message_ids": [id], "quote": null, "paid_message_star_count": 0 });
            send(request, |answer| log_error("resend a message", answer));
        }
    });
    conversation.on_open_sponsored(|id| {
        if let Ok(id) = id.parse() {
            open_sponsored(id);
        }
    });
    conversation.on_toggle_mute(|| {
        if let Some(chat_id) = open_chat() {
            toggle_mute(chat_id);
        }
    });
    conversation.on_toggle_pin(|| {
        if let Some(chat_id) = open_chat() {
            toggle_pin(chat_id);
        }
    });
    conversation.on_mark_read(|| {
        if let Some(chat_id) = open_chat() {
            mark_read(chat_id);
        }
    });
    conversation.on_open_link(|url| open_link(&url));
    conversation.on_load_picture(|id| {
        if let Ok(id) = id.parse() {
            load_picture(id);
        }
    });
}

/// A message of the open chat with a photo, a video or a GIF is in view: download its picture and
/// decode it, then its row shows it instead of the tiny preview.
fn load_picture(message_id: i64) {
    let file = store::with(|store| {
        let chat_id = store.open?;
        let message = store.histories.get(&chat_id)?.messages.get(&message_id)?;
        store::picture(&message.content).or_else(|| store::sticker_picture(&message.content))?.file.cloned()
    })
    .flatten();
    if let Some(file) = file {
        fetch_picture(file, files::ON_SCREEN);
    }
}

/// Download a picture and decode it into the store, unless it is there already; the rows and the
/// media viewer then show it.
pub fn fetch_picture(file: super::api::File, priority: i32) {
    if store::with(|store| store.pictures.contains(&file.id)).unwrap_or(true) {
        return;
    }
    let id = file.id;
    files::download(&file, priority, move |path| {
        images::load(path, move |picture| {
            let Some(picture) = picture else { return };
            store::with(|store| {
                store.pictures.insert(id, picture);
                store.dirty.conversation = true;
                store.dirty.viewer = true;
            });
            store::refresh();
        });
    });
}

fn open_chat() -> Option<i64> {
    store::with(|store| store.open).flatten()
}

/// Bring `chat_id` to the front: its tab (opened if needed), its messages.
pub fn open(chat_id: i64) {
    let Some((previous, needs_history, kind)) = store::with(|store| {
        let chat = store.chats.get(&chat_id)?;
        let kind = chat.kind;
        let previous = store.open.replace(chat_id);
        if !store.tabs.contains(&chat_id) {
            store.tabs.push(chat_id);
            // Only the front chat is open in TDLib, so the oldest tab just goes.
            if store.tabs.len() > TABS {
                let oldest = store.tabs.remove(0);
                store.histories.remove(&oldest);
                store.sponsored.remove(&oldest);
            }
        }
        let needs_history = !store.histories.contains_key(&chat_id);
        store.dirty.chats = true;
        store.dirty.conversation = true;
        store.dirty.scroll_to_end = true;
        Some((previous, needs_history, kind))
    })
    .flatten() else {
        return;
    };

    if previous != Some(chat_id) {
        actions::chat_changed(Some(chat_id));
        if let Some(previous) = previous {
            send(json!({ "@type": "closeChat", "chat_id": previous }), |_| {});
        }
        send(json!({ "@type": "openChat", "chat_id": chat_id }), |answer| log_error("open a chat", answer));
    }

    if needs_history {
        store::with(|store| {
            store.histories.insert(chat_id, History { has_older: true, ..History::default() });
        });
        load_history(chat_id, 0, None);
    } else {
        view_newest(chat_id);
    }
    if let ChatType::Supergroup { supergroup_id, is_channel } = kind {
        // The member count, which the supergroup itself mostly leaves at 0.
        send(json!({ "@type": "getSupergroupFullInfo", "supergroup_id": supergroup_id }), move |answer| {
            let Ok(Ok(info)) = answer.map(serde_json::from_value::<SupergroupFullInfo>) else { return };
            store::with(|store| {
                store.supergroup_members.insert(supergroup_id, info.member_count);
                store.dirty.chats = true;
            });
            store::refresh();
        });
        if is_channel {
            load_sponsored(chat_id);
        }
    }
    store::refresh();
}

/// Close a tab. When it was the front one, the tab next to it comes to the front.
fn close(chat_id: i64) {
    enum Then {
        Nothing,
        Open(i64),
        ShowNone,
    }
    let then = store::with(|store| {
        let Some(index) = store.tabs.iter().position(|id| *id == chat_id) else { return Then::Nothing };
        store.tabs.remove(index);
        store.histories.remove(&chat_id);
        store.sponsored.remove(&chat_id);
        store.dirty.chats = true;
        if store.open != Some(chat_id) {
            return Then::Nothing;
        }
        store.open = None;
        store.dirty.conversation = true;
        match store.tabs.get(index).or_else(|| store.tabs.last()) {
            Some(next) => Then::Open(*next),
            None => Then::ShowNone,
        }
    })
    .unwrap_or(Then::Nothing);
    if !matches!(then, Then::Nothing) {
        send(json!({ "@type": "closeChat", "chat_id": chat_id }), |_| {});
    }
    match then {
        Then::Open(next) => open(next),
        Then::ShowNone => {
            actions::chat_changed(None);
            store::refresh();
        }
        Then::Nothing => store::refresh(),
    }
}

/// Keep only the tab of `keep`, which comes to the front. The others just go: only the front one is
/// open in TDLib, and bringing `keep` to the front closes it there.
fn close_others(keep: i64) {
    let front = store::with(|store| {
        if !store.tabs.contains(&keep) {
            return None;
        }
        let closed: Vec<i64> = store.tabs.iter().copied().filter(|id| *id != keep).collect();
        store.tabs.retain(|id| *id == keep);
        for id in closed {
            store.histories.remove(&id);
            store.sponsored.remove(&id);
        }
        store.dirty.chats = true;
        Some(store.open)
    })
    .flatten();
    match front {
        Some(Some(front_chat)) if front_chat == keep => store::refresh(),
        Some(_) => open(keep),
        None => {}
    }
}

/// Close every tab: no chat is open.
fn close_all() {
    let Some(front) = store::with(|store| {
        for id in std::mem::take(&mut store.tabs) {
            store.histories.remove(&id);
            store.sponsored.remove(&id);
        }
        store.dirty.chats = true;
        store.dirty.conversation = true;
        store.open.take()
    }) else {
        return;
    };
    if let Some(chat_id) = front {
        send(json!({ "@type": "closeChat", "chat_id": chat_id }), |_| {});
    }
    actions::chat_changed(None);
    store::refresh();
}

/// Ask for messages before `from_message_id` (0: the newest), then `then`. A first page that comes
/// back short (TDLib answers from its database first) asks again until the chat has enough to fill
/// the view.
fn load_history(chat_id: i64, from_message_id: i64, then: Option<Box<dyn FnOnce()>>) {
    let asked = store::with(|store| {
        let history = store.histories.get_mut(&chat_id)?;
        if history.loading {
            return None;
        }
        history.loading = true;
        Some(())
    })
    .flatten();
    if asked.is_none() {
        return;
    }
    store::with(|store| store.dirty.conversation = true);
    let request = json!({
        "@type": "getChatHistory", "chat_id": chat_id, "from_message_id": from_message_id,
        "offset": 0, "limit": PAGE, "only_local": false,
    });
    send(request, move |answer| {
        let messages = match answer.map(serde_json::from_value::<Messages>) {
            Ok(Ok(messages)) => messages.messages.into_iter().flatten().collect::<Vec<_>>(),
            Ok(Err(err)) => {
                eprintln!("telegram: cannot read a chat's history: {err}");
                Vec::new()
            }
            Err(err) => {
                log_error("load a chat's history", Err(err));
                Vec::new()
            }
        };
        let ids: Vec<i64> = messages.iter().map(|message| message.id).collect();
        let next = store::with(|store| {
            let open = store.open == Some(chat_id);
            let history = store.histories.get_mut(&chat_id)?;
            history.loading = false;
            // The answer may hold the message it started from: older ones are what is new.
            let older = messages.iter().filter(|message| from_message_id == 0 || message.id < from_message_id).count();
            history.has_older = older > 0;
            for message in messages {
                history.messages.insert(message.id, message);
            }
            let oldest = history.messages.keys().next().copied();
            let more = from_message_id == 0 && history.has_older && history.messages.len() < ENOUGH;
            store.dirty.conversation = open;
            store.dirty.scroll_to_end = open && from_message_id == 0;
            Some((open, if more { oldest } else { None }))
        })
        .flatten();
        store::refresh();
        if let Some(then) = then {
            then();
        }
        let Some((open, more)) = next else { return };
        if open && from_message_id == 0 && !ids.is_empty() {
            view(chat_id, ids);
        }
        if let Some(oldest) = more {
            load_history(chat_id, oldest, None);
        }
    });
}

/// The view reached the top: older messages.
fn load_older() {
    let Some((chat_id, oldest)) = store::with(|store| {
        let chat_id = store.open?;
        let history = store.histories.get(&chat_id)?;
        if !history.has_older || history.loading {
            return None;
        }
        Some((chat_id, *history.messages.keys().next()?))
    })
    .flatten() else {
        return;
    };
    load_history(chat_id, oldest, None);
}

/// Load the page before the oldest message loaded, then `then` (a reply's original is looked for).
pub fn load_older_then(chat_id: i64, then: impl FnOnce() + 'static) {
    let state = store::with(|store| {
        let history = store.histories.get(&chat_id)?;
        Some((history.loading, history.has_older, *history.messages.keys().next()?))
    })
    .flatten();
    match state {
        // A page is on its way already: after it.
        Some((true, _, _)) => slint::Timer::single_shot(Duration::from_millis(200), move || load_older_then(chat_id, then)),
        Some((false, true, oldest)) => load_history(chat_id, oldest, Some(Box::new(then))),
        _ => {}
    }
}

/// Fetch the messages replies answer that are not in the history, by chat and message, once each;
/// a reply to one that was deleted then says so.
pub fn load_replied(missing: Vec<(i64, i64)>) {
    for key in missing {
        let new = store::with(|store| store.asking_replies.insert(key)).unwrap_or(false);
        if !new {
            continue;
        }
        let (chat_id, message_id) = key;
        send(json!({ "@type": "getMessage", "chat_id": chat_id, "message_id": message_id }), move |answer| {
            let found = match answer {
                Ok(message) => match serde_json::from_value::<api::Message>(message) {
                    Ok(message) => Some(Some(message)),
                    Err(err) => {
                        eprintln!("telegram: cannot read a replied message: {err}");
                        None
                    }
                },
                Err(Error::Telegram { code: 404, .. }) => Some(None),
                Err(Error::Telegram { .. }) => None,
                Err(Error::Stopped) => {
                    // Asked again at the next refresh.
                    store::with(|store| store.asking_replies.remove(&key));
                    None
                }
            };
            if let Some(found) = found {
                store::with(|store| {
                    store.replied.insert(key, found);
                    store.dirty.conversation = store.open == Some(chat_id);
                });
                store::refresh();
            }
        });
    }
}

/// The chat came back to the front: what arrived meanwhile is seen now.
fn view_newest(chat_id: i64) {
    let ids = store::with(|store| {
        store.histories.get(&chat_id).map(|history| history.messages.keys().rev().take(PAGE as usize).copied().collect::<Vec<_>>())
    })
    .flatten()
    .unwrap_or_default();
    if !ids.is_empty() {
        view(chat_id, ids);
    }
}

/// The messages are on the screen of the open chat. While the window is in the background they
/// are not seen yet: they wait until it comes to the front, as in Telegram's own apps.
pub fn view(chat_id: i64, message_ids: Vec<i64>) {
    if !super::window_is_in_front() {
        UNSEEN.with(|unseen| unseen.borrow_mut().push((chat_id, message_ids)));
        return;
    }
    let request = json!({
        "@type": "viewMessages", "chat_id": chat_id, "message_ids": message_ids,
        "source": { "@type": "messageSourceChatHistory" }, "force_read": false,
    });
    send(request, |answer| log_error("mark messages as seen", answer));
}

fn load_sponsored(chat_id: i64) {
    send(json!({ "@type": "getChatSponsoredMessages", "chat_id": chat_id }), move |answer| {
        let Ok(Ok(sponsored)) = answer.map(serde_json::from_value::<SponsoredMessages>) else { return };
        let shown = sponsored.messages.first().map(|message| message.message_id);
        store::with(|store| {
            store.sponsored.insert(chat_id, sponsored.messages);
            store.dirty.conversation = store.open == Some(chat_id);
        });
        store::refresh();
        // The channel opens at its newest post, and the sponsored message is under it, whole.
        if let Some(id) = shown {
            view(chat_id, vec![id]);
        }
    });
}

fn open_sponsored(message_id: i64) {
    let Some((chat_id, url)) = store::with(|store| {
        let chat_id = store.open?;
        let message = store.sponsored.get(&chat_id)?.iter().find(|message| message.message_id == message_id)?;
        Some((chat_id, message.sponsor.url.clone()))
    })
    .flatten() else {
        return;
    };
    let request = json!({
        "@type": "clickChatSponsoredMessage", "chat_id": chat_id, "message_id": message_id,
        "is_media_click": false, "from_fullscreen": false,
    });
    send(request, |answer| log_error("open a sponsored message", answer));
    crate::platform::open_link(&url);
}

/// Send `text` to the open chat, or with the strip above the composer: a reply, an edit, messages
/// to forward after it (actions.rs).
fn write(text: &str) {
    let text = text.trim();
    let Some(chat_id) = open_chat() else { return };
    // The message ends the typing; the next words say it again at once.
    TYPING.set(None);
    if actions::send_with_bar(chat_id, text) || text.is_empty() {
        return;
    }
    send_text(chat_id, text, None);
}

/// What the user writes changed: the open chat sees that we are typing, or that we stopped when
/// the words are gone. Editing a message is no typing.
fn edited(words: &str) {
    let Some(chat_id) = open_chat() else { return };
    if actions::editing(chat_id) {
        return;
    }
    let said = TYPING.get().and_then(|(chat, at)| (chat == chat_id).then(|| at.elapsed()));
    let Some(typing) = typing_now(!words.trim().is_empty(), said) else { return };
    TYPING.set(typing.then(|| (chat_id, Instant::now())));
    let action = if typing { json!({ "@type": "chatActionTyping" }) } else { json!({ "@type": "chatActionCancel" }) };
    let request = json!({ "@type": "sendChatAction", "chat_id": chat_id, "topic_id": null, "business_connection_id": "", "action": action });
    send(request, |answer| log_error("say that we are typing", answer));
}

/// What to tell a chat, `said` being how long ago it was last told that we are typing: that we are
/// (true), that we stopped (false), or nothing.
fn typing_now(has_words: bool, said: Option<Duration>) -> Option<bool> {
    if has_words {
        said.is_none_or(|said| said >= TYPING_SAID_EVERY).then_some(true)
    } else {
        // Only while the others may still see it.
        said.is_some_and(|said| said < store::TYPING_LASTS).then_some(false)
    }
}

/// Send `text` to a chat, as a reply to `reply_to` when given. The message shows itself as TDLib
/// sends it back (updateNewMessage), first as being sent.
pub fn send_text(chat_id: i64, text: &str, reply_to: Option<i64>) {
    let reply_to = reply_to.map(|message_id| {
        json!({ "@type": "inputMessageReplyToMessage", "message_id": message_id, "quote": null, "checklist_task_id": 0, "poll_option_id": "" })
    });
    let request = json!({
        "@type": "sendMessage", "chat_id": chat_id, "topic_id": null, "reply_to": reply_to,
        "options": null, "reply_markup": null,
        "input_message_content": {
            "@type": "inputMessageText",
            "text": { "@type": "formattedText", "text": text, "entities": [] },
            "link_preview_options": null,
            "clear_draft": true,
        },
    });
    send(request, |answer| log_error("send a message", answer));
    store::with(|store| store.dirty.scroll_to_end = true);
}

/// Mute a chat for good, or unmute it.
pub fn toggle_mute(chat_id: i64) {
    let Some(settings) = store::with(|store| {
        let chat = store.chats.get(&chat_id)?;
        let mute_for = if store.muted(chat) { 0 } else { MUTE_FOREVER };
        Some(chat.notification_settings.with_mute_for(mute_for))
    })
    .flatten() else {
        return;
    };
    let request = json!({ "@type": "setChatNotificationSettings", "chat_id": chat_id, "notification_settings": settings });
    send(request, |answer| log_error("mute a chat", answer));
}

/// Unpin a chat wherever it is pinned, in All chats or in folders, or pin it in All chats: a
/// pinned chat is at the top of Workbench's tree whichever list it is pinned in.
pub fn toggle_pin(chat_id: i64) {
    let Some(pinned_in) = store::with(|store| store.chats.get(&chat_id).map(store::pinned_in)).flatten() else {
        return;
    };
    if pinned_in.is_empty() {
        pin(chat_id, ChatList::Main, true);
    }
    for list in pinned_in {
        pin(chat_id, list, false);
    }
}

/// Pin a chat in one list, or unpin it there: the lists of Broadsheet and Terminal, one folder at
/// a time.
pub fn toggle_pin_in(chat_id: i64, list: ChatList) {
    let Some(pinned) = store::with(|store| store.chats.get(&chat_id).map(|chat| store::position(chat, list).is_some_and(|position| position.is_pinned)))
        .flatten()
    else {
        return;
    };
    pin(chat_id, list, !pinned);
}

fn pin(chat_id: i64, list: ChatList, pinned: bool) {
    let request = json!({ "@type": "toggleChatIsPinned", "chat_list": list.to_json(), "chat_id": chat_id, "is_pinned": pinned });
    send(request, |answer| log_error("pin a chat", answer));
}

/// Mark a chat as read: its last message seen, its unread mark and its mentions gone.
pub fn mark_read(chat_id: i64) {
    let Some((last, marked, mentions)) = store::with(|store| {
        let chat = store.chats.get(&chat_id)?;
        Some((chat.last_message.as_ref().map(|message| message.id), chat.is_marked_as_unread, chat.unread_mention_count > 0))
    })
    .flatten() else {
        return;
    };
    if let Some(last) = last {
        let request = json!({
            "@type": "viewMessages", "chat_id": chat_id, "message_ids": [last],
            "source": { "@type": "messageSourceChatList" }, "force_read": true,
        });
        send(request, |answer| log_error("mark a chat as read", answer));
    }
    if marked {
        let request = json!({ "@type": "toggleChatIsMarkedAsUnread", "chat_id": chat_id, "is_marked_as_unread": false });
        send(request, |answer| log_error("mark a chat as read", answer));
    }
    if mentions {
        send(json!({ "@type": "readAllChatMentions", "chat_id": chat_id }), |answer| log_error("read mentions", answer));
    }
}

/// A link in a message or in its preview: a chat FinchGram opens itself (by its username, from an
/// @mention or t.me; a user), else the browser or the mail app.
fn open_link(url: &str) {
    let open_answer = |what: &'static str| {
        move |answer: Result<serde_json::Value, Error>| match answer {
            Ok(chat) => {
                if let Some(chat_id) = chat["id"].as_i64() {
                    open(chat_id);
                }
            }
            Err(err) => log_error(what, Err(err)),
        }
    };
    if let Some(username) = public_username(url) {
        send(json!({ "@type": "searchPublicChat", "username": username }), open_answer("open a chat by its username"));
    } else if let Some(user_id) = url.strip_prefix("tg://user?id=").and_then(|id| id.parse::<i64>().ok()) {
        send(json!({ "@type": "createPrivateChat", "user_id": user_id, "force": false }), open_answer("open a chat with a user"));
    } else {
        crate::platform::open_link(url);
    }
}

/// The username a link names: tg://resolve?domain=name, or t.me/name (a post's link, t.me/name/12,
/// names its channel). Other t.me links (invitations, sticker sets, …) are for the browser.
fn public_username(url: &str) -> Option<&str> {
    let name = if let Some(query) = url.strip_prefix("tg://resolve?domain=") {
        query.split('&').next()?
    } else {
        let path = ["https://t.me/", "http://t.me/", "https://telegram.me/", "http://telegram.me/"]
            .iter()
            .find_map(|site| url.strip_prefix(site))?;
        path.split(['/', '?', '#']).next()?
    };
    const NOT_NAMES: [&str; 12] = ["joinchat", "addstickers", "addemoji", "addlist", "share", "proxy", "socks", "iv", "login", "c", "s", "m"];
    let valid = name.len() >= 4
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !NOT_NAMES.contains(&name.to_ascii_lowercase().as_str());
    valid.then_some(name)
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
    fn typing_is_said_again_while_writing_and_taken_back_when_the_words_are_gone() {
        let seconds = Duration::from_secs;
        assert_eq!(typing_now(true, None), Some(true));
        assert_eq!(typing_now(true, Some(seconds(2))), None);
        assert_eq!(typing_now(true, Some(seconds(5))), Some(true));
        assert_eq!(typing_now(false, Some(seconds(2))), Some(false));
        assert_eq!(typing_now(false, Some(seconds(9))), None, "seen as stopped already");
        assert_eq!(typing_now(false, None), None);
    }

    #[test]
    fn links_that_name_a_chat() {
        assert_eq!(public_username("tg://resolve?domain=finchgram&start=hi"), Some("finchgram"));
        assert_eq!(public_username("https://t.me/finchgram"), Some("finchgram"));
        assert_eq!(public_username("https://t.me/durov/123"), Some("durov"));
        assert_eq!(public_username("https://t.me/joinchat/AbCdEf"), None);
        assert_eq!(public_username("https://t.me/+AbCdEf"), None);
        assert_eq!(public_username("https://example.org/finchgram"), None);
    }
}
