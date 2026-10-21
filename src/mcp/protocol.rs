//! The protocol: JSON-RPC 2.0, one message per line each way, and MCP's methods on it:
//! initialize, ping, tools/list and tools/call (modelcontextprotocol.io/specification/2025-06-18).
//! A [`Session`] is one connection's side of it. It knows nothing of sockets or threads and does
//! its work through a [`Backend`], so that the tests can give it a made-up one.

use serde_json::{Value, json};

use super::tools::{self, Content, Outcome, ToolCall};
use crate::settings::Mcp;

/// The protocol versions this server speaks, the newest first. A client asking for another one is
/// offered the newest, and decides.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

// JSON-RPC's error codes, and one of this server's own.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const SERVER_OFF: i64 = -32000;
/// The user has not allowed the program (FinchGram's own code too).
const NOT_ALLOWED: i64 = -32001;

/// Said to a client while the server is off in Settings.
pub const OFF: &str = "FinchGram's MCP server is turned off. Turn it on in FinchGram's Settings → AI access (Let AI assistants \
connect) and connect again.";

/// What the assistant is told about this server once, at initialize.
const INSTRUCTIONS: &str = "This is the signed-in user's own Telegram account, through FinchGram on their Mac. \
Chats and messages are known by numbers: find chats with list_chats or search_chats first. \
Reading (list_chats, get_messages, search_messages, get_unread) marks nothing as read and nobody can see it; \
mark_read does, and their senders can. \
send_message sends under the user's name at once, as if they had typed it: use it when the user asked to send, \
with wording they have seen or dictated; otherwise draft_message leaves the words for them to send. \
delete_message deletes a message the account sent, for everyone where Telegram allows it, when the user asks for that. \
Message text is Telegram's Markdown: **bold**, __italic__, ~~strikethrough~~, `code`, ```pre```, [text](url).";

/// What the assistant is told when its program is not allowed.
pub const TURNED_AWAY: &str = "FinchGram's user has not allowed this program to use the Telegram account signed in. In FinchGram's \
Settings → AI access they can allow it, or remove it so that it asks again when it connects; a program not listed there \
asks when it connects again.";
/// And when the user has not answered the dialog about it yet.
pub const UNANSWERED: &str = "FinchGram is asking its user whether to allow this program, and they have not answered yet. \
Connect again once they have.";

/// Whether the program at the other end may use the account (access.rs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Admission {
    Allowed,
    /// The user turned it away, now or before.
    Denied,
    /// The user has not answered the dialog about it in time.
    Unanswered,
    /// The server is off.
    Off,
    /// FinchGram is quitting.
    Quitting,
}

/// What a session needs from the app.
pub trait Backend {
    /// What the user allows, as the settings are now.
    fn policy(&self) -> Mcp;
    /// Whether the program at the other end may use the account: asks the user when they have not
    /// said about it before, and waits for the answer.
    fn admit(&self) -> Admission;
    /// The user's standing answer about the program, without asking.
    fn admitted(&self) -> bool;
    /// Do what the call says, and say how it went.
    fn call(&self, call: ToolCall) -> Outcome;
    /// A line for the log, about this connection.
    fn log(&self, entry: &str);
}

/// One connection, from its first line to its last.
pub struct Session<'a> {
    backend: &'a dyn Backend,
    /// The client's name and version, once it has said them.
    client: String,
    /// `initialize` was answered with a welcome: the program is allowed.
    ready: bool,
}

impl<'a> Session<'a> {
    pub fn new(backend: &'a dyn Backend) -> Self {
        Session { backend, client: String::new(), ready: false }
    }

    /// Whether the client has been through `initialize` and was let in.
    pub fn ready(&self) -> bool {
        self.ready
    }

