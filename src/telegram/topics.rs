//! Topics of a group with topics (a forum supergroup; the design's fifth round). Such a group
//! opens on the list of its topics (getForumTopics): each with its icon, name, last message,
//! unread count, pinned or closed. A topic opened shows its messages alone (getForumTopicHistory,
//! conversation.rs with History::topic_id) with the way back to the list in the header; what we
//! send there goes with the topic (messageTopicForum). A closed topic takes no messages: the pages
//! say so in place of the composer. A new topic takes a name and one of Telegram's icons
//! (getForumTopicDefaultIcons, createForumTopic). TDLib tells of changes (updateForumTopicInfo,
//! updateForumTopic); a changed topic's row is fetched whole (getForumTopic), as its unread
//! count is not in the updates.

use serde_json::json;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::api::{self, ForumTopic, ForumTopicInfo, ForumTopics};
use super::store::{self, History};
use super::{Error, conversation, errors, send, with_ui};
use crate::{Conversation, MainWindow};

/// How many topics are asked for: enough for any group one reads.
const TOPICS: i32 = 100;
/// Telegram's colours for topics' icons, as it gives them (0xRRGGBB).
const COLOURS: [i32; 6] = [0x6FB9F0, 0xFFD67E, 0xCB86DB, 0x8EEE98, 0xFF93B2, 0xFB6F5F];

pub fn connect(ui: &MainWindow) {
    let conversation = ui.global::<Conversation>();
    conversation.on_open_topic(|id| {
        if let Ok(topic_id) = id.parse() {
            open_topic(topic_id);
        }
    });
    conversation.on_back_to_topics(back_to_topics);
    conversation.on_start_new_topic(start_new);
    conversation.on_cancel_new_topic(cancel_new);
    conversation.on_create_topic(create);
}

fn open_chat() -> Option<i64> {
    store::with(|store| store.open).flatten()
}

fn with_conversation(change: impl FnOnce(&Conversation)) {
    with_ui(|ui| change(&ui.global::<Conversation>()));
}

/// A chat came to the front. A group with topics opens on its list: true, and conversation.rs
/// loads no messages; its topics are fetched (anew each time: they are few, and their unread
/// counts move). The topic open, if any, was the last chat's: none is open in this one, whatever
/// it is. Left set, it went with every message and draft of a chat without topics
/// ([`topic_json`]), and TDLib refused them all with "Chat is not a forum".
pub fn chat_opened(chat_id: i64) -> bool {
    let forum = store::with(|store| {
        store.open_topic = None;
        let forum = store.chats.get(&chat_id).is_some_and(|chat| store.is_forum(chat));
        if forum {
            store.histories.remove(&chat_id);
        }
        forum
    })
    .unwrap_or(false);
    if forum {
        load_topics(chat_id);
        load_icons();
    }
    forum
}

/// The group's topics, as TDLib orders them: pinned first, then by their last message.
fn load_topics(chat_id: i64) {
    let asked = store::with(|store| {
        if !store.topics_loading.insert(chat_id) {
            return false;
        }
        store.dirty.header = true;
        true
    })
    .unwrap_or(false);
    if !asked {
        return;
    }
    let request = json!({
        "@type": "getForumTopics", "chat_id": chat_id, "query": "", "offset_date": 0, "offset_message_id": 0,
        "offset_forum_topic_id": 0, "limit": TOPICS,
    });
    send(request, move |answer| {
        let topics = match answer.map(serde_json::from_value::<ForumTopics>) {
            Ok(Ok(topics)) => Some(topics.topics),
            Ok(Err(err)) => {
                eprintln!("telegram: cannot read a group's topics: {err}");
                None
            }
            Err(Error::Stopped) => return,
            Err(err) => {
                eprintln!("telegram: cannot ask for a group's topics: {err}");
                None
            }
        };
        store::with(|store| {
            store.topics_loading.remove(&chat_id);
            if let Some(mut topics) = topics {
                sort(&mut topics);
                store.topics.insert(chat_id, topics);
            }
            store.dirty.header = true;
            store.dirty.topics = true;
        });
        store::refresh();
    });
}

/// Pinned topics first, then the newest activity first, as Telegram's apps list them.
fn sort(topics: &mut [ForumTopic]) {
    topics.sort_by(|a, b| b.is_pinned.cmp(&a.is_pinned).then(b.order.cmp(&a.order)).then(b.info.forum_topic_id.cmp(&a.info.forum_topic_id)));
}

