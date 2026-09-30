//! finchgram-tdlib running as a separate program (docs/architecture.md): TDLib's own JSON objects,
//! one per line, over its standard input and output. Its standard error goes to ours, as TDLib's
//! log.
//!
//! The program is only ever looked for next to our own executable: in `Contents/MacOS/` of the
//! .app, or in `target/<profile>/` for `cargo run`, where build.rs copies it from vendor/tdlib/bin/.
//! Never anywhere else on the machine.
//!
//! Lines are written by a thread of their own, so the UI never waits for a full pipe. What the
//! program writes is read and parsed on another thread and collected in an [`Inbox`]; the UI thread
//! is only woken when the inbox was empty, so a burst of updates arrives as one batch.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::Error;
use super::api::{TdError, Update};

pub const PROGRAM: &str = "finchgram-tdlib";

/// One message from the program, parsed.
pub enum Output {
    /// The answer to the request with this "@extra": the object TDLib returned, or its error.
    Answer { extra: u64, result: Result<Value, Error> },
    Update(Update),
    /// Its standard output closed: the program has ended.
    Ended,
}

/// What the reader thread has parsed and the UI thread not yet taken.
#[derive(Clone, Default)]
pub struct Inbox(Arc<Mutex<Vec<Output>>>);

impl Inbox {
    /// True when the inbox was empty, that is when nobody has been woken for it yet.
    fn push(&self, output: Output) -> bool {
        let mut outputs = self.0.lock().unwrap();
        outputs.push(output);
        outputs.len() == 1
    }

    pub fn take(&self) -> Vec<Output> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

pub struct Process {
    child: Child,
    /// Lines for the writer thread. Dropping it closes the program's standard input, which asks
    /// TDLib to close and the program to end.
    input: Option<Sender<String>>,
}

impl Process {
    /// Start the program. `wake` is called on the reader thread whenever `inbox` goes from empty
    /// to not empty, and should have the UI thread take what is in it.
    pub fn start(inbox: Inbox, wake: impl Fn() + Send + 'static) -> Result<Process, String> {
        let program = location()?;
        let mut child = Command::new(&program)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|err| format!("cannot start {}: {err}", program.display()))?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");

        let (input, lines) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for mut line in lines {
                line.push('\n');
                if stdin.write_all(line.as_bytes()).and_then(|()| stdin.flush()).is_err() {
                    break; // the program has ended; the reader thread notices too
                }
            }
        });

        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Some(output) = parse(&line)
                    && inbox.push(output)
                {
                    wake();
                }
            }
            if inbox.push(Output::Ended) {
                wake();
            }
        });

        Ok(Process { child, input: Some(input) })
    }

    pub fn send(&self, line: String) {
        if let Some(input) = &self.input {
            // Fails only once the writer thread has stopped, when the program is ending anyway.
            let _ = input.send(line);
        }
    }

    /// Ask the program to end: TDLib closes, writes its database out, and the program ends.
    pub fn close_input(&mut self) {
        self.input = None;
    }

    /// The program has ended (its output closed): collect it.
    pub fn reap(mut self) -> Option<ExitStatus> {
        self.child.wait().ok()
    }

    /// End the program, giving TDLib up to `timeout` to close, then stop it the hard way.
    pub fn shut_down(mut self, timeout: Duration) {
        self.close_input();
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        eprintln!("telegram: {PROGRAM} did not end within {} s, stopping it", timeout.as_secs());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// finchgram-tdlib next to our executable.
fn location() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|err| format!("cannot find our own executable: {err}"))?;
    let program = exe.parent().ok_or("our executable is not in a folder")?.join(PROGRAM);
    if program.is_file() {
        Ok(program)
    } else {
        Err(format!("{} is missing: the app is packaged wrongly", program.display()))
    }
}

/// One line from the program: an answer when it carries our "@extra", else an update. Updates
/// nothing follows yet are dropped here, off the UI thread.
fn parse(line: &str) -> Option<Output> {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(err) => {
            eprintln!("telegram: unreadable line from {PROGRAM}: {err}: {line:.200}");
            return None;
        }
    };
    if let Some(extra) = message.get("@extra").and_then(Value::as_u64) {
        let result = if message.get("@type").and_then(Value::as_str) == Some("error") {
            let error: TdError = serde_json::from_value(message)
                .unwrap_or_else(|err| TdError { code: 0, message: format!("unreadable error: {err}") });
            Err(Error::Telegram { code: error.code, message: error.message })
        } else {
            Ok(message)
        };
        return Some(Output::Answer { extra, result });
    }
    let kind = message.get("@type").and_then(Value::as_str).unwrap_or("?").to_string();
    match serde_json::from_value::<Update>(message) {
        Ok(Update::Other) => None,
        Ok(update) => Some(Output::Update(update)),
        Err(err) => {
            eprintln!("telegram: cannot read {kind}: {err}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telegram::api::AuthorizationState;

    #[test]
    fn an_answer_carries_the_extra_of_its_request() {
        let Some(Output::Answer { extra, result: Ok(answer) }) =
            parse(r#"{"@type":"optionValueString","value":"1.8.67","@extra":7,"@client_id":1}"#)
        else {
            panic!("not an answer");
        };
        assert_eq!(extra, 7);
        assert_eq!(answer["value"], "1.8.67");
    }

    #[test]
    fn an_error_answer_is_an_error() {
        let Some(Output::Answer { extra: 3, result: Err(Error::Telegram { code, message }) }) =
            parse(r#"{"@type":"error","code":400,"message":"PHONE_NUMBER_INVALID","@extra":3,"@client_id":1}"#)
        else {
            panic!("not an error answer");
        };
        assert_eq!((code, message.as_str()), (400, "PHONE_NUMBER_INVALID"));
    }

    #[test]
    fn a_followed_update_is_read_and_its_unknown_fields_ignored() {
        let Some(Output::Update(Update::AuthorizationState { authorization_state })) = parse(
            r#"{"@type":"updateAuthorizationState","authorization_state":{"@type":"authorizationStateWaitPassword","password_hint":"cat","has_recovery_email_address":true},"@client_id":1}"#,
        ) else {
            panic!("not an authorization state");
        };
        assert_eq!(authorization_state, AuthorizationState::WaitPassword { password_hint: "cat".to_string() });
    }

    #[test]
    fn other_updates_and_unreadable_lines_are_dropped() {
        assert!(parse(r#"{"@type":"updateFile","file":{"@type":"file","id":1},"@client_id":1}"#).is_none());
        assert!(parse("not json").is_none());
    }

    #[test]
    fn the_ui_thread_is_woken_once_per_batch() {
        let inbox = Inbox::default();
        assert!(inbox.push(Output::Ended), "the first message wakes the UI thread");
        assert!(!inbox.push(Output::Ended), "later ones join the batch");
        assert_eq!(inbox.take().len(), 2);
        assert!(inbox.push(Output::Ended), "after the batch was taken, the next message wakes it again");
    }
}
