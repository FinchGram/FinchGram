//! What the MCP server's tools do with Telegram (src/mcp/tools.rs names them and checks their
//! arguments). On the UI thread, where the store and TDLib are: each call reads what the store has,
//! asks TDLib for the rest, and answers through `reply`, which carries the outcome back to the
//! connection's thread. What the user allows (Settings → AI access, `settings::Mcp`) is held to
//! here, in one place: a chat outside the allowed ones is not read, searched, written to or marked,
//! and a secret chat never is, whatever the settings. Reading marks nothing as read (getChatHistory,
//! never viewMessages); only mark_read does, and only when asked. Self-destructing media is
//! described but never handed out, nor is the media of a chat that restricts saving
//! (getMessageProperties: can_be_saved), as the media viewer's save button follows the same rule.
//! Each call also says what it did, in a word and a count, for the log on Settings → AI access
//! (src/mcp/access.rs); and the page's lists of chats come from here.

use std::cell::RefCell;
use std::rc::Rc;

use base64::Engine;
use serde_json::{Value, json};
use slint::ComponentHandle;

use super::api::{self, Chat, ChatList, ChatType, Message, MessageContent as M, MessageReplyTo, MessageSender, StickerFormat, TextEntityType, UserType};
use super::store::{self, Names, Store};
use super::{Error, send};
use crate::mcp::tools::{Activity, ActivityKind, Content, Outcome, ToolCall};
use crate::settings::{Mcp, McpScope, McpSending};
use crate::{AiChat, ChatKind, Chats, MainWindow};

/// A picture handed to the assistant as itself is at most this long on its longer side (Telegram
/// keeps photos in several sizes), and this large.
const PICTURE_SIDE_MAX: i32 = 1280;
const PICTURE_BYTES_MAX: u64 = 4 * 1024 * 1024;
/// getChatHistory may give fewer messages than asked for; it is asked again this often at most.
const HISTORY_PAGES_MAX: usize = 6;
/// How many of the chosen chats a search across them looks into.
const SEARCHED_CHATS_MAX: usize = 25;
/// A chat's last message in its row: this many characters at most.
const PREVIEW_CHARS: usize = 120;
/// TDLib's download priority for an assistant's file, 1 … 32: the pages' own pictures come first.
const DOWNLOAD_PRIORITY: i32 = 1;

const NOT_ALLOWED: &str = "This chat is not among the ones the user lets assistants see.";

type Found<T> = Box<dyn FnOnce(Result<T, Error>)>;

/// Where a tool's outcome goes, with what it did for the page's log (access.rs); a failure says
/// nothing there, except a chat outside the chosen ones, which the user should know was asked for.
pub struct Reply {
    send: Box<dyn FnOnce(Outcome, Option<Activity>)>,
}

impl Reply {
    fn done(self, outcome: Outcome, activity: Activity) {
        (self.send)(outcome, Some(activity));
    }

    fn failed(self, reason: impl Into<String>) {
        (self.send)(Outcome::failed(reason), None);
    }

    fn refuse(self, refusal: Refusal) {
        match refusal {
            Refusal::NotAllowed { chat } => (self.send)(Outcome::failed(NOT_ALLOWED), Some(Activity::about(ActivityKind::Refused, chat))),
            Refusal::Other(reason) => self.failed(reason),
        }
    }
}

/// Why a chat was not given: it is outside what the user allows (named, for the log), or TDLib
/// would not give it.
enum Refusal {
    NotAllowed { chat: String },
    Other(String),
}

/// Do what `call` asks, within `policy`, and give `reply` the outcome, now or once TDLib has
/// answered. UI thread.
pub fn call(ui: &MainWindow, call: ToolCall, policy: Mcp, reply: impl FnOnce(Outcome, Option<Activity>) + 'static) {
    let reply = Reply { send: Box::new(reply) };
    if !policy.enabled {
        return reply.failed(crate::mcp::protocol::OFF);
    }
    if store::with(|store| store.my_id == 0).unwrap_or(true) {
        return reply.failed("FinchGram is not signed in to Telegram, or is still starting; try again in a moment.");
    }
    let names = Names::from(ui);
    let loaded = ui.global::<Chats>().get_loaded();
    match call {
        ToolCall::ListChats { limit, unread_only } => list_chats(&policy, &names, loaded, limit, unread_only, reply),
        ToolCall::SearchChats { query, limit } => search_chats(policy, names, query, limit, reply),
        ToolCall::GetChat { chat_id } => get_chat(policy, names, chat_id, reply),
        ToolCall::GetMessages { chat_id, limit, before_message_id } => {
            get_messages(policy, names, chat_id, limit, before_message_id, reply)
        }
        ToolCall::SearchMessages { query, chat_id, limit } => search_messages(policy, names, query, chat_id, limit, reply),
        ToolCall::GetUnread { chats_limit, messages_per_chat } => get_unread(policy, names, chats_limit, messages_per_chat, reply),
        ToolCall::DownloadMedia { chat_id, message_id } => download_media(policy, names, chat_id, message_id, reply),
        ToolCall::DraftMessage { chat_id, text, reply_to_message_id } => {
            write_message(policy, names, chat_id, text, reply_to_message_id, false, reply)
        }
        ToolCall::SendMessage { chat_id, text, reply_to_message_id } => {
            write_message(policy, names, chat_id, text, reply_to_message_id, true, reply)
        }
        ToolCall::MarkRead { chat_id, message_ids } => mark_read(policy, names, chat_id, message_ids, reply),
        ToolCall::GetMe => get_me(&names, reply),
    }
}

// ---- what the user allows ------------------------------------------------------------------------

/// Whether the assistant may see `chat`: within the user's choice, and never a secret chat.
fn allowed(policy: &Mcp, chat: &Chat) -> bool {
    !matches!(chat.kind, ChatType::Secret { .. }) && policy.allows(chat.id)
}