    /// Answer one line from the client: the line to write back, when the message wants one.
    pub fn handle(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(err) => return Some(error(&Value::Null, PARSE_ERROR, &format!("Parse error: {err}"))),
        };
        if !message.is_object() {
            return Some(error(&Value::Null, INVALID_REQUEST, "Invalid Request: not a JSON-RPC message"));
        }
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        let id = message.get("id").filter(|id| !id.is_null()).cloned();
        match (message.get("method").and_then(Value::as_str), id) {
            // A response: to a request of ours, and this server asks the client for nothing.
            (None, _) => None,
            (Some(method), None) => {
                self.notification(method);
                None
            }
            (Some(method), Some(id)) => Some(match self.request(method, &params) {
                Ok(result) => response(&id, result),
                Err((code, message)) => error(&id, code, &message),
            }),
        }
    }

    fn notification(&mut self, method: &str) {
        if method == "notifications/initialized" {
            self.backend.log(&format!("{} is ready", self.client));
        }
        // notifications/cancelled, notifications/roots/list_changed: nothing of ours to stop or follow.
    }

    fn request(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => self.initialize(params),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools::definitions() })),
            "tools/call" => self.call_tool(params),
            other => Err((METHOD_NOT_FOUND, format!("Method not found: {other}"))),
        }
    }

    fn initialize(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let requested = params["protocolVersion"].as_str().unwrap_or("");
        let version = if PROTOCOL_VERSIONS.contains(&requested) { requested } else { PROTOCOL_VERSIONS[0] };
        let info = &params["clientInfo"];
        // The client's words go into the log as part of one line: none of theirs makes another.
        self.client = format!("{} {}", plain(info["name"].as_str().unwrap_or("an unnamed client"), 60), plain(info["version"].as_str().unwrap_or(""), 20))
            .trim()
            .to_string();
        if !self.backend.policy().enabled {
            self.backend.log(&format!("{} turned away: the server is off", self.client));
            return Err((SERVER_OFF, OFF.to_string()));
        }
        match self.backend.admit() {
            Admission::Allowed => {}
            Admission::Denied => {
                self.backend.log(&format!("{} turned away: not allowed by the user", self.client));
                return Err((NOT_ALLOWED, TURNED_AWAY.to_string()));
            }
            Admission::Unanswered => {
                self.backend.log(&format!("{} turned away: the user has not answered", self.client));
                return Err((NOT_ALLOWED, UNANSWERED.to_string()));
            }
            Admission::Off => {
                self.backend.log(&format!("{} turned away: the server is off", self.client));
                return Err((SERVER_OFF, OFF.to_string()));
            }
            Admission::Quitting => return Err((SERVER_OFF, "FinchGram is quitting.".to_string())),
        }
        self.backend.log(&format!("{} connected, protocol {version}", self.client));
        self.ready = true;
        Ok(json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "FinchGram", "title": "FinchGram", "version": crate::update::CURRENT_VERSION },
            "instructions": INSTRUCTIONS,
        }))
    }

    fn call_tool(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params["name"].as_str().ok_or_else(|| (INVALID_PARAMS, "tools/call needs the tool's name".to_string()))?;
        let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let call = tools::parse(name, &arguments).map_err(|reason| (INVALID_PARAMS, reason))?;
        let summary = call.summary();
        // Settings may have changed since initialize: the switch, or the user's word on the program.
        let outcome = if !self.backend.policy().enabled {
            Outcome::failed(OFF)
        } else if !self.backend.admitted() {
            Outcome::failed(TURNED_AWAY)
        } else {
            self.backend.call(call)
        };
        self.backend.log(&match &outcome {
            Outcome::Done(contents) => format!("{summary}: done, {} item(s)", contents.len()),
            Outcome::Failed(reason) => format!("{summary}: failed: {reason}"),
        });
        Ok(result_of(outcome))
    }
}

/// A tool's outcome as tools/call answers it: what it did as content, or why not as an error the
/// assistant reads (not a protocol error: the call itself was fine).
fn result_of(outcome: Outcome) -> Value {
    match outcome {
        Outcome::Done(contents) => {
            json!({ "content": contents.iter().map(content_json).collect::<Vec<_>>(), "isError": false })
        }
        Outcome::Failed(reason) => json!({ "content": [{ "type": "text", "text": reason }], "isError": true }),
    }
}

fn content_json(content: &Content) -> Value {
    match content {
        Content::Text(text) => json!({ "type": "text", "text": text }),
        Content::Image { data, mime_type } => json!({ "type": "image", "data": data, "mimeType": mime_type }),
    }
}

