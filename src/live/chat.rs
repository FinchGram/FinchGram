//! Owncast's chat beside the picture (the canvas, round 12: "Chat from Owncast · read and send").
//! FinchGram joins a server's chat as an anonymous chat user, under the name the server gives
//! (never the user's Telegram name), which the user may change; it reads the history, then the
//! server's WebSocket, and sends what the user writes. Nothing is kept: the messages live in the
//! view's model while the stream is open. One session at a time, on a thread of its own.

use std::cell::{Cell, RefCell};
use std::io::Read as _;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

use serde_json::{Value, json};
use slint::{Model, ModelRc, SharedString, VecModel};

use super::with_directory;
use crate::LiveComment;
use crate::speech::service::{self as socket, Socket};

/// The messages kept in the column; older ones go.
const KEPT: usize = 200;
/// A read waits this long for the server before the session looks at what the user sent.
const READ_WAIT: Duration = Duration::from_millis(250);

/// What the user asks of the session.
enum Out {
    Send(String),
    Rename(String),
    Stop,
}

/// What the session tells the view.
enum In {
    Joined { name: String, user: String },
    Messages(Vec<Message>),
    Renamed(String),
    Hidden(Vec<String>),
    Failed(String),
}

struct Message {
    id: String,
    name: String,
    user: String,
    text: String,
}