/// The chat with `chat_id`, from the store or from TDLib, when the assistant may see it; else why
/// not.
fn with_chat(policy: Mcp, chat_id: i64, then: impl FnOnce(Result<Chat, Refusal>) + 'static) {
    let refused = move |chat: Option<&Chat>| Refusal::NotAllowed {
        chat: chat.map(|chat| chat.title.clone()).filter(|title| !title.is_empty()).unwrap_or_else(|| format!("chat {chat_id}")),
    };
    if !policy.allows(chat_id) {
        let chat = store::with(|store| store.chats.get(&chat_id).map(|chat| refused(Some(chat)))).flatten().unwrap_or_else(|| refused(None));
        return then(Err(chat));
    }
    let check = move |chat: Chat| if allowed(&policy, &chat) { Ok(chat) } else { Err(refused(Some(&chat))) };
    if let Some(chat) = store::with(|store| store.chats.get(&chat_id).cloned()).flatten() {
        return then(check(chat));
    }
    send(json!({ "@type": "getChat", "chat_id": chat_id }), move |answer| {
        then(match answer.map(serde_json::from_value::<Chat>) {
            Ok(Ok(chat)) => check(chat),
            Ok(Err(err)) => Err(Refusal::Other(format!("cannot read the chat: {err}"))),
            Err(err) => Err(Refusal::Other(telegram_error("find the chat", err))),
        })
    });
}

fn telegram_error(doing: &str, err: Error) -> String {
    match err {
        Error::Telegram { code, message } => format!("Telegram would not {doing}: {message} ({code})"),
        Error::Stopped => "FinchGram is restarting its connection to Telegram; try again in a moment.".to_string(),
    }
}

// ---- the tools -------------------------------------------------------------------------------------

fn list_chats(policy: &Mcp, names: &Names, loaded: bool, limit: usize, unread_only: bool, reply: Reply) {
    let (value, count) = store::with(|store| {
        let chats = main_list(store, policy, |chat| !unread_only || chat.unread_count > 0 || chat.is_marked_as_unread);
        let rows: Vec<Value> = chats.iter().take(limit).map(|chat| chat_json(store, names, chat)).collect();
        let count = rows.len();
        let mut value = json!({ "chats": rows, "total": chats.len() });
        if !loaded {
            value["note"] = "The chat list is still coming from Telegram; ask again for the rest.".into();
        }
        (value, count)
    })
    .unwrap_or_else(|| (json!({ "chats": [], "total": 0 }), 0));
    reply.done(Outcome::json(&value), Activity::counted(ActivityKind::Listed, "", count));
}

/// The allowed chats of the main list that `keep` keeps, in the list's order.
fn main_list<'a>(store: &'a Store, policy: &Mcp, keep: impl Fn(&Chat) -> bool) -> Vec<&'a Chat> {
    let mut chats: Vec<(&Chat, i64)> = store
        .chats
        .values()
        .filter(|chat| allowed(policy, chat) && keep(chat))
        .filter_map(|chat| store::position(chat, ChatList::Main).map(|position| (chat, position.order)))
        .collect();
    chats.sort_by(|(a, a_order), (b, b_order)| b_order.cmp(a_order).then(b.id.cmp(&a.id)));
    chats.into_iter().map(|(chat, _)| chat).collect()
}

fn search_chats(policy: Mcp, names: Names, query: String, limit: usize, reply: Reply) {
    let query = query.trim_start_matches('@').to_string();
    send(json!({ "@type": "searchChats", "query": query, "limit": limit as i32 }), move |answer| {
        let ids: Vec<i64> = match answer {
            Ok(found) => found["chat_ids"].as_array().map(|ids| ids.iter().filter_map(Value::as_i64).collect()).unwrap_or_default(),
            Err(err) => return reply.failed(telegram_error("search the chats", err)),
        };
        // TDLib has sent every chat it names before it answers.
        let rows: Vec<Value> = store::with(|store| {
            ids.iter()
                .filter_map(|id| store.chats.get(id))
                .filter(|chat| allowed(&policy, chat))
                .map(|chat| chat_json(store, &names, chat))
                .collect()
        })
        .unwrap_or_default();
        let count = rows.len();
        reply.done(Outcome::json(&json!({ "chats": rows })), Activity::counted(ActivityKind::SearchedChats, "", count));
    });
}

fn get_chat(policy: Mcp, names: Names, chat_id: i64, reply: Reply) {
    with_chat(policy, chat_id, move |chat| match chat {
        Err(refusal) => reply.refuse(refusal),
        Ok(chat) => {
            let (value, title) = store::with(|store| (chat_json(store, &names, &chat), store.title(&chat, &names))).unwrap_or_default();
            reply.done(Outcome::json(&value), Activity::about(ActivityKind::Looked, title));
        }
    });
}

fn get_messages(policy: Mcp, names: Names, chat_id: i64, limit: usize, before: i64, reply: Reply) {
    with_chat(policy, chat_id, move |chat| {
        let chat = match chat {
            Ok(chat) => chat,
            Err(refusal) => return reply.refuse(refusal),
        };
        history(chat.id, before, limit, Vec::new(), 0, Box::new(move |messages| {
            let messages = match messages {
                Ok(messages) => messages,
                Err(err) => return reply.failed(telegram_error("read the messages", err)),
            };
            let (value, title) = store::with(|store| {
                let title = store.title(&chat, &names);
                let rows: Vec<Value> = messages.iter().map(|message| message_json(store, &names, message)).collect();
                let mut value = json!({
                    "chat": { "id": chat.id, "title": title, "kind": kind_of(store, &chat) },
                    "messages": rows,
                });
                if let Some(oldest) = messages.last() {
                    value["older_before_message_id"] = oldest.id.into();
                }
                (value, title)
            })
            .unwrap_or_default();
            reply.done(Outcome::json(&value), Activity::counted(ActivityKind::Read, title, messages.len()));
        }));
    });
}

