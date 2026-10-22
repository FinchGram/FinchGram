//! Drafts: words left in a chat's composer stay with that chat, and Telegram keeps them for the
//! user's other devices (setChatDraftMessage; updateChatDraftMessage brings what was written
//! elsewhere). The composer is one (Conversation.draft): when another chat comes to the front,
//! what it holds is saved for the chat that leaves and the chat that comes puts its own back.
//! Writing saves after a short pause too, so that nothing is lost when the app quits. The chat
//! list shows a chat's draft in place of its last message.

use std::cell::RefCell;
use std::time::Duration;

use serde_json::json;
use slint::{ComponentHandle, Timer, TimerMode};

use super::{actions, send, store, topics, with_ui};
use crate::Conversation;

/// Writing pauses this long before the draft goes to Telegram.
const PAUSE: Duration = Duration::from_millis(1500);

thread_local! {
    /// The save waiting for the pause to end, for which chat.
    static SAVE: RefCell<Option<(i64, Timer)>> = const { RefCell::new(None) };
}

/// The words of the composer as the page has them.
fn composer_words() -> String {
    let mut words = String::new();
    with_ui(|ui| words = ui.global::<Conversation>().get_draft().to_string());
    words
}

/// The composer's words changed in the chat in front: kept for it, and saved after a pause.
pub fn changed(chat_id: i64, words: &str) {
    // While a message is being edited, the composer holds its words, not a draft.
    if actions::editing(chat_id) {
        return;
    }
    // The chat list shows the draft once it is saved (save_now), not at every keystroke.
    store::with(|store| {
        store.drafts.insert(chat_id, words.to_string());
    });
    SAVE.with(|save| {
        let timer = Timer::default();
        timer.start(TimerMode::SingleShot, PAUSE, move || save_now(chat_id));
        *save.borrow_mut() = Some((chat_id, timer));
    });
}

/// The chat leaves the front (another opens, or it closes): what the composer holds is saved
/// at once.
pub fn leaving(chat_id: i64) {
    let pending = SAVE.with(|save| save.borrow_mut().take().map(|(id, _)| id));
    if pending == Some(chat_id) || store::with(|store| store.drafts.contains_key(&chat_id)).unwrap_or(false) {
        save_now(chat_id);
    }
}

/// A chat comes to the front: its draft goes into the composer (unless an edit holds it).
pub fn entering(chat_id: i64) {
    if actions::editing(chat_id) {
        return;
    }
    let words = store::with(|store| store.chats.get(&chat_id).map(|chat| store.draft_of(chat))).flatten().unwrap_or_default();
    actions::set_draft(&words);
}

/// Telegram has a draft for the chat in front from another device: the composer takes it, unless
/// the user is writing something else here.
pub fn draft_changed(chat_id: i64) {
    if actions::editing(chat_id) {
        return;
    }
    let words = store::with(|store| store.chats.get(&chat_id).map(|chat| store.draft_of(chat))).flatten().unwrap_or_default();
    let typing = SAVE.with(|save| save.borrow().as_ref().is_some_and(|(id, _)| *id == chat_id));
    if !typing && composer_words() != words {
        actions::set_draft(&words);
    }
}

/// The draft goes to Telegram: what is kept here, else what the composer holds when the chat is
/// in front. Nothing is sent when Telegram already has the same.
fn save_now(chat_id: i64) {
    SAVE.with(|save| {
        if save.borrow().as_ref().is_some_and(|(id, _)| *id == chat_id) {
            *save.borrow_mut() = None;
        }
    });
    let (words, same) = store::with(|store| {
        let words = store.drafts.get(&chat_id).cloned().unwrap_or_else(|| if store.open == Some(chat_id) { composer_words() } else { String::new() });
        let known = store.chats.get(&chat_id).and_then(|chat| chat.draft_message.as_ref()).map(|draft| draft.words().to_string()).unwrap_or_default();
        let same = known == words.trim_end();
        (words, same)
    })
    .unwrap_or_default();
    store::with(|store| store.dirty.chats = true);
    if same {
        store::with(|store| store.drafts.remove(&chat_id));
        store::refresh();
        return;
    }
    store::refresh();
    let words = words.trim_end().to_string();
    let draft = if words.is_empty() {
        serde_json::Value::Null
    } else {
        json!({
            "@type": "draftMessage", "reply_to": null, "date": 0, "effect_id": 0, "suggested_post_info": null,
            "content": {
                "@type": "draftMessageContentText",
                "text": { "@type": "formattedText", "text": words, "entities": [] },
                "link_preview_options": null,
            },
        })
    };
    let request = json!({ "@type": "setChatDraftMessage", "chat_id": chat_id, "topic_id": topics::topic_json(chat_id), "draft_message": draft });
    send(request, move |answer| {
        if let Err(err) = answer {
            eprintln!("telegram: cannot save a draft: {err}");
        }
    });
}

/// The account is logging out, or the chat's message went: nothing waits.
pub fn forget() {
    SAVE.with(|save| *save.borrow_mut() = None);
}

/// A message was sent in the chat: its draft is gone (Telegram clears it with the message).
pub fn sent(chat_id: i64) {
    SAVE.with(|save| {
        if save.borrow().as_ref().is_some_and(|(id, _)| *id == chat_id) {
            *save.borrow_mut() = None;
        }
    });
    store::with(|store| {
        store.drafts.remove(&chat_id);
        store.dirty.chats = true;
    });
}
