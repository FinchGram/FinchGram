//! Reactions to messages (the design's fifth round): the chips under a message with how many
//! gave each emoji, the smiley by a message that opens the picker, and a double click that gives
//! the default reaction. The picker offers what the chat allows, in Telegram's order for the
//! message (getMessageAvailableReactions); one gives a reaction with addMessageReaction and takes
//! it back with removeMessageReaction, and TDLib keeps to how many one person may give here
//! (chatAvailableReactions.max_reaction_count: one, three with Premium, fewer where the chat says
//! so). Custom emoji reactions are drawn as the emoji they stand for, asked from TDLib once
//! (getCustomEmojiStickers).

use serde_json::json;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::api::{self, AvailableReactions, ReactionType};
use super::{Error, actions, errors, send, store, with_ui};
use crate::{Conversation, MainWindow};

/// The picker's choices when Telegram names none: its own first eight.
const DEFAULT_CHOICES: [&str; 8] = ["👍", "❤️", "🔥", "🎉", "😂", "😮", "😢", "🙏"];
/// How many the picker shows.
const CHOICES: usize = 8;

pub fn connect(ui: &MainWindow) {
    let conversation = ui.global::<Conversation>();
    conversation.on_open_reactions(|id| {
        if let Ok(message_id) = id.parse() {
            open_picker(message_id);
        }
    });
    conversation.on_close_reactions(close_picker);
    conversation.on_react(|id, emoji| {
        if let Ok(message_id) = id.parse() {
            react(message_id, ReactionType::Emoji { emoji: emoji.to_string() });
        }
    });
    conversation.on_quick_react(|id| {
        if let Ok(message_id) = id.parse() {
            quick_react(message_id);
        }
    });
}

fn open_chat() -> Option<i64> {
    store::with(|store| store.open).flatten()
}

fn with_conversation(change: impl FnOnce(&Conversation)) {
    with_ui(|ui| change(&ui.global::<Conversation>()));
}

/// The picker opens for a message with what the chat allows; Telegram's order for this message
/// comes when it answers.
fn open_picker(message_id: i64) {
    let Some(chat_id) = open_chat() else { return };
    let allowed = store::with(|store| {
        let chat = store.chats.get(&chat_id)?;
        match &chat.available_reactions {
            Some(api::ChatAvailableReactions::Some { reactions, .. }) => {
                Some(reactions.iter().filter_map(|kind| emoji_of(store, kind)).take(CHOICES).collect::<Vec<_>>())
            }
            _ => None,
        }
    })
    .flatten();
    let first: Vec<SharedString> = match &allowed {
        Some(list) if !list.is_empty() => list.iter().map(|emoji| emoji.as_str().into()).collect(),
        _ => DEFAULT_CHOICES.iter().map(|emoji| SharedString::from(*emoji)).collect(),
    };
    with_conversation(|conversation| {
        conversation.set_reaction_choices(ModelRc::new(VecModel::from(first)));
        conversation.set_reaction_picker(message_id.to_string().into());
    });
    let request = json!({ "@type": "getMessageAvailableReactions", "chat_id": chat_id, "message_id": message_id, "row_size": CHOICES });
    send(request, move |answer| {
        let Ok(Ok(available)) = answer.map(serde_json::from_value::<AvailableReactions>) else { return };
        let choices: Vec<SharedString> = store::with(|store| {
            let mut seen = Vec::new();
            for reaction in available.top_reactions.iter().chain(&available.recent_reactions).chain(&available.popular_reactions) {
                if reaction.needs_premium && !store.is_premium {
                    continue;
                }
                if let Some(emoji) = emoji_of(store, &reaction.kind)
                    && !seen.contains(&emoji)
                {
                    seen.push(emoji);
                }
                if seen.len() == CHOICES {
                    break;
                }
            }
            seen.into_iter().map(SharedString::from).collect()
        })
        .unwrap_or_default();
        if choices.is_empty() {
            return;
        }
        with_conversation(|conversation| {
            if conversation.get_reaction_picker() == message_id.to_string().as_str() {
                conversation.set_reaction_choices(ModelRc::new(VecModel::from(choices)));
            }
        });
    });
}