/// `want` messages of a chat, newest first, older than `before` (0: from the end), asking TDLib
/// again while it gives fewer than asked for.
fn history(chat_id: i64, before: i64, want: usize, mut got: Vec<Message>, pages: usize, then: Found<Vec<Message>>) {
    let ask = want.saturating_sub(got.len()).clamp(1, 100) as i32;
    let request = json!({
        "@type": "getChatHistory", "chat_id": chat_id, "from_message_id": before, "offset": 0, "limit": ask, "only_local": false,
    });
    send(request, move |answer| match answer.map(serde_json::from_value::<api::Messages>) {
        Err(err) => then(Err(err)),
        Ok(Err(err)) => {
            eprintln!("mcp: cannot read the messages TDLib gave: {err}");
            then(Ok(got));
        }
        Ok(Ok(page)) => {
            let page: Vec<Message> = page
                .messages
                .into_iter()
                .flatten()
                .filter(|message| (before == 0 || message.id < before) && !got.iter().any(|known| known.id == message.id))
                .collect();
            let oldest = page.last().map(|message| message.id);
            got.extend(page);
            match oldest {
                Some(oldest) if got.len() < want && pages + 1 < HISTORY_PAGES_MAX => {
                    history(chat_id, oldest, want, got, pages + 1, then)
                }
                _ => then(Ok(got)),
            }
        }
    });
}

fn search_messages(policy: Mcp, names: Names, query: String, chat_id: Option<i64>, limit: usize, reply: Reply) {
    if let Some(chat_id) = chat_id {
        return with_chat(policy, chat_id, move |chat| {
            let chat = match chat {
                Ok(chat) => chat,
                Err(refusal) => return reply.refuse(refusal),
            };
            let title = store::with(|store| store.title(&chat, &names)).unwrap_or_default();
            search_in(chat.id, query, limit, Box::new(move |found| match found {
                Err(err) => reply.failed(telegram_error("search the messages", err)),
                Ok(messages) => reply.done(found_json(&names, &messages, true), Activity::counted(ActivityKind::Searched, title, 1)),
            }));
        });
    }
    match policy.scope {
        McpScope::All => {
            let request = json!({
                "@type": "searchMessages", "chat_list": null, "query": query, "offset": "", "limit": limit as i32,
                "filter": null, "chat_type_filter": null, "min_date": 0, "max_date": 0,
            });
            send(request, move |answer| match answer {
                Err(err) => reply.failed(telegram_error("search the messages", err)),
                Ok(found) => {
                    let messages: Vec<Message> = serde_json::from_value(found["messages"].clone()).unwrap_or_default();
                    let messages: Vec<Message> = store::with(|store| {
                        messages
                            .into_iter()
                            .filter(|message| store.chats.get(&message.chat_id).is_none_or(|chat| allowed(&policy, chat)))
                            .collect()
                    })
                    .unwrap_or_default();
                    reply.done(found_json(&names, &messages, false), Activity::new(ActivityKind::SearchedAll));
                }
            });
        }
        McpScope::Chosen => {
            let chats: Vec<i64> = policy.chats.iter().copied().take(SEARCHED_CHATS_MAX).collect();
            if chats.is_empty() {
                return reply.done(Outcome::json(&json!({ "messages": [], "count": 0 })), Activity::counted(ActivityKind::Searched, "", 0));
            }
            let searched = chats.len();
            let gather = Gather::new(chats.len(), move |pages: Vec<Vec<Message>>| {
                let mut messages: Vec<Message> = pages.into_iter().flatten().collect();
                messages.sort_by(|a, b| b.date.cmp(&a.date).then(b.id.cmp(&a.id)));
                messages.truncate(limit);
                reply.done(found_json(&names, &messages, false), Activity::counted(ActivityKind::Searched, "", searched));
            });
            for (index, chat_id) in chats.into_iter().enumerate() {
                let gather = gather.clone();
                search_in(chat_id, query.clone(), limit, Box::new(move |found| {
                    Gather::done(&gather, index, found.unwrap_or_default());
                }));
            }
        }
    }
}

/// The messages of one chat with `query`'s words, newest first (Telegram's servers search).
fn search_in(chat_id: i64, query: String, limit: usize, then: Found<Vec<Message>>) {
    let request = json!({
        "@type": "searchChatMessages", "chat_id": chat_id, "query": query, "sender_id": null,
        "from_message_id": 0, "offset": 0, "limit": limit as i32, "filter": { "@type": "searchMessagesFilterEmpty" },
    });
    send(request, move |answer| {
        then(answer.map(|found| serde_json::from_value::<api::FoundChatMessages>(found).map(|found| found.messages).unwrap_or_default()))
    });
}

/// Found messages as the assistant gets them; with their chat named when they are from several.
fn found_json(names: &Names, messages: &[Message], one_chat: bool) -> Outcome {
    let rows: Vec<Value> = store::with(|store| {
        messages
            .iter()
            .map(|message| {
                let mut row = message_json(store, names, message);
                if !one_chat {
                    row["chat_id"] = message.chat_id.into();
                    row["chat"] = store.chats.get(&message.chat_id).map(|chat| store.title(chat, names)).unwrap_or_default().into();
                }
                row
            })
            .collect()
    })
    .unwrap_or_default();
    let count = rows.len();
    Outcome::json(&json!({ "messages": rows, "count": count }))
}

fn get_unread(policy: Mcp, names: Names, chats_limit: usize, per_chat: usize, reply: Reply) {
    let chats: Vec<(i64, i64, usize)> = store::with(|store| {
        main_list(store, &policy, |chat| chat.unread_count > 0)
            .into_iter()
            .take(chats_limit)
            .map(|chat| (chat.id, chat.last_read_inbox_message_id, usize::try_from(chat.unread_count).unwrap_or(1).min(per_chat)))
            .collect()
    })
    .unwrap_or_default();
    if chats.is_empty() {
        return reply.done(Outcome::json(&json!({ "chats": [], "note": "Nothing is unread." })), Activity::counted(ActivityKind::Unread, "", 0));
    }
    let gather = Gather::new(chats.len(), move |pages: Vec<(i64, i64, Vec<Message>)>| {
        let rows: Vec<Value> = store::with(|store| {
            pages
                .iter()
                .filter_map(|(chat_id, last_read, messages)| {
                    let chat = store.chats.get(chat_id)?;
                    // Unread: what came in after the last message read, not what the user sent.
                    let unread: Vec<Value> = messages
                        .iter()
                        .filter(|message| message.id > *last_read && !message.is_outgoing)
                        .map(|message| message_json(store, &names, message))
                        .collect();
                    Some(json!({ "chat": chat_json(store, &names, chat), "messages": unread }))
                })
                .collect()
        })
        .unwrap_or_default();
        let count = rows.len();
        reply.done(Outcome::json(&json!({ "chats": rows })), Activity::counted(ActivityKind::Unread, "", count));
    });
    for (index, (chat_id, last_read, want)) in chats.into_iter().enumerate() {
        let gather = gather.clone();
        history(chat_id, 0, want, Vec::new(), 0, Box::new(move |messages| {
            Gather::done(&gather, index, (chat_id, last_read, messages.unwrap_or_default()));
        }));
    }
}