/// A topic changed, or has a new message (store.rs): its row is fetched whole.
pub fn topic_changed(chat_id: i64, topic_id: i32) {
    send(json!({ "@type": "getForumTopic", "chat_id": chat_id, "forum_topic_id": topic_id }), move |answer| {
        let topic = match answer.map(serde_json::from_value::<ForumTopic>) {
            Ok(Ok(topic)) => Some(topic),
            Err(Error::Telegram { code: 404, .. }) => None,
            _ => return,
        };
        store::with(|store| {
            let Some(topics) = store.topics.get_mut(&chat_id) else { return };
            topics.retain(|known| known.info.forum_topic_id != topic_id);
            if let Some(topic) = topic {
                topics.push(topic);
                sort(topics);
            }
            if store.open == Some(chat_id) {
                store.dirty.topics = true;
                store.dirty.header = true;
            }
        });
        store::refresh();
    });
}

/// A topic chosen from the list: its messages take the pane.
fn open_topic(topic_id: i32) {
    let Some(chat_id) = open_chat() else { return };
    store::with(|store| {
        store.open_topic = Some(topic_id);
        store.histories.insert(chat_id, History { topic_id: Some(topic_id), has_older: true, ..History::default() });
        store.dirty.conversation = true;
        store.dirty.header = true;
        store.dirty.scroll_to_end = true;
    });
    with_conversation(|conversation| conversation.set_new_topic_open(false));
    conversation::load_history_from(chat_id, 0);
    store::refresh();
}

/// "All topics": the list again.
fn back_to_topics() {
    let Some(chat_id) = open_chat() else { return };
    store::with(|store| {
        store.open_topic = None;
        store.histories.remove(&chat_id);
        store.dirty.conversation = true;
        store.dirty.header = true;
    });
    load_topics(chat_id);
    store::refresh();
}

/// The topic open in a chat, as a sendMessage's topic; null elsewhere.
pub fn topic_json(chat_id: i64) -> serde_json::Value {
    match store::with(|store| store.open_topic.filter(|_| store.open == Some(chat_id))).flatten() {
        Some(topic_id) => json!({ "@type": "messageTopicForum", "forum_topic_id": topic_id }),
        None => serde_json::Value::Null,
    }
}

// ---- a new topic --------------------------------------------------------------------------

/// The icons a new topic may take: Telegram's set of them, each a custom emoji; asked once.
fn load_icons() {
    let known = store::with(|store| !store.topic_icons.is_empty()).unwrap_or(true);
    if known {
        set_icons();
        return;
    }
    send(json!({ "@type": "getForumTopicDefaultIcons" }), |answer| {
        let Ok(Ok(stickers)) = answer.map(serde_json::from_value::<api::Stickers>) else { return };
        store::with(|store| {
            store.topic_icons = stickers.stickers.into_iter().filter(|sticker| !sticker.emoji.is_empty()).map(|sticker| (sticker.id, sticker.emoji)).collect();
        });
        set_icons();
    });
}

fn set_icons() {
    let icons: Vec<SharedString> = store::with(|store| store.topic_icons.iter().map(|(_, emoji)| emoji.as_str().into()).collect()).unwrap_or_default();
    with_conversation(|conversation| conversation.set_topic_icons(ModelRc::new(VecModel::from(icons))));
}

fn start_new() {
    with_conversation(|conversation| {
        conversation.set_new_topic_name(SharedString::new());
        conversation.set_new_topic_icon(0);
        conversation.set_new_topic_error(SharedString::new());
        conversation.set_new_topic_busy(false);
        conversation.set_new_topic_open(true);
    });
}

fn cancel_new() {
    with_conversation(|conversation| conversation.set_new_topic_open(false));
}