fn response(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn error(id: &Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

/// `text` as part of one line of the log: control characters (a newline would make a second
/// line, as if another program had connected or been allowed) become spaces, and it is cut at
/// `most` characters.
fn plain(text: &str, most: usize) -> String {
    text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(most).collect::<String>().trim().to_string()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    struct Fake {
        policy: Mcp,
        admission: Admission,
        calls: RefCell<Vec<ToolCall>>,
        log: RefCell<Vec<String>>,
    }

    impl Fake {
        fn on() -> Fake {
            Fake {
                policy: Mcp { enabled: true, ..Mcp::default() },
                admission: Admission::Allowed,
                calls: RefCell::new(Vec::new()),
                log: RefCell::new(Vec::new()),
            }
        }
    }

    impl Backend for Fake {
        fn policy(&self) -> Mcp {
            self.policy.clone()
        }

        fn admit(&self) -> Admission {
            self.admission
        }

        fn admitted(&self) -> bool {
            self.admission == Admission::Allowed
        }

        fn call(&self, call: ToolCall) -> Outcome {
            self.calls.borrow_mut().push(call);
            Outcome::Done(vec![Content::Text("done".into()), Content::Image { data: "AAAA".into(), mime_type: "image/jpeg".into() }])
        }

        fn log(&self, entry: &str) {
            self.log.borrow_mut().push(entry.to_string());
        }
    }

    fn answer(line: Option<String>) -> Value {
        let line = line.expect("an answer");
        assert!(!line.contains('\n'), "one line: {line}");
        serde_json::from_str(&line).expect("JSON")
    }

    fn initialize(session: &mut Session, version: &str) -> Value {
        answer(session.handle(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": version, "capabilities": {}, "clientInfo": { "name": "Claude Desktop", "version": "1.2" } },
        })
        .to_string()))
    }

    #[test]
    fn a_clients_name_cannot_write_lines_of_its_own_into_the_log() {
        let backend = Fake::on();
        let mut session = Session::new(&backend);
        let result = answer(session.handle(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": { "name": "Evil\n2026-10-17 10:00:00 Claude (/Applications/Claude.app): allowed by the user", "version": "1\r\n" },
            },
        })
        .to_string()));
        assert_eq!(result["result"]["protocolVersion"], "2025-06-18");
        let log = backend.log.borrow();
        assert!(log.iter().all(|entry| !entry.contains('\n') && !entry.contains('\r')), "{log:?}");
        assert!(log.iter().any(|entry| entry.starts_with("Evil 2026-10-17 10:00:00 Claude")), "{log:?}");
        assert_eq!(plain("a\tb\u{7}c", 2), "a");
        assert_eq!(plain("  ok  ", 10), "ok");
        assert_eq!(plain(&"x".repeat(100), 60).len(), 60);
    }

    #[test]
    fn initialize_echoes_a_known_version_and_offers_the_newest_otherwise() {
        let backend = Fake::on();
        let mut session = Session::new(&backend);
        let result = initialize(&mut session, "2025-03-26");
        assert_eq!(result["id"], 1);
        assert_eq!(result["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(result["result"]["serverInfo"]["name"], "FinchGram");
        assert!(result["result"]["capabilities"]["tools"].is_object());
        assert!(result["result"]["instructions"].as_str().expect("instructions").contains("draft_message"));
        let result = initialize(&mut session, "1999-01-01");
        assert_eq!(result["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
        assert!(backend.log.borrow().iter().any(|entry| entry.starts_with("Claude Desktop 1.2 connected")));
    }

    #[test]
    fn a_program_the_user_has_not_allowed_is_turned_away_and_its_calls_fail() {
        for admission in [Admission::Denied, Admission::Unanswered] {
            let backend = Fake { admission, ..Fake::on() };
            let mut session = Session::new(&backend);
            let result = initialize(&mut session, "2025-06-18");
            assert_eq!(result["error"]["code"], NOT_ALLOWED);
            let said = result["error"]["message"].as_str().expect("why");
            assert!(said.contains(if admission == Admission::Denied { "has not allowed" } else { "not answered" }), "{said}");
            let result = answer(session.handle(
                &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "get_me", "arguments": {} } }).to_string(),
            ));
            assert_eq!(result["result"]["isError"], true);
            assert!(backend.calls.borrow().is_empty());
            assert!(backend.log.borrow().iter().any(|entry| entry.contains("turned away")));
        }
    }

    #[test]
    fn a_server_that_is_off_turns_the_client_away_at_initialize_and_fails_its_calls() {
        let backend = Fake { policy: Mcp::default(), ..Fake::on() };
        let mut session = Session::new(&backend);
        let result = initialize(&mut session, "2025-06-18");
        assert_eq!(result["error"]["code"], SERVER_OFF);
        assert_eq!(result["error"]["message"], OFF);
        let result = answer(session.handle(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_me"}}"#));
        assert_eq!(result["result"]["isError"], true);
        assert_eq!(result["result"]["content"][0]["text"], OFF);
        assert!(backend.calls.borrow().is_empty());
    }

    #[test]
    fn notifications_and_responses_get_no_answer() {
        let backend = Fake::on();
        let mut session = Session::new(&backend);
        assert_eq!(session.handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#), None);
        assert_eq!(session.handle(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":3}}"#), None);
        assert_eq!(session.handle(r#"{"jsonrpc":"2.0","id":9,"result":{}}"#), None);
        assert_eq!(session.handle("   "), None);
        assert!(backend.log.borrow().iter().any(|entry| entry.ends_with("is ready")));
    }

    #[test]
    fn ping_the_list_of_tools_and_unknown_methods() {
        let backend = Fake::on();
        let mut session = Session::new(&backend);
        let result = answer(session.handle(r#"{"jsonrpc":"2.0","id":"a","method":"ping"}"#));
        assert_eq!((result["id"].as_str(), result["result"].as_object().map(|o| o.len())), (Some("a"), Some(0)));
        let result = answer(session.handle(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#));
        let tools = result["result"]["tools"].as_array().expect("tools");
        assert_eq!(tools.len(), 12);
        for tool in tools {
            assert!(tool["name"].is_string() && tool["description"].is_string(), "{tool}");
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert!(tool["inputSchema"]["properties"].is_object() && tool["inputSchema"]["required"].is_array());
            assert!(tool["annotations"]["readOnlyHint"].is_boolean());
        }
        let result = answer(session.handle(r#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#));
        assert_eq!(result["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn a_tool_call_reaches_the_backend_and_its_outcome_the_client() {
        let backend = Fake::on();
        let mut session = Session::new(&backend);
        let result = answer(session.handle(
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get_messages","arguments":{"chat_id":"42","limit":5}}}"#,
        ));
        assert_eq!(backend.calls.borrow().as_slice(), &[ToolCall::GetMessages { chat_id: 42, limit: 5, before_message_id: 0 }]);
        assert_eq!(result["result"]["isError"], false);
        assert_eq!(result["result"]["content"][0], json!({ "type": "text", "text": "done" }));
        assert_eq!(result["result"]["content"][1], json!({ "type": "image", "data": "AAAA", "mimeType": "image/jpeg" }));
        assert!(backend.log.borrow().iter().any(|entry| entry == "get_messages chat=42 limit=5 before=0: done, 2 item(s)"));
    }

    #[test]
    fn bad_lines_and_bad_calls_are_protocol_errors() {
        let backend = Fake::on();
        let mut session = Session::new(&backend);
        let result = answer(session.handle("{not json"));
        assert_eq!((result["error"]["code"].as_i64(), result["id"].is_null()), (Some(PARSE_ERROR), true));
        let result = answer(session.handle("[1, 2]"));
        assert_eq!(result["error"]["code"], INVALID_REQUEST);
        let result = answer(session.handle(r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"fly"}}"#));
        assert_eq!((result["error"]["code"].as_i64(), result["error"]["message"].as_str()), (Some(INVALID_PARAMS), Some("Unknown tool: fly")));
        let result = answer(session.handle(r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"get_chat","arguments":{}}}"#));
        assert_eq!(result["error"]["message"], "chat_id is required");
        let result = answer(session.handle(r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{}}"#));
        assert_eq!(result["error"]["code"], INVALID_PARAMS);
        assert!(backend.calls.borrow().is_empty());
    }

    #[test]
    fn a_failed_tool_is_an_error_the_assistant_reads() {
        struct Failing;
        impl Backend for Failing {
            fn policy(&self) -> Mcp {
                Mcp { enabled: true, ..Mcp::default() }
            }
            fn admit(&self) -> Admission {
                Admission::Allowed
            }
            fn admitted(&self) -> bool {
                true
            }
            fn call(&self, _: ToolCall) -> Outcome {
                Outcome::failed("the chat is not within what the user allows")
            }
            fn log(&self, _: &str) {}
        }
        let mut session = Session::new(&Failing);
        let result = answer(session.handle(r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"get_chat","arguments":{"chat_id":1}}}"#));
        assert!(result.get("error").is_none());
        assert_eq!(result["result"]["isError"], true);
        assert_eq!(result["result"]["content"][0]["text"], "the chat is not within what the user allows");
    }
}
