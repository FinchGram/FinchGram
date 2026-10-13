//! The tools the MCP server offers an assistant: each one's name, what it does and its arguments,
//! as the assistant reads them (tools/list), and the typed call a tools/call becomes. What a call
//! does with Telegram is src/telegram/tools.rs, on the UI thread; the two sides agree through
//! [`ToolCall`], so a tool cannot be offered without being done, or done otherwise than described.

use serde_json::{Value, json};

/// Telegram's longest message.
pub const TEXT_MAX: usize = 4096;
/// How many of a kind one call gives at most, and how many when the assistant does not say.
pub const CHATS_MAX: usize = 200;
pub const CHATS_DEFAULT: usize = 50;
pub const FOUND_CHATS_MAX: usize = 50;
pub const FOUND_CHATS_DEFAULT: usize = 10;
pub const MESSAGES_MAX: usize = 100;
pub const MESSAGES_DEFAULT: usize = 30;
pub const FOUND_MESSAGES_MAX: usize = 100;
pub const FOUND_MESSAGES_DEFAULT: usize = 20;
pub const UNREAD_CHATS_MAX: usize = 50;
pub const UNREAD_CHATS_DEFAULT: usize = 20;
pub const UNREAD_PER_CHAT_MAX: usize = 20;
pub const UNREAD_PER_CHAT_DEFAULT: usize = 5;
pub const MARK_READ_MAX: usize = 100;

/// What a tool gives back: text and pictures for the assistant, or why it was not done, which the
/// assistant reads as the tool's error.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Done(Vec<Content>),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Text(String),
    /// A picture, base64, with its media type ("image/jpeg").
    Image { data: String, mime_type: String },
}

impl Outcome {
    pub fn text(text: impl Into<String>) -> Outcome {
        Outcome::Done(vec![Content::Text(text.into())])
    }

    /// `value` as indented JSON.
    pub fn json(value: &Value) -> Outcome {
        Outcome::text(serde_json::to_string_pretty(value).unwrap_or_default())
    }

    pub fn failed(reason: impl Into<String>) -> Outcome {
        Outcome::Failed(reason.into())
    }
}

/// What a call did, for Settings → AI access's log (access.rs): which kind of thing, which chat,
/// how many. Never the words of a message or a search.
#[derive(Debug, Clone, PartialEq)]
pub struct Activity {
    pub kind: ActivityKind,
    /// The chat it was about, by title, where there is one.
    pub chat: String,
    /// How many messages or chats, where it matters.
    pub count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActivityKind {
    /// An app connected, or was turned away because the user has not allowed it.
    Connected,
    TurnedAway,
    Listed,
    SearchedChats,
    Looked,
    Read,
    Searched,
    SearchedAll,
    Unread,
    Media,
    Draft,
    Sent,
    Deleted,
    Marked,
    Account,
    /// A chat outside the ones the user chose was asked for.
    Refused,
}

impl Activity {
    pub fn new(kind: ActivityKind) -> Activity {
        Activity { kind, chat: String::new(), count: 0 }
    }

    pub fn about(kind: ActivityKind, chat: impl Into<String>) -> Activity {
        Activity { kind, chat: chat.into(), count: 0 }
    }

    pub fn counted(kind: ActivityKind, chat: impl Into<String>, count: usize) -> Activity {
        Activity { kind, chat: chat.into(), count }
    }
}

/// A call of one of the tools, its arguments checked and within bounds.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolCall {
    ListChats { limit: usize, unread_only: bool },
    SearchChats { query: String, limit: usize },
    GetChat { chat_id: i64 },
    GetMessages { chat_id: i64, limit: usize, before_message_id: i64 },
    SearchMessages { query: String, chat_id: Option<i64>, limit: usize },
    GetUnread { chats_limit: usize, messages_per_chat: usize },
    DownloadMedia { chat_id: i64, message_id: i64 },
    DraftMessage { chat_id: i64, text: String, reply_to_message_id: Option<i64> },
    SendMessage { chat_id: i64, text: String, reply_to_message_id: Option<i64> },
    DeleteMessage { chat_id: i64, message_id: i64 },
    MarkRead { chat_id: i64, message_ids: Vec<i64> },
    GetMe,
}