fn download_media(policy: Mcp, names: Names, chat_id: i64, message_id: i64, reply: Reply) {
    with_chat(policy, chat_id, move |chat| {
        let chat = match chat {
            Ok(chat) => chat,
            Err(refusal) => return reply.refuse(refusal),
        };
        let title = store::with(|store| store.title(&chat, &names)).unwrap_or_default();
        send(json!({ "@type": "getMessage", "chat_id": chat.id, "message_id": message_id }), move |answer| {
            let message = match answer.map(serde_json::from_value::<Message>) {
                Ok(Ok(message)) => message,
                Ok(Err(err)) => return reply.failed(format!("cannot read the message: {err}")),
                Err(err) => return reply.failed(telegram_error("find the message", err)),
            };
            if store::is_secret(&message.content) {
                return reply.failed("Self-destructing media is for the user's eyes only; it is not handed to assistants.");
            }
            let Some(target) = media_file(&message.content) else {
                return reply.failed("This message has no photo, sticker, video, GIF or file to download.");
            };
            send(json!({ "@type": "getMessageProperties", "chat_id": chat.id, "message_id": message_id }), move |answer| {
                match answer {
                    Ok(properties) if properties["can_be_saved"].as_bool() == Some(true) => {}
                    Ok(_) => return reply.failed("This chat restricts saving its content, so its media stays in FinchGram."),
                    Err(err) => return reply.failed(telegram_error("check the message", err)),
                }
                let request = json!({
                    "@type": "downloadFile", "file_id": target.file_id, "priority": DOWNLOAD_PRIORITY, "offset": 0, "limit": 0, "synchronous": true,
                });
                send(request, move |answer| {
                    let file = match answer.map(serde_json::from_value::<api::File>) {
                        Ok(Ok(file)) => file,
                        Ok(Err(err)) => return reply.failed(format!("cannot read the file TDLib gave: {err}")),
                        Err(err) => return reply.failed(telegram_error("download the file", err)),
                    };
                    if !file.local.is_downloading_completed || file.local.path.is_empty() {
                        return reply.failed("The download did not finish.");
                    }
                    match deliver(&target, &file) {
                        Ok(outcome) => reply.done(outcome, Activity::about(ActivityKind::Media, title)),
                        Err(reason) => reply.failed(reason),
                    }
                });
            });
        });
    });
}

/// A message's file for download_media, and how to hand it over.
struct Target {
    file_id: i32,
    kind: &'static str,
    file_name: String,
    mime_type: String,
    /// It can be given as a picture (the assistant sees it), not only as a path.
    picture: bool,
}

fn media_file(content: &M) -> Option<Target> {
    Some(match content {
        M::Photo { photo, .. } => {
            let size = fitting(photo)?;
            Target { file_id: size.photo.id, kind: "photo", file_name: String::new(), mime_type: "image/jpeg".into(), picture: true }
        }
        M::Sticker { sticker } => Target {
            file_id: sticker.sticker.id,
            kind: "sticker",
            file_name: String::new(),
            mime_type: match sticker.format {
                StickerFormat::Webp => "image/webp",
                StickerFormat::Tgs => "application/x-tgsticker",
                StickerFormat::Webm => "video/webm",
            }
            .into(),
            picture: sticker.format == StickerFormat::Webp,
        },
        M::Video { video, .. } => Target {
            file_id: video.video.id,
            kind: "video",
            file_name: video.file_name.clone(),
            mime_type: "video/mp4".into(),
            picture: false,
        },
        M::Animation { animation, .. } => Target {
            file_id: animation.animation.id,
            kind: "gif",
            file_name: animation.file_name.clone(),
            mime_type: "video/mp4".into(),
            picture: false,
        },
        M::Document { document, .. } => Target {
            file_id: document.document.id,
            kind: "file",
            file_name: document.file_name.clone(),
            mime_type: document.mime_type.clone(),
            picture: matches!(document.mime_type.as_str(), "image/jpeg" | "image/png" | "image/gif" | "image/webp"),
        },
        _ => return None,
    })
}

/// The downloaded file for the assistant: a picture as itself (and where it is), anything else as
/// where it is.
fn deliver(target: &Target, file: &api::File) -> Result<Outcome, String> {
    let about = json!({
        "kind": target.kind, "path": file.local.path, "file_name": target.file_name, "mime_type": target.mime_type,
        "size": file.size_or_expected(),
    });
    if target.picture && u64::try_from(file.size_or_expected()).is_ok_and(|size| size <= PICTURE_BYTES_MAX) {
        match std::fs::read(&file.local.path) {
            Ok(bytes) if bytes.len() as u64 <= PICTURE_BYTES_MAX => {
                let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
                return Ok(Outcome::Done(vec![
                    Content::Image { data, mime_type: target.mime_type.clone() },
                    Content::Text(serde_json::to_string_pretty(&about).unwrap_or_default()),
                ]));
            }
            Ok(_) => {}
            Err(err) => return Err(format!("cannot read the downloaded file: {err}")),
        }
    }
    Ok(Outcome::json(&about))
}