thread_local! {
    static SESSION: RefCell<Option<(u64, Sender<Out>)>> = const { RefCell::new(None) };
    static NEXT: Cell<u64> = const { Cell::new(1) };
    /// The messages' ids by their row, for hiding one the moderators hid.
    static IDS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static ME: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Join the chat of the Owncast server at `origin` (https://host), leaving any other.
pub fn open(origin: &str) {
    close();
    let session = NEXT.get();
    NEXT.set(session + 1);
    let (sender, outgoing) = mpsc::channel();
    SESSION.with(|kept| *kept.borrow_mut() = Some((session, sender)));
    IDS.with(|ids| ids.borrow_mut().clear());
    with_directory(|directory| {
        directory.set_chat_messages(ModelRc::new(VecModel::from(Vec::<LiveComment>::new())));
        directory.set_chat_name(SharedString::new());
        directory.set_chat_joined(false);
        directory.set_chat_problem(SharedString::new());
    });
    let origin = origin.to_string();
    let _ = std::thread::Builder::new().name("owncast chat".into()).spawn(move || run(session, &origin, outgoing));
}

/// Leave the chat (the stream stopped or another opened).
pub fn close() {
    SESSION.with(|kept| {
        if let Some((_, sender)) = kept.borrow_mut().take() {
            let _ = sender.send(Out::Stop);
        }
    });
    with_directory(|directory| {
        directory.set_chat_joined(false);
        directory.set_chat_messages(ModelRc::new(VecModel::from(Vec::<LiveComment>::new())));
    });
}

pub fn send(text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    tell(Out::Send(text.chars().take(500).collect()));
}

pub fn rename(name: &str) {
    let name = name.trim();
    if !name.is_empty() {
        tell(Out::Rename(name.chars().take(30).collect()));
    }
}

fn tell(out: Out) {
    SESSION.with(|kept| {
        if let Some((_, sender)) = kept.borrow().as_ref() {
            let _ = sender.send(out);
        }
    });
}

fn report(session: u64, came: In) {
    let _ = slint::invoke_from_event_loop(move || {
        let current = SESSION.with(|kept| kept.borrow().as_ref().is_some_and(|(id, _)| *id == session));
        if current {
            show(came);
        }
    });
}

fn show(came: In) {
    match came {
        In::Joined { name, user } => {
            ME.with(|me| *me.borrow_mut() = user);
            with_directory(|directory| {
                directory.set_chat_name(name.into());
                directory.set_chat_joined(true);
            });
        }
        In::Renamed(name) => with_directory(|directory| directory.set_chat_name(name.into())),
        In::Messages(messages) => {
            let me = ME.with(|me| me.borrow().clone());
            with_directory(|directory| {
                let model = directory.get_chat_messages();
                let mut rows: Vec<LiveComment> = model.iter().collect();
                IDS.with(|ids| {
                    let mut ids = ids.borrow_mut();
                    for message in messages {
                        if ids.contains(&message.id) {
                            continue;
                        }
                        rows.push(LiveComment {
                            id: i32::try_from(ids.len()).unwrap_or(0),
                            initial: super::channel::initial_of(&message.name).into(),
                            name: message.name.into(),
                            text: message.text.into(),
                            stars: 0,
                            tier: 0,
                            host: false,
                            mine: !me.is_empty() && message.user == me,
                            deletable: false,
                        });
                        ids.push(message.id);
                    }
                    if rows.len() > KEPT {
                        let extra = rows.len() - KEPT;
                        rows.drain(..extra);
                        ids.drain(..extra);
                    }
                });
                directory.set_chat_messages(ModelRc::new(VecModel::from(rows)));
            });
        }
        In::Hidden(hidden) => {
            with_directory(|directory| {
                let model = directory.get_chat_messages();
                let mut rows: Vec<LiveComment> = model.iter().collect();
                IDS.with(|ids| {
                    let mut ids = ids.borrow_mut();
                    let mut index = 0;
                    while index < ids.len() {
                        if hidden.contains(&ids[index]) {
                            ids.remove(index);
                            if index < rows.len() {
                                rows.remove(index);
                            }
                        } else {
                            index += 1;
                        }
                    }
                });
                directory.set_chat_messages(ModelRc::new(VecModel::from(rows)));
            });
        }
        In::Failed(problem) => with_directory(|directory| {
            directory.set_chat_joined(false);
            directory.set_chat_problem(problem.into());
        }),
    }
}

/// Owncast writes a message's body as HTML: its words, an emoji image by its alt text.
pub fn plain(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else { break };
        let tag = &rest[start..start + end + 1];
        if tag.starts_with("<img")
            && let Some(alt) = tag.split("alt=\"").nth(1).and_then(|after| after.split('"').next())
        {
            out.push_str(alt);
        }
        if tag.starts_with("<br") || tag.starts_with("</p") {
            out.push(' ');
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    let decoded = out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&nbsp;", " ").replace("&amp;", "&");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn message_of(event: &Value) -> Option<Message> {
    if event.get("type").and_then(Value::as_str) != Some("CHAT") || event.get("visible").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let text = plain(event.get("body").and_then(Value::as_str)?);
    if text.is_empty() {
        return None;
    }
    Some(Message {
        id: event.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
        name: event.pointer("/user/displayName").and_then(Value::as_str).unwrap_or("?").to_string(),
        user: event.pointer("/user/id").and_then(Value::as_str).unwrap_or_default().to_string(),
        text,
    })
}

/// The session: register, read the history, then the socket, until told to stop.
fn run(session: u64, origin: &str, outgoing: Receiver<Out>) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .user_agent(format!("FinchGram/{}", crate::update::CURRENT_VERSION))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .timeout_connect(Some(Duration::from_secs(8)))
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .into();
    let registered = agent.post(&format!("{origin}/api/chat/register")).header("Content-Type", "application/json").send("{}");
    let account: Option<Value> = registered.ok().and_then(|mut response| {
        let mut body = String::new();
        response.body_mut().as_reader().take(64 * 1024).read_to_string(&mut body).ok()?;
        serde_json::from_str(&body).ok()
    });
    let Some(account) = account else {
        return report(session, In::Failed("The server's chat did not let FinchGram in.".into()));
    };
    let (Some(token), Some(name)) = (account.get("accessToken").and_then(Value::as_str), account.get("displayName").and_then(Value::as_str)) else {
        return report(session, In::Failed("The server's chat did not give FinchGram a name.".into()));
    };
    let user = account.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
    report(session, In::Joined { name: name.to_string(), user });
    if let Ok(mut response) = agent.get(&format!("{origin}/api/chat?accessToken={token}")).call() {
        let mut body = String::new();
        if response.body_mut().as_reader().take(4 * 1024 * 1024).read_to_string(&mut body).is_ok()
            && let Ok(Value::Array(events)) = serde_json::from_str::<Value>(&body)
        {
            let messages: Vec<Message> = events.iter().filter_map(message_of).collect();
            let skip = messages.len().saturating_sub(KEPT);
            report(session, In::Messages(messages.into_iter().skip(skip).collect()));
        }
    }
    let address = format!("{}/ws?accessToken={token}", origin.replacen("https://", "wss://", 1).replacen("http://", "ws://", 1));
    let mut socket = match socket::connect(&address, &[]) {
        Ok(socket) => socket,
        Err(failure) => return report(session, In::Failed(failure.detail)),
    };
    set_wait(&socket);
    loop {
        match outgoing.try_recv() {
            Ok(Out::Stop) | Err(TryRecvError::Disconnected) => {
                let _ = socket.close(None);
                return;
            }
            Ok(Out::Send(text)) => {
                if socket::send_text(&mut socket, json!({ "type": "CHAT", "body": text }).to_string()).is_err() {
                    return report(session, In::Failed("The chat's connection broke.".into()));
                }
            }
            Ok(Out::Rename(name)) => {
                if socket::send_text(&mut socket, json!({ "type": "NAME_CHANGE", "newName": name }).to_string()).is_ok() {
                    report(session, In::Renamed(name));
                }
            }
            Err(TryRecvError::Empty) => {}
        }
        match socket.read() {
            Ok(tungstenite::Message::Text(text)) => {
                // A frame may hold several events, one a line.
                let mut messages = Vec::new();
                for line in text.lines() {
                    let Ok(event) = serde_json::from_str::<Value>(line) else { continue };
                    match event.get("type").and_then(Value::as_str) {
                        Some("CHAT") => messages.extend(message_of(&event)),
                        Some("VISIBILITY-UPDATE") if event.get("visible").and_then(Value::as_bool) == Some(false) => {
                            let ids: Vec<String> = event.get("ids").and_then(Value::as_array).map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
                            report(session, In::Hidden(ids));
                        }
                        _ => {}
                    }
                }
                if !messages.is_empty() {
                    report(session, In::Messages(messages));
                }
            }
            Ok(tungstenite::Message::Close(_)) => return report(session, In::Failed("The server closed its chat.".into())),
            Ok(_) => {}
            Err(err) if socket::nothing_yet(&err) => {}
            Err(_) => return report(session, In::Failed("The chat's connection broke.".into())),
        }
    }
}

/// Reads wait a little while, not the speech sessions' few milliseconds.
fn set_wait(socket: &Socket) {
    let _ = match socket.get_ref() {
        socket::Stream::Plain(tcp) => tcp.set_read_timeout(Some(READ_WAIT)),
        socket::Stream::Tls(tls) => tls.sock.set_read_timeout(Some(READ_WAIT)),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_body_reads_as_its_words() {
        assert_eq!(plain("<p>hello <strong>there</strong></p>"), "hello there");
        assert_eq!(plain("nice <img class=\"emoji\" alt=\":fire:\" src=\"/img/emoji/fire.gif\"> set"), "nice :fire: set");
        assert_eq!(plain("a &amp; b &lt;3"), "a & b <3");
        assert_eq!(plain("line<br>two"), "line two");
    }

    #[test]
    fn only_visible_chat_messages_count() {
        let event = json!({ "type": "CHAT", "id": "m1", "body": "<p>hi</p>", "user": { "id": "u1", "displayName": "Jie" } });
        let message = message_of(&event).expect("a message");
        assert_eq!((message.name.as_str(), message.text.as_str(), message.user.as_str()), ("Jie", "hi", "u1"));
        assert!(message_of(&json!({ "type": "CHAT", "body": "x", "visible": false })).is_none());
        assert!(message_of(&json!({ "type": "USER_JOINED" })).is_none());
    }
}