impl ToolCall {
    pub fn name(&self) -> &'static str {
        match self {
            ToolCall::ListChats { .. } => "list_chats",
            ToolCall::SearchChats { .. } => "search_chats",
            ToolCall::GetChat { .. } => "get_chat",
            ToolCall::GetMessages { .. } => "get_messages",
            ToolCall::SearchMessages { .. } => "search_messages",
            ToolCall::GetUnread { .. } => "get_unread",
            ToolCall::DownloadMedia { .. } => "download_media",
            ToolCall::DraftMessage { .. } => "draft_message",
            ToolCall::SendMessage { .. } => "send_message",
            ToolCall::DeleteMessage { .. } => "delete_message",
            ToolCall::MarkRead { .. } => "mark_read",
            ToolCall::GetMe => "get_me",
        }
    }

    /// The call for the log: which tool, which chat, how much. Never the words of a message or a
    /// search.
    pub fn summary(&self) -> String {
        let chars = |text: &str| text.chars().count();
        let details = match self {
            ToolCall::ListChats { limit, unread_only } => format!("limit={limit} unread_only={unread_only}"),
            ToolCall::SearchChats { query, limit } => format!("({} chars) limit={limit}", chars(query)),
            ToolCall::GetChat { chat_id } => format!("chat={chat_id}"),
            ToolCall::GetMessages { chat_id, limit, before_message_id } => format!("chat={chat_id} limit={limit} before={before_message_id}"),
            ToolCall::SearchMessages { query, chat_id, limit } => format!(
                "({} chars) chat={} limit={limit}",
                chars(query),
                chat_id.map_or_else(|| "all".to_string(), |id| id.to_string())
            ),
            ToolCall::GetUnread { chats_limit, messages_per_chat } => format!("chats={chats_limit} per_chat={messages_per_chat}"),
            ToolCall::DownloadMedia { chat_id, message_id } => format!("chat={chat_id} message={message_id}"),
            ToolCall::DraftMessage { chat_id, text, reply_to_message_id } | ToolCall::SendMessage { chat_id, text, reply_to_message_id } => {
                format!("chat={chat_id} ({} chars) reply_to={}", chars(text), reply_to_message_id.unwrap_or(0))
            }
            ToolCall::DeleteMessage { chat_id, message_id } => format!("chat={chat_id} message={message_id}"),
            ToolCall::MarkRead { chat_id, message_ids } => format!("chat={chat_id} messages={}", message_ids.len()),
            ToolCall::GetMe => String::new(),
        };
        if details.is_empty() { self.name().to_string() } else { format!("{} {details}", self.name()) }
    }
}