/// A draft (`sending` false) or a message, with its Markdown read into formatting by TDLib.
fn write_message(policy: Mcp, names: Names, chat_id: i64, text: String, reply_to: Option<i64>, sending: bool, reply: Reply) {
    match (policy.sending, sending) {
        (McpSending::Off, _) => return reply.failed("The user does not let assistants write messages or drafts."),
        (McpSending::Drafts, true) => {
            return reply.failed("The user lets assistants write drafts only: use draft_message, and the user sends it.");
        }
        _ => {}
    }
    with_chat(policy, chat_id, move |chat| {
        let chat = match chat {
            Ok(chat) => chat,
            Err(refusal) => return reply.refuse(refusal),
        };
        let words = json!({ "@type": "formattedText", "text": text, "entities": [] });
        send(json!({ "@type": "parseMarkdown", "text": words }), move |answer| {
            let formatted = match answer {
                Ok(formatted) => formatted,
                Err(err) => return reply.failed(telegram_error("read the Markdown", err)),
            };
            let reply_to = reply_to.map(|message_id| {
                json!({ "@type": "inputMessageReplyToMessage", "message_id": message_id, "quote": null, "checklist_task_id": 0, "poll_option_id": "" })
            });
            let title = store::with(|store| store.title(&chat, &names)).unwrap_or_default();
            if sending {
                // The user's own draft in the chat stays: this is not it being sent.
                let request = json!({
                    "@type": "sendMessage", "chat_id": chat.id, "topic_id": null, "reply_to": reply_to, "options": null, "reply_markup": null,
                    "input_message_content": {
                        "@type": "inputMessageText", "text": formatted, "link_preview_options": null, "clear_draft": false,
                    },
                });
                send(request, move |answer| match answer {
                    Ok(sent) => reply.done(
                        Outcome::json(&json!({ "sent": true, "chat_id": chat.id, "chat": title, "message_id": sent["id"] })),
                        Activity::about(ActivityKind::Sent, title),
                    ),
                    Err(err) => reply.failed(telegram_error("send the message", err)),
                });
            } else {
                let request = json!({
                    "@type": "setChatDraftMessage", "chat_id": chat.id, "topic_id": null,
                    "draft_message": {
                        "@type": "draftMessage", "reply_to": reply_to, "date": 0,
                        "content": { "@type": "draftMessageContentText", "text": formatted, "link_preview_options": null },
                    },
                });
                send(request, move |answer| match answer {
                    Ok(_) => reply.done(
                        Outcome::json(&json!({
                            "draft_saved": true, "chat_id": chat.id, "chat": title,
                            "note": "The user can send it from FinchGram or any Telegram app.",
                        })),
                        Activity::about(ActivityKind::Draft, title),
                    ),
                    Err(err) => reply.failed(telegram_error("save the draft", err)),
                });
            }
        });
    });
}

fn mark_read(policy: Mcp, names: Names, chat_id: i64, message_ids: Vec<i64>, reply: Reply) {
    with_chat(policy, chat_id, move |chat| {
        let chat = match chat {
            Ok(chat) => chat,
            Err(refusal) => return reply.refuse(refusal),
        };
        let title = store::with(|store| store.title(&chat, &names)).unwrap_or_default();
        let count = message_ids.len();
        let request = json!({
            "@type": "viewMessages", "chat_id": chat.id, "message_ids": message_ids,
            "source": { "@type": "messageSourceChatHistory" }, "force_read": true,
        });
        send(request, move |answer| match answer {
            Ok(_) => reply.done(Outcome::json(&json!({ "marked_read": count, "chat_id": chat.id })), Activity::counted(ActivityKind::Marked, title, count)),
            Err(err) => reply.failed(telegram_error("mark the messages as read", err)),
        });
    });
}

fn get_me(names: &Names, reply: Reply) {
    let value = store::with(|store| {
        let user = store.users.get(&store.my_id);
        json!({
            "id": store.my_id,
            "name": store.user_name(store.my_id, names),
            "username": user.and_then(|user| user.usernames.as_ref()).and_then(|usernames| usernames.active_usernames.first().cloned()),
            "is_premium": store.is_premium,
        })
    });
    match value {
        Some(value) => reply.done(Outcome::json(&value), Activity::new(ActivityKind::Account)),
        None => reply.failed("FinchGram has no account yet."),
    }
}

// ---- Settings → AI access -------------------------------------------------------------------------

/// The chats the picker offers (Choose chats): those of the main list, in its order, matching
/// `query` by title. Secret chats are among them, marked, so that the user sees why one cannot be
/// chosen.
pub fn picker_chats(ui: &MainWindow, query: &str, chosen: &[i64]) -> Vec<AiChat> {
    let names = Names::from(ui);
    let query = query.trim().to_lowercase();
    store::with(|store| {
        let mut chats: Vec<(&Chat, i64)> = store
            .chats
            .values()
            .filter_map(|chat| store::position(chat, ChatList::Main).map(|position| (chat, position.order)))
            .filter(|(chat, _)| query.is_empty() || store.title(chat, &names).to_lowercase().contains(&query))
            .collect();
        chats.sort_by(|(a, a_order), (b, b_order)| b_order.cmp(a_order).then(b.id.cmp(&a.id)));
        chats.into_iter().map(|(chat, _)| ai_chat(store, &names, chat, chosen.contains(&chat.id))).collect()
    })
    .unwrap_or_default()
}

/// The chosen chats as the page lists them, in the order chosen; one the store does not know yet
/// (the list is still coming) goes by its id.
pub fn chosen_chats(ui: &MainWindow, chosen: &[i64]) -> Vec<AiChat> {
    let names = Names::from(ui);
    store::with(|store| {
        chosen
            .iter()
            .map(|id| match store.chats.get(id) {
                Some(chat) => ai_chat(store, &names, chat, true),
                None => AiChat { id: id.to_string().into(), title: format!("Chat {id}").into(), kind: ChatKind::User, members: 0, secret: false, chosen: true },
            })
            .collect()
    })
    .unwrap_or_default()
}