/// A reaction as the picker can show it: an emoji; a custom emoji's when known.
fn emoji_of(store: &store::Store, kind: &ReactionType) -> Option<String> {
    match kind {
        ReactionType::Emoji { emoji } => Some(emoji.clone()),
        ReactionType::CustomEmoji { custom_emoji_id } => store.custom_emoji.get(custom_emoji_id).filter(|emoji| !emoji.is_empty()).cloned(),
        ReactionType::Paid => None,
    }
}

fn close_picker() {
    with_conversation(|conversation| conversation.set_reaction_picker(SharedString::new()));
}

/// Give a reaction, or take it back when it is ours already. TDLib keeps to how many one may
/// give: the oldest of ours goes when a new one would be one too many.
fn react(message_id: i64, kind: ReactionType) {
    close_picker();
    let Some(chat_id) = open_chat() else { return };
    let chosen = store::with(|store| {
        let message = store.histories.get(&chat_id)?.messages.get(&message_id)?;
        Some(message.reactions().iter().any(|reaction| reaction.kind == kind && reaction.is_chosen))
    })
    .flatten()
    .unwrap_or(false);
    let request = if chosen {
        json!({ "@type": "removeMessageReaction", "chat_id": chat_id, "message_id": message_id, "reaction_type": kind.to_json() })
    } else {
        json!({
            "@type": "addMessageReaction", "chat_id": chat_id, "message_id": message_id, "reaction_type": kind.to_json(),
            "is_big": false, "update_recent_reactions": true,
        })
    };
    send(request, |answer| match answer {
        Ok(_) | Err(Error::Stopped) => {}
        Err(Error::Telegram { message, .. }) => actions::show_failure(&errors::explain(&message)),
    });
}

/// A double click: the account's default reaction (updateDefaultReactionType).
fn quick_react(message_id: i64) {
    let kind = store::with(|store| store.default_reaction.clone()).unwrap_or(ReactionType::Emoji { emoji: "❤️".into() });
    let kind = match kind {
        ReactionType::Paid => ReactionType::Emoji { emoji: "❤️".into() },
        other => other,
    };
    react(message_id, kind);
}

/// Custom emoji the rows met (reactions, topics' icons): which emoji each stands for.
pub fn fetch_custom_emoji(ids: Vec<i64>) {
    let fresh: Vec<i64> = store::with(|store| {
        let mut fresh = Vec::new();
        for id in ids {
            if let std::collections::hash_map::Entry::Vacant(entry) = store.custom_emoji.entry(id) {
                entry.insert(String::new());
                fresh.push(id);
            }
        }
        fresh
    })
    .unwrap_or_default();
    // TDLib takes two hundred at a time.
    for batch in fresh.chunks(200) {
        let ids: Vec<String> = batch.iter().map(|id| id.to_string()).collect();
        let asked: Vec<i64> = batch.to_vec();
        send(json!({ "@type": "getCustomEmojiStickers", "custom_emoji_ids": ids }), move |answer| {
            let Ok(Ok(stickers)) = answer.map(serde_json::from_value::<api::Stickers>) else {
                // Asked again another time.
                store::with(|store| {
                    for id in asked {
                        store.custom_emoji.remove(&id);
                    }
                });
                return;
            };
            store::with(|store| {
                for sticker in stickers.stickers {
                    let emoji = if sticker.emoji.is_empty() { "✨".to_string() } else { sticker.emoji };
                    store.custom_emoji.insert(sticker.id, emoji);
                }
                store.dirty.conversation = true;
                store.dirty.topics = true;
            });
            store::refresh();
        });
    }
}

/// Esc: the picker closes if it is open.
pub(super) fn escape() -> bool {
    let mut open = false;
    with_conversation(|conversation| open = !conversation.get_reaction_picker().is_empty());
    if open {
        close_picker();
    }
    open
}

/// Another chat came to the front, or none: the picker is for no message.
pub fn chat_changed() {
    close_picker();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_choices_are_telegrams_first_eight() {
        assert_eq!(DEFAULT_CHOICES.len(), CHOICES);
        assert_eq!(DEFAULT_CHOICES[0], "👍");
    }
}