/// The tools as tools/list gives them: name, title, what each does, its arguments as a JSON
/// Schema, and what kind of thing it is (read-only, or not).
pub fn definitions() -> Vec<Value> {
    let count = |what: &str, default: usize, max: usize| {
        json!({ "type": "integer", "description": format!("{what} Default {default}, at most {max}."), "minimum": 1, "maximum": max, "default": default })
    };
    let integer = |what: &str| json!({ "type": "integer", "description": what });
    let text = |what: &str| json!({ "type": "string", "description": what });
    let read_only = json!({ "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false });
    let markdown = "Telegram's Markdown: **bold**, __italic__, ~~strikethrough~~, `code`, ```pre```, [text](url).";
    vec![
        tool(
            "list_chats",
            "List chats",
            "The user's chats, the most recently active first, as FinchGram's chat list shows them: each chat's id, title, kind (private, bot, group, channel, saved_messages), unread count and last message. The ids are what the other tools take. Set unread_only to see only chats with unread messages.",
            json!({
                "limit": count("How many chats.", CHATS_DEFAULT, CHATS_MAX),
                "unread_only": { "type": "boolean", "description": "Only chats with unread messages.", "default": false },
            }),
            &[],
            read_only.clone(),
        ),
        tool(
            "search_chats",
            "Search chats",
            "Find chats among the user's own by title or @username (not the whole of Telegram). Gives the same fields as list_chats.",
            json!({
                "query": text("Part of a title, or a @username."),
                "limit": count("How many chats.", FOUND_CHATS_DEFAULT, FOUND_CHATS_MAX),
            }),
            &["query"],
            read_only.clone(),
        ),
        tool(
            "get_chat",
            "Get a chat",
            "One chat by id: title, kind, username and member count where known, unread count, whether it is muted or pinned.",
            json!({ "chat_id": integer("The chat's id, from list_chats or search_chats.") }),
            &["chat_id"],
            read_only.clone(),
        ),
        tool(
            "get_messages",
            "Read messages",
            "The newest messages of a chat, newest first, with sender, time, text, links and any media (photo, video, file, …). Reading marks nothing as read. For older messages, pass before_message_id: the id of the oldest message you have.",
            json!({
                "chat_id": integer("The chat's id."),
                "limit": count("How many messages.", MESSAGES_DEFAULT, MESSAGES_MAX),
                "before_message_id": integer("Only messages older than this one; leave out for the newest."),
            }),
            &["chat_id"],
            read_only.clone(),
        ),
        tool(
            "search_messages",
            "Search messages",
            "Find messages by their words: in one chat when chat_id is given, otherwise across the chats the user allows. Telegram's servers search, by whole words. Newest first.",
            json!({
                "query": text("The words to find."),
                "chat_id": integer("Search one chat only."),
                "limit": count("How many messages.", FOUND_MESSAGES_DEFAULT, FOUND_MESSAGES_MAX),
            }),
            &["query"],
            read_only.clone(),
        ),
        tool(
            "get_unread",
            "What is unread",
            "The chats with unread messages, the most recently active first, each with its newest unread messages. Marks nothing as read.",
            json!({
                "chats_limit": count("How many chats.", UNREAD_CHATS_DEFAULT, UNREAD_CHATS_MAX),
                "messages_per_chat": count("How many of each chat's unread messages, the newest.", UNREAD_PER_CHAT_DEFAULT, UNREAD_PER_CHAT_MAX),
            }),
            &[],
            read_only.clone(),
        ),
        tool(
            "download_media",
            "Get a message's media",
            "The photo, sticker, video, GIF or file of a message. A photo or a still sticker comes as the picture itself; anything else is downloaded and its path on this Mac given. Not for self-destructing media, nor in chats that restrict saving content.",
            json!({
                "chat_id": integer("The chat's id."),
                "message_id": integer("The message's id, from get_messages or search_messages."),
            }),
            &["chat_id", "message_id"],
            read_only,
        ),
        tool(
            "draft_message",
            "Write a draft",
            &format!("Put a draft into a chat for the user to look at and send themselves, in FinchGram or any Telegram app (drafts follow the account). Use this rather than send_message unless the user asked to send right away. {markdown}"),
            json!({
                "chat_id": integer("The chat's id."),
                "text": text("The draft."),
                "reply_to_message_id": integer("Make it a reply to this message."),
            }),
            &["chat_id", "text"],
            json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }),
        ),
        tool(
            "send_message",
            "Send a message",
            &format!("Send a message to a chat under the user's name, at once and for good. Only when the user asked to send, with wording they have seen or dictated. {markdown}"),
            json!({
                "chat_id": integer("The chat's id."),
                "text": text("The message."),
                "reply_to_message_id": integer("Make it a reply to this message."),
            }),
            &["chat_id", "text"],
            json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": true }),
        ),
        tool(
            "delete_message",
            "Delete a message",
            "Delete a message the user's account sent (their own, or one an assistant sent under their name), for everyone, where Telegram allows it: at any time in private chats, within 48 hours in groups. Never someone else's message. Only when the user asked for it.",
            json!({
                "chat_id": integer("The chat's id."),
                "message_id": integer("The message's id, from get_messages or from send_message's answer."),
            }),
            &["chat_id", "message_id"],
            json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": true, "openWorldHint": true }),
        ),
        tool(
            "mark_read",
            "Mark as read",
            "Mark messages of a chat as read, which their senders can see. Only when the user asks for it; reading with get_messages never does this.",
            json!({
                "chat_id": integer("The chat's id."),
                "message_ids": { "type": "array", "items": { "type": "integer" }, "description": format!("The messages to mark, at most {MARK_READ_MAX}."), "minItems": 1, "maxItems": MARK_READ_MAX },
            }),
            &["chat_id", "message_ids"],
            json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": true }),
        ),
        tool(
            "get_me",
            "Whose account",
            "The signed-in account: id, name and username.",
            json!({}),
            &[],
            json!({ "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }),
        ),
    ]
}

fn tool(name: &str, title: &str, description: &str, properties: Value, required: &[&str], annotations: Value) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required },
        "annotations": annotations,
    })
}