fn ai_chat(store: &Store, names: &Names, chat: &Chat, chosen: bool) -> AiChat {
    AiChat {
        id: chat.id.to_string().into(),
        title: store.title(chat, names).into(),
        kind: store.kind(chat),
        members: store.members(chat),
        secret: matches!(chat.kind, ChatType::Secret { .. }),
        chosen,
    }
}

/// A moment in seconds since the epoch, taken apart for the pages (time.rs).
pub fn moment(unix: i64) -> crate::Moment {
    super::time::moment(i32::try_from(unix).unwrap_or(0))
}

// ---- several requests at once ---------------------------------------------------------------------

/// Several TDLib requests in flight; `finish` gets every result, in the order asked, once the last
/// has come.
struct Gather<T> {
    left: usize,
    results: Vec<(usize, T)>,
    finish: Option<Box<dyn FnOnce(Vec<T>)>>,
}

impl<T: 'static> Gather<T> {
    fn new(count: usize, finish: impl FnOnce(Vec<T>) + 'static) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Gather { left: count, results: Vec::new(), finish: Some(Box::new(finish)) }))
    }

    fn done(this: &Rc<RefCell<Self>>, index: usize, result: T) {
        let complete = {
            let mut gather = this.borrow_mut();
            gather.results.push((index, result));
            gather.left = gather.left.saturating_sub(1);
            if gather.left == 0 {
                let mut results = std::mem::take(&mut gather.results);
                results.sort_by_key(|(index, _)| *index);
                gather.finish.take().map(|finish| (finish, results))
            } else {
                None
            }
        };
        if let Some((finish, results)) = complete {
            finish(results.into_iter().map(|(_, result)| result).collect());
        }
    }
}

// ---- how things are described -----------------------------------------------------------------------

fn chat_json(store: &Store, names: &Names, chat: &Chat) -> Value {
    let mut value = json!({
        "id": chat.id,
        "title": store.title(chat, names),
        "kind": kind_of(store, chat),
        "unread_count": chat.unread_count,
        "muted": store.muted(chat),
        "pinned": store::pinned_in(chat).contains(&ChatList::Main),
    });
    if let Some(username) = username_of(store, chat) {
        value["username"] = username.into();
    }
    if let Some(members) = member_count(store, chat) {
        value["member_count"] = members.into();
    }
    if chat.is_marked_as_unread {
        value["marked_unread"] = true.into();
    }
    if let Some(last) = &chat.last_message {
        value["last_message"] = json!({
            "id": last.id, "date": iso(last.date), "from": sender_of(store, names, last), "text": preview(store, names, last),
        });
    }
    value
}

fn kind_of(store: &Store, chat: &Chat) -> &'static str {
    match chat.kind {
        ChatType::Private { user_id } if user_id == store.my_id => "saved_messages",
        ChatType::Private { user_id } => {
            if store.users.get(&user_id).is_some_and(|user| user.kind == UserType::Bot) { "bot" } else { "private" }
        }
        ChatType::BasicGroup { .. } | ChatType::Supergroup { is_channel: false, .. } => "group",
        ChatType::Supergroup { is_channel: true, .. } => "channel",
        ChatType::Secret { .. } => "secret",
    }
}

fn username_of(store: &Store, chat: &Chat) -> Option<String> {
    let usernames = match chat.kind {
        ChatType::Private { user_id } | ChatType::Secret { user_id } => store.users.get(&user_id)?.usernames.as_ref()?,
        ChatType::Supergroup { supergroup_id, .. } => store.supergroups.get(&supergroup_id)?.usernames.as_ref()?,
        ChatType::BasicGroup { .. } => return None,
    };
    usernames.active_usernames.first().cloned()
}

fn member_count(store: &Store, chat: &Chat) -> Option<i32> {
    match chat.kind {
        ChatType::BasicGroup { basic_group_id } => store.basic_groups.get(&basic_group_id).map(|group| group.member_count),
        ChatType::Supergroup { supergroup_id, .. } => store
            .supergroup_members
            .get(&supergroup_id)
            .copied()
            .or_else(|| store.supergroups.get(&supergroup_id).map(|group| group.member_count))
            .filter(|count| *count > 0),
        _ => None,
    }
}

fn sender_of(store: &Store, names: &Names, message: &Message) -> String {
    let name = store.sender_name(&message.sender_id, names);
    if name.is_empty() { "unknown".to_string() } else { name }
}

/// A chat's last message in a word or two: its words, or what kind of thing it is.
fn preview(store: &Store, names: &Names, message: &Message) -> String {
    if let Some(service) = service_text(store, names, message) {
        return service;
    }
    let words = text_of(&message.content).unwrap_or_default();
    let label = media_json(&message.content).and_then(|media| media["kind"].as_str().map(str::to_string));
    let words: String = words.chars().take(PREVIEW_CHARS).collect();
    match (label, words.is_empty()) {
        (None, _) => words,
        (Some(label), true) => format!("[{label}]"),
        (Some(label), false) => format!("[{label}] {words}"),
    }
}

/// A message as the assistant gets it.
fn message_json(store: &Store, names: &Names, message: &Message) -> Value {
    let mut value = json!({
        "id": message.id, "date": iso(message.date), "from": sender_of(store, names, message), "outgoing": message.is_outgoing,
    });
    match &message.sender_id {
        MessageSender::User { user_id } => value["from_user_id"] = (*user_id).into(),
        MessageSender::Chat { chat_id } => value["from_chat_id"] = (*chat_id).into(),
    }
    if let Some(text) = text_of(&message.content) {
        value["text"] = text.into();
    }
    if let Some(formatted) = store::formatted(&message.content) {
        let links = links_of(formatted);
        if !links.is_empty() {
            value["links"] = links.into();
        }
    }
    if let Some(media) = media_json(&message.content) {
        value["media"] = media;
    }
    if let Some(service) = service_text(store, names, message) {
        value["service"] = service.into();
    }
    if let Some(MessageReplyTo::Message { message_id, .. }) = &message.reply_to {
        value["reply_to_message_id"] = (*message_id).into();
    }
    if let Some(forward) = &message.forward_info {
        value["forwarded_from"] = store.origin_name(&forward.origin, names).into();
    }
    if message.edit_date > 0 {
        value["edited"] = true.into();
    }
    if store::is_secret(&message.content) {
        value["self_destructing"] = true.into();
    }
    value
}