/// Create the topic with the name typed and the icon chosen (createForumTopic), and open it.
fn create() {
    let Some(chat_id) = open_chat() else { return };
    let (mut name, mut icon) = (String::new(), 0);
    with_conversation(|conversation| {
        name = conversation.get_new_topic_name().trim().to_string();
        icon = conversation.get_new_topic_icon();
    });
    if name.is_empty() {
        return;
    }
    let custom_emoji_id = store::with(|store| usize::try_from(icon).ok().and_then(|index| store.topic_icons.get(index)).map(|(id, _)| *id)).flatten().unwrap_or(0);
    // Telegram gives a topic one of six colours; a name's letters pick one here as its apps do.
    let colour = COLOURS[name.bytes().map(usize::from).sum::<usize>() % COLOURS.len()];
    with_conversation(|conversation| {
        conversation.set_new_topic_busy(true);
        conversation.set_new_topic_error(SharedString::new());
    });
    let request = json!({
        "@type": "createForumTopic", "chat_id": chat_id, "name": name, "is_name_implicit": false,
        "icon": { "@type": "forumTopicIcon", "color": colour, "custom_emoji_id": custom_emoji_id.to_string() },
    });
    send(request, move |answer| {
        with_conversation(|conversation| conversation.set_new_topic_busy(false));
        match answer.map(serde_json::from_value::<ForumTopicInfo>) {
            Ok(Ok(info)) => {
                with_conversation(|conversation| conversation.set_new_topic_open(false));
                topic_changed(chat_id, info.forum_topic_id);
                if open_chat() == Some(chat_id) {
                    open_topic(info.forum_topic_id);
                }
            }
            Ok(Err(err)) => eprintln!("telegram: cannot read the new topic: {err}"),
            Err(Error::Stopped) => {}
            Err(Error::Telegram { message, .. }) => {
                with_conversation(|conversation| conversation.set_new_topic_error(errors::explain(&message).into()));
            }
        }
    });
}

/// Esc: the new topic's form closes if it is open.
pub(super) fn escape() -> bool {
    let mut open = false;
    with_conversation(|conversation| open = conversation.get_new_topic_open());
    if open {
        cancel_new();
    }
    open
}

pub fn forget() {
    with_conversation(|conversation| {
        conversation.set_new_topic_open(false);
        conversation.set_topics(ModelRc::default());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topic(id: i32, order: i64, pinned: bool) -> ForumTopic {
        serde_json::from_value(json!({
            "info": { "chat_id": -1, "forum_topic_id": id, "name": format!("t{id}"), "icon": { "color": 0, "custom_emoji_id": "0" } },
            "order": order.to_string(), "is_pinned": pinned, "unread_count": 0,
        }))
        .unwrap()
    }

    #[test]
    fn pinned_topics_come_first_then_the_newest() {
        let mut topics = vec![topic(1, 10, false), topic(2, 30, false), topic(3, 20, true)];
        sort(&mut topics);
        let ids: Vec<i32> = topics.iter().map(|topic| topic.info.forum_topic_id).collect();
        assert_eq!(ids, [3, 2, 1]);
    }

    /// A topic was open in a group with topics; then a plain group came to the front. What is
    /// sent there names no topic: TDLib refuses a topic in a chat without them ("Chat is not a
    /// forum"), and it once did, for every message and draft, until the user opened a forum again.
    #[test]
    fn a_topic_open_does_not_follow_into_a_chat_without_topics() {
        const FORUM: i64 = -1001;
        const GROUP: i64 = -2;
        let group: api::Chat = serde_json::from_value(json!({
            "@type": "chat", "id": GROUP, "type": { "@type": "chatTypeBasicGroup", "basic_group_id": 2 },
            "title": "a plain group", "permissions": { "@type": "chatPermissions", "can_send_basic_messages": true },
            "positions": [], "is_marked_as_unread": false, "unread_count": 0, "last_read_inbox_message_id": 0,
            "last_read_outbox_message_id": 0, "unread_mention_count": 0,
            "notification_settings": { "@type": "chatNotificationSettings", "use_default_mute_for": true, "mute_for": 0 }
        }))
        .expect("a chat");
        let slot = crate::telegram::context();
        store::add(slot);
        store::with(|store| {
            store.chats.insert(GROUP, group);
            store.open = Some(FORUM);
            store.open_topic = Some(5);
        });
        assert_eq!(topic_json(GROUP), serde_json::Value::Null, "the topic is the forum's, not the group's");

        // conversation::open puts the chat in front, then tells topics.rs.
        store::with(|store| store.open = Some(GROUP));
        assert!(!chat_opened(GROUP));
        assert_eq!(store::with(|store| store.open_topic), Some(None));
        assert_eq!(topic_json(GROUP), serde_json::Value::Null);
        store::remove(slot);
    }
}