/// The call a tools/call asks for, or what is wrong with it.
pub fn parse(name: &str, arguments: &Value) -> Result<ToolCall, String> {
    let args = Arguments(arguments);
    Ok(match name {
        "list_chats" => ToolCall::ListChats {
            limit: args.count("limit", CHATS_DEFAULT, CHATS_MAX)?,
            unread_only: args.flag("unread_only")?,
        },
        "search_chats" => ToolCall::SearchChats {
            query: args.words("query")?,
            limit: args.count("limit", FOUND_CHATS_DEFAULT, FOUND_CHATS_MAX)?,
        },
        "get_chat" => ToolCall::GetChat { chat_id: args.id("chat_id")? },
        "get_messages" => ToolCall::GetMessages {
            chat_id: args.id("chat_id")?,
            limit: args.count("limit", MESSAGES_DEFAULT, MESSAGES_MAX)?,
            before_message_id: args.integer("before_message_id")?.unwrap_or(0),
        },
        "search_messages" => ToolCall::SearchMessages {
            query: args.words("query")?,
            chat_id: args.integer("chat_id")?,
            limit: args.count("limit", FOUND_MESSAGES_DEFAULT, FOUND_MESSAGES_MAX)?,
        },
        "get_unread" => ToolCall::GetUnread {
            chats_limit: args.count("chats_limit", UNREAD_CHATS_DEFAULT, UNREAD_CHATS_MAX)?,
            messages_per_chat: args.count("messages_per_chat", UNREAD_PER_CHAT_DEFAULT, UNREAD_PER_CHAT_MAX)?,
        },
        "download_media" => ToolCall::DownloadMedia { chat_id: args.id("chat_id")?, message_id: args.id("message_id")? },
        "draft_message" => ToolCall::DraftMessage {
            chat_id: args.id("chat_id")?,
            text: args.message("text")?,
            reply_to_message_id: args.integer("reply_to_message_id")?.filter(|id| *id != 0),
        },
        "send_message" => ToolCall::SendMessage {
            chat_id: args.id("chat_id")?,
            text: args.message("text")?,
            reply_to_message_id: args.integer("reply_to_message_id")?.filter(|id| *id != 0),
        },
        "delete_message" => ToolCall::DeleteMessage { chat_id: args.id("chat_id")?, message_id: args.id("message_id")? },
        "mark_read" => ToolCall::MarkRead { chat_id: args.id("chat_id")?, message_ids: args.ids("message_ids")? },
        "get_me" => ToolCall::GetMe,
        other => return Err(format!("Unknown tool: {other}")),
    })
}

/// The arguments of a call, read leniently: a number may come as digits in a string, a flag as
/// "true", and anything not asked for is ignored.
struct Arguments<'a>(&'a Value);

impl Arguments<'_> {
    fn given(&self, key: &str) -> Option<&Value> {
        self.0.get(key).filter(|value| !value.is_null())
    }

    fn integer(&self, key: &str) -> Result<Option<i64>, String> {
        let Some(value) = self.given(key) else { return Ok(None) };
        whole_number(value).map(Some).ok_or_else(|| format!("{key} must be a whole number"))
    }

    fn id(&self, key: &str) -> Result<i64, String> {
        self.integer(key)?.ok_or_else(|| format!("{key} is required"))
    }

    /// A number of things, within 1 … `max`; `default` when not given.
    fn count(&self, key: &str, default: usize, max: usize) -> Result<usize, String> {
        Ok(match self.integer(key)? {
            None => default,
            Some(n) => usize::try_from(n.max(1)).unwrap_or(max).min(max),
        })
    }

    fn flag(&self, key: &str) -> Result<bool, String> {
        match self.given(key) {
            None => Ok(false),
            Some(Value::Bool(flag)) => Ok(*flag),
            Some(Value::String(word)) if word == "true" || word == "false" => Ok(word == "true"),
            Some(_) => Err(format!("{key} must be true or false")),
        }
    }

    /// Words to look for: required, not blank.
    fn words(&self, key: &str) -> Result<String, String> {
        let words = self.given(key).and_then(Value::as_str).map(str::trim).unwrap_or_default();
        if words.is_empty() {
            return Err(format!("{key} is required"));
        }
        Ok(words.to_string())
    }

    /// A message's text: required, not blank, no longer than Telegram allows.
    fn message(&self, key: &str) -> Result<String, String> {
        let text = self.given(key).and_then(Value::as_str).ok_or_else(|| format!("{key} is required"))?;
        if text.trim().is_empty() {
            return Err(format!("{key} must not be empty"));
        }
        if text.chars().count() > TEXT_MAX {
            return Err(format!("{key} is longer than Telegram's {TEXT_MAX} characters"));
        }
        Ok(text.to_string())
    }

    fn ids(&self, key: &str) -> Result<Vec<i64>, String> {
        let Some(Value::Array(items)) = self.given(key) else { return Err(format!("{key} must be a list of ids")) };
        if items.is_empty() {
            return Err(format!("{key} must not be empty"));
        }
        if items.len() > MARK_READ_MAX {
            return Err(format!("{key} may hold at most {MARK_READ_MAX} ids"));
        }
        items.iter().map(|item| whole_number(item).ok_or_else(|| format!("{key} must hold whole numbers"))).collect()
    }
}