/// A message's words: its text or caption, or the emoji it is.
fn text_of(content: &M) -> Option<String> {
    match content {
        M::AnimatedEmoji { emoji } | M::Dice { emoji } => Some(emoji.clone()),
        _ => store::formatted(content).map(|text| text.text.clone()).filter(|text| !text.is_empty()),
    }
}

/// The links in a text that its words do not show: words that lead to an address, and names that
/// lead to a user.
fn links_of(text: &api::FormattedText) -> Vec<Value> {
    text.entities
        .iter()
        .filter_map(|entity| match &entity.kind {
            TextEntityType::TextUrl { url } => Some(json!({ "text": utf16_slice(&text.text, entity.offset, entity.length), "url": url })),
            TextEntityType::MentionName { user_id } => {
                Some(json!({ "text": utf16_slice(&text.text, entity.offset, entity.length), "user_id": user_id }))
            }
            _ => None,
        })
        .collect()
}

/// What a message holds besides words, as the assistant is told of it. Nothing for a text or a
/// service message. The file ids of self-destructing media are kept back.
fn media_json(content: &M) -> Option<Value> {
    let secret = store::is_secret(content);
    let file_id = |file: &api::File| if secret { Value::Null } else { file.id.into() };
    Some(match content {
        M::Photo { photo, .. } => {
            let size = biggest(photo);
            json!({
                "kind": "photo",
                "file_id": size.map(|size| file_id(&size.photo)).unwrap_or(Value::Null),
                "width": size.map_or(0, |size| size.width), "height": size.map_or(0, |size| size.height),
                "size": size.map_or(0, |size| size.photo.size_or_expected()),
            })
        }
        M::Video { video, .. } => json!({
            "kind": "video", "file_id": file_id(&video.video), "duration": video.duration, "width": video.width, "height": video.height,
            "file_name": video.file_name, "size": video.video.size_or_expected(),
        }),
        M::Animation { animation, .. } => json!({
            "kind": "gif", "file_id": file_id(&animation.animation), "duration": animation.duration, "width": animation.width,
            "height": animation.height, "size": animation.animation.size_or_expected(),
        }),
        M::Audio { audio, .. } => json!({ "kind": "audio", "title": audio.title, "performer": audio.performer, "file_name": audio.file_name }),
        M::Document { document, .. } => json!({
            "kind": "file", "file_id": file_id(&document.document), "file_name": document.file_name, "mime_type": document.mime_type,
            "size": document.document.size_or_expected(),
        }),
        M::VoiceNote { .. } => json!({ "kind": "voice_note" }),
        M::VideoNote {} => json!({ "kind": "video_note" }),
        M::Sticker { sticker } => json!({
            "kind": "sticker", "emoji": sticker.emoji, "animated": sticker.format != StickerFormat::Webp, "file_id": sticker.sticker.id,
        }),
        M::Location {} => json!({ "kind": "location" }),
        M::Venue { venue } => json!({ "kind": "venue", "title": venue.title }),
        M::Contact { contact } => {
            json!({ "kind": "contact", "name": format!("{} {}", contact.first_name, contact.last_name).trim() })
        }
        M::Poll { poll } => json!({ "kind": "poll", "question": poll.question.text }),
        M::Game { game } => json!({ "kind": "game", "title": game.title }),
        M::Invoice {} => json!({ "kind": "invoice" }),
        M::Call {} => json!({ "kind": "call" }),
        M::Story {} => json!({ "kind": "story" }),
        M::Gift {} => json!({ "kind": "gift" }),
        M::ExpiredPhoto {} | M::ExpiredVideo {} | M::ExpiredVideoNote {} | M::ExpiredVoiceNote {} => {
            json!({ "kind": "expired", "note": "self-destructing media that is gone" })
        }
        M::Other => json!({ "kind": "unsupported" }),
        _ => return None,
    })
}

/// A service message in words ("Alice joined"), or None for an ordinary one.
fn service_text(store: &Store, names: &Names, message: &Message) -> Option<String> {
    use crate::Content as C;
    let (kind, _, detail) = store.content(message, names);
    if !store::is_service(kind) {
        return None;
    }
    let who = sender_of(store, names, message);
    Some(match kind {
        C::ChatCreated => format!("{who} created the chat \"{detail}\""),
        C::TitleChanged => format!("{who} changed the title to \"{detail}\""),
        C::PhotoChanged => format!("{who} changed the chat photo"),
        C::PhotoRemoved => format!("{who} removed the chat photo"),
        C::MembersAdded => format!("{who} added {detail}"),
        C::MemberJoined => format!("{who} joined"),
        C::MemberLeft => format!("{who} left"),
        C::MemberRemoved => format!("{who} removed {detail}"),
        C::MessagePinned => format!("{who} pinned a message"),
        C::ScreenshotTaken => format!("{who} took a screenshot"),
        C::ContactJoined => format!("{who} joined Telegram"),
        _ => "a service message".to_string(),
    })
}

/// The photo's largest size.
fn biggest(photo: &api::Photo) -> Option<&api::PhotoSize> {
    photo.sizes.iter().max_by_key(|size| i64::from(size.width) * i64::from(size.height))
}

/// The photo's largest size within what is handed to an assistant, or its smallest when none is.
fn fitting(photo: &api::Photo) -> Option<&api::PhotoSize> {
    photo
        .sizes
        .iter()
        .filter(|size| size.width.max(size.height) <= PICTURE_SIDE_MAX)
        .max_by_key(|size| i64::from(size.width) * i64::from(size.height))
        .or_else(|| photo.sizes.iter().min_by_key(|size| i64::from(size.width) * i64::from(size.height)))
}