fn whole_number(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().filter(|f| f.fract() == 0.0 && f.abs() < 9.0e15).map(|f| f as i64)),
        Value::String(digits) => digits.trim().parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_defined_tool_parses_and_is_named_the_same() {
        let defined: Vec<String> = definitions().iter().map(|tool| tool["name"].as_str().expect("a name").to_string()).collect();
        assert_eq!(defined.len(), 12);
        let examples = json!({
            "chat_id": 42, "message_id": 7, "query": "hello", "text": "hi", "message_ids": [1, 2],
        });
        for name in &defined {
            let call = parse(name, &examples).unwrap_or_else(|err| panic!("{name}: {err}"));
            assert_eq!(call.name(), name);
            assert!(call.summary().starts_with(name.as_str()), "{}", call.summary());
        }
        assert_eq!(parse("delete_chat", &json!({})), Err("Unknown tool: delete_chat".to_string()));
    }

    #[test]
    fn counts_have_defaults_and_bounds_and_ids_may_be_strings() {
        assert_eq!(parse("list_chats", &json!({})), Ok(ToolCall::ListChats { limit: CHATS_DEFAULT, unread_only: false }));
        assert_eq!(
            parse("list_chats", &json!({ "limit": 1000, "unread_only": "true" })),
            Ok(ToolCall::ListChats { limit: CHATS_MAX, unread_only: true })
        );
        assert_eq!(parse("get_messages", &json!({ "chat_id": "-1001234", "limit": 0 })), Ok(ToolCall::GetMessages {
            chat_id: -1001234,
            limit: 1,
            before_message_id: 0,
        }));
        assert_eq!(parse("get_messages", &json!({ "chat_id": 12.0 })).map(|call| call.name()), Ok("get_messages"));
        assert_eq!(parse("get_messages", &json!({ "chat_id": "twelve" })), Err("chat_id must be a whole number".to_string()));
        assert_eq!(parse("get_messages", &json!({})), Err("chat_id is required".to_string()));
        assert_eq!(parse("list_chats", &json!({ "unread_only": 1 })), Err("unread_only must be true or false".to_string()));
    }

    #[test]
    fn words_and_messages_must_be_there_and_messages_within_telegrams_length() {
        assert_eq!(parse("search_chats", &json!({ "query": "  " })), Err("query is required".to_string()));
        assert_eq!(
            parse("search_messages", &json!({ "query": " rust " })),
            Ok(ToolCall::SearchMessages { query: "rust".into(), chat_id: None, limit: FOUND_MESSAGES_DEFAULT })
        );
        assert_eq!(parse("send_message", &json!({ "chat_id": 1, "text": " " })), Err("text must not be empty".to_string()));
        let long = "x".repeat(TEXT_MAX + 1);
        assert_eq!(
            parse("send_message", &json!({ "chat_id": 1, "text": long })),
            Err(format!("text is longer than Telegram's {TEXT_MAX} characters"))
        );
        assert_eq!(
            parse("draft_message", &json!({ "chat_id": 1, "text": "hi", "reply_to_message_id": 0 })),
            Ok(ToolCall::DraftMessage { chat_id: 1, text: "hi".into(), reply_to_message_id: None })
        );
        assert_eq!(parse("mark_read", &json!({ "chat_id": 1, "message_ids": [] })), Err("message_ids must not be empty".to_string()));
        assert_eq!(parse("mark_read", &json!({ "chat_id": 1, "message_ids": 5 })), Err("message_ids must be a list of ids".to_string()));
    }

    #[test]
    fn the_summary_tells_how_much_but_never_the_words() {
        let call = parse("send_message", &json!({ "chat_id": 9, "text": "a secret plan" })).expect("parses");
        let summary = call.summary();
        assert!(summary.contains("chat=9") && summary.contains("(13 chars)") && !summary.contains("secret"), "{summary}");
        let call = parse("search_messages", &json!({ "query": "secret" })).expect("parses");
        assert!(!call.summary().contains("secret"));
    }
}