/// A moment as the assistant reads it: the local time with its offset, "2026-10-12T14:03:05+08:00".
fn iso(unix: i32) -> String {
    chrono::DateTime::from_timestamp(i64::from(unix), 0)
        .map(|moment| moment.with_timezone(&chrono::Local).to_rfc3339_opts(chrono::SecondsFormat::Secs, false))
        .unwrap_or_default()
}

/// The part of `text` an entity covers: TDLib counts offsets and lengths in UTF-16 code units.
fn utf16_slice(text: &str, offset: i32, length: i32) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    let start = usize::try_from(offset).unwrap_or(0).min(units.len());
    let end = start.saturating_add(usize::try_from(length).unwrap_or(0)).min(units.len());
    String::from_utf16_lossy(&units[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(content: &str) -> Message {
        serde_json::from_str(&format!(
            r#"{{"@type":"message","id":77,"sender_id":{{"@type":"messageSenderUser","user_id":5}},"chat_id":9,"is_outgoing":false,
            "date":1760000000,"edit_date":0,"content":{content}}}"#
        ))
        .expect("a message")
    }

    fn file(id: i32, size: i64) -> String {
        format!(
            r#"{{"@type":"file","id":{id},"size":{size},"expected_size":{size},"local":{{"@type":"localFile","path":"","is_downloading_active":false,"is_downloading_completed":false,"downloaded_size":0}},"remote":{{"@type":"remoteFile","is_uploading_active":false,"is_uploading_completed":true,"uploaded_size":{size}}}}}"#
        )
    }

    fn photo(secret: bool) -> String {
        format!(
            r#"{{"@type":"messagePhoto","photo":{{"@type":"photo","has_stickers":false,"sizes":[
            {{"@type":"photoSize","type":"m","photo":{},"width":320,"height":240,"progressive_sizes":[]}},
            {{"@type":"photoSize","type":"x","photo":{},"width":800,"height":600,"progressive_sizes":[]}},
            {{"@type":"photoSize","type":"y","photo":{},"width":1280,"height":960,"progressive_sizes":[]}},
            {{"@type":"photoSize","type":"w","photo":{},"width":2560,"height":1920,"progressive_sizes":[]}}]}},
            "caption":{{"@type":"formattedText","text":"the view","entities":[]}},"has_spoiler":false,"is_secret":{secret}}}"#,
            file(1, 10_000),
            file(2, 60_000),
            file(3, 150_000),
            file(4, 600_000)
        )
    }

    #[test]
    fn a_photo_is_described_by_its_largest_size_and_fetched_in_the_one_that_fits() {
        let message = message(&photo(false));
        let media = media_json(&message.content).expect("media");
        assert_eq!((media["kind"].as_str(), media["file_id"].as_i64(), media["width"].as_i64()), (Some("photo"), Some(4), Some(2560)));
        let M::Photo { photo, .. } = &message.content else { panic!("a photo") };
        assert_eq!(fitting(photo).map(|size| size.photo.id), Some(3));
        let target = media_file(&message.content).expect("a file");
        assert!((target.file_id, target.kind, target.picture) == (3, "photo", true) && target.mime_type == "image/jpeg");
        assert_eq!(text_of(&message.content), Some("the view".to_string()));
    }

    #[test]
    fn self_destructing_media_keeps_its_file_id_to_itself() {
        let message = message(&photo(true));
        let media = media_json(&message.content).expect("media");
        assert!(media["file_id"].is_null());
        assert!(store::is_secret(&message.content));
    }

    #[test]
    fn links_are_cut_by_utf16_units_and_only_hidden_ones_are_listed() {
        let text = api::FormattedText {
            text: "😀 see docs and @alice".to_string(),
            entities: vec![
                api::TextEntity { offset: 7, length: 4, kind: TextEntityType::TextUrl { url: "https://example.org/docs".into() } },
                api::TextEntity { offset: 16, length: 6, kind: TextEntityType::Mention },
                api::TextEntity { offset: 3, length: 3, kind: TextEntityType::Bold },
            ],
        };
        let links = links_of(&text);
        assert_eq!(links, vec![json!({ "text": "docs", "url": "https://example.org/docs" })]);
        assert_eq!(utf16_slice("😀 see", 0, 2), "😀");
        assert_eq!(utf16_slice("short", 3, 100), "rt");
        assert_eq!(utf16_slice("short", -1, 2), "sh");
    }

    #[test]
    fn kinds_of_content_without_words_are_named_and_a_text_has_no_media() {
        let text = message(r#"{"@type":"messageText","text":{"@type":"formattedText","text":"hi","entities":[]}}"#);
        assert_eq!(media_json(&text.content), None);
        assert_eq!(text_of(&text.content), Some("hi".to_string()));
        let dice = message(r#"{"@type":"messageDice","emoji":"🎲","value":3}"#);
        assert_eq!(text_of(&dice.content), Some("🎲".to_string()));
        let voice = message(r#"{"@type":"messageVoiceNote","caption":{"@type":"formattedText","text":"","entities":[]},"is_listened":false}"#);
        assert_eq!(media_json(&voice.content).map(|media| media["kind"].to_string()), Some("\"voice_note\"".to_string()));
        assert!(media_file(&voice.content).is_none());
        let gone = message(r#"{"@type":"messageExpiredPhoto"}"#);
        assert_eq!(media_json(&gone.content).map(|media| media["kind"].to_string()), Some("\"expired\"".to_string()));
    }

    #[test]
    fn a_moment_is_local_time_with_its_offset() {
        let moment = iso(1_760_000_000);
        assert_eq!(moment.len(), 25, "{moment}");
        assert_eq!(moment.as_bytes()[10], b'T');
    }

    #[test]
    fn gather_finishes_once_in_the_order_asked() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let gather = Gather::new(3, {
            let seen = seen.clone();
            move |results: Vec<&str>| seen.borrow_mut().push(results)
        });
        Gather::done(&gather, 2, "c");
        Gather::done(&gather, 0, "a");
        assert!(seen.borrow().is_empty());
        Gather::done(&gather, 1, "b");
        assert_eq!(seen.borrow().as_slice(), &[vec!["a", "b", "c"]]);
    }
}
