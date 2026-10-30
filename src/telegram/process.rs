//! finchgram-tdlib running as a separate program (docs/architecture.md): TDLib's own JSON objects,
//! one per line, over its standard input and output. Its standard error goes to ours, as TDLib's
//! log.
//!
//! The program is only ever looked for next to our own executable: in `Contents/MacOS/` of the
//! .app, or in `target/<profile>/` for `cargo run`, where build.rs copies it from vendor/tdlib/bin/.
//! Never anywhere else on the machine. A release is signed by a team (scripts/sign.sh), the
//! program with it: then the program is started only when it carries the same team's signature,
//! unbroken (`location`). It gets the key of the Telegram database, which the Keychain gives
//! this app alone, and everything of the account: a program put in its place by something else
//! on the machine would get them too.
//!
//! Lines are written by a thread of their own, so the UI never waits for a full pipe. What the
//! program writes is read and parsed on another thread and collected in an [`Inbox`]; the UI thread
//! is only woken when the inbox was empty, so a burst of updates arrives as one batch. Neither a
//! line nor the inbox grows without end (LONGEST_LINE, MOST_WAITING).

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::Error;
use super::api::{TdError, Update};

pub const PROGRAM: &str = "finchgram-tdlib";

/// A line from the program longer than this is dropped, and said so: nothing TDLib says comes
/// near it (a hundred messages with their texts are a few megabytes), so it is the program gone
/// wrong, and a line without end would otherwise fill the memory.
const LONGEST_LINE: usize = 64 * 1024 * 1024;
/// How much the inbox holds before the reader thread waits for the UI thread to take it, and how
/// long it waits at most (`Inbox::push`).
const MOST_WAITING: usize = 10_000;
const WAIT_FOR_UI: Duration = Duration::from_secs(2);

/// One message from the program, parsed.
pub enum Output {
    /// The answer to the request with this "@extra": the object TDLib returned, or its error.
    Answer { extra: u64, result: Result<Value, Error> },
    /// (Boxed: an update is much larger than the rest.)
    Update(Box<Update>),
    /// Its standard output closed: the program has ended.
    Ended,
}

/// What the reader thread has parsed and the UI thread not yet taken. It holds MOST_WAITING at
/// most: past that the reader waits for the UI thread to take them, and reads nothing meanwhile,
/// so the program's pipe fills and the program waits in turn, rather than a burst of updates
/// piling up here while the UI thread is busy. Not for ever: a UI thread that takes nothing (the
/// app closing) does not hold the reader.
#[derive(Clone, Default)]
pub struct Inbox(Arc<(Mutex<Vec<Output>>, Condvar)>);

impl Inbox {
    /// True when the inbox was empty, that is when nobody has been woken for it yet.
    fn push(&self, output: Output) -> bool {
        let (outputs, taken) = &*self.0;
        let mut outputs = outputs.lock().unwrap();
        if outputs.len() >= MOST_WAITING {
            outputs = taken.wait_timeout_while(outputs, WAIT_FOR_UI, |outputs| outputs.len() >= MOST_WAITING).unwrap().0;
        }
        outputs.push(output);
        outputs.len() == 1
    }

    pub fn take(&self) -> Vec<Output> {
        let (outputs, taken) = &*self.0;
        let outputs = std::mem::take(&mut *outputs.lock().unwrap());
        taken.notify_all();
        outputs
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
            read_lines(stdout, LONGEST_LINE, |line| {
                if let Some(output) = parse(line)
                    && inbox.push(output)
                {
                    wake();
                }
            });
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

}

/// End several programs at once, giving their TDLibs up to `timeout` all together to close, then
/// stopping the rest the hard way.
pub fn shut_down_all(processes: Vec<Process>, timeout: Duration) {
    let mut processes = processes;
    for process in &mut processes {
        process.close_input();
    }
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        processes.retain_mut(|process| !matches!(process.child.try_wait(), Ok(Some(_)) | Err(_)));
        if processes.is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    eprintln!("telegram: {} {PROGRAM} did not end within {} s, stopping them", processes.len(), timeout.as_secs());
    for mut process in processes {
        let _ = process.child.kill();
        let _ = process.child.wait();
    }
}

/// finchgram-tdlib next to our executable, and ours: when this FinchGram is signed by a team (a
/// release), the program must be signed by the same team and unchanged since, or it is not
/// started. A FinchGram without such a signature (development: `cargo run`, a bundle signed ad
/// hoc) has nothing to hold the program to, and starts the one that is there.
fn location() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|err| format!("cannot find our own executable: {err}"))?;
    let program = exe.parent().ok_or("our executable is not in a folder")?.join(PROGRAM);
    if !program.is_file() {
        return Err(format!("{} is missing: the app is packaged wrongly", program.display()));
    }
    if let Some(ours) = crate::platform::code_signature(std::process::id() as i32).and_then(|(_, team)| team) {
        match crate::platform::file_code_signature(&program) {
            Some((_, Some(team))) if team == ours => {}
            Some((_, Some(team))) => return Err(format!("{} is signed by another team ({team}, not FinchGram's {ours}): it is not FinchGram's, and is not started", program.display())),
            _ => return Err(format!("{} is not signed by FinchGram's team, or was changed since it was signed: it is not started", program.display())),
        }
    }
    Ok(program)
}

/// The lines of `input` to `each`, without their line ends, until it ends. One longer than
/// `longest` is dropped, with a note, and the next read.
fn read_lines(input: impl Read, longest: usize, mut each: impl FnMut(&str)) {
    let mut reader = BufReader::new(input);
    let mut line = Vec::new();
    loop {
        line.clear();
        match (&mut reader).take(longest as u64 + 1).read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if line.len() > longest && line.last() != Some(&b'\n') {
            eprintln!("telegram: a line from {PROGRAM} is longer than {} MB: dropped", longest / (1024 * 1024));
            if !skip_line(&mut reader) {
                return;
            }
            continue;
        }
        while matches!(line.last(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        each(&String::from_utf8_lossy(&line));
    }
}

/// The rest of the line `reader` is in, read and dropped: false when the input ended first.
fn skip_line(reader: &mut impl BufRead) -> bool {
    loop {
        let (newline, length) = match reader.fill_buf() {
            Ok([]) | Err(_) => return false,
            Ok(buffer) => (buffer.iter().position(|&byte| byte == b'\n'), buffer.len()),
        };
        match newline {
            Some(at) => {
                reader.consume(at + 1);
                return true;
            }
            None => reader.consume(length),
        }
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
        Ok(update) => Some(Output::Update(Box::new(update))),
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
        let Some(Output::Update(update)) = parse(
            r#"{"@type":"updateAuthorizationState","authorization_state":{"@type":"authorizationStateWaitPassword","password_hint":"cat","has_recovery_email_address":true,"has_passport_data":false},"@client_id":1}"#,
        ) else {
            panic!("not an update");
        };
        let Update::AuthorizationState { authorization_state } = *update else {
            panic!("not an authorization state");
        };
        assert_eq!(
            authorization_state,
            AuthorizationState::WaitPassword {
                password_hint: "cat".to_string(),
                has_recovery_email_address: true,
                recovery_email_address_pattern: String::new(),
            }
        );
    }

    #[test]
    fn other_updates_and_unreadable_lines_are_dropped() {
        assert!(parse(r#"{"@type":"updateAnimationSearchParameters","provider":"tenor","emojis":[],"@client_id":1}"#).is_none());
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

    #[test]
    fn a_full_inbox_holds_the_reader_until_the_ui_thread_takes_it() {
        let inbox = Inbox::default();
        for _ in 0..MOST_WAITING {
            inbox.push(Output::Ended);
        }
        let pushing = {
            let inbox = inbox.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                inbox.push(Output::Ended);
                started.elapsed()
            })
        };
        std::thread::sleep(Duration::from_millis(100));
        assert!(!pushing.is_finished(), "the reader waits while the inbox is full");
        assert_eq!(inbox.take().len(), MOST_WAITING);
        let waited = pushing.join().expect("pushed");
        assert!(waited >= Duration::from_millis(100) && waited < WAIT_FOR_UI, "{waited:?}");
        assert_eq!(inbox.take().len(), 1);
    }

    #[test]
    fn a_line_without_end_is_dropped_and_the_next_read() {
        let mut lines = Vec::new();
        read_lines(&b"{\"a\":1}\nthis line is far too long to be read\n{\"b\":2}\r\n{\"c\":3}"[..], 16, |line| lines.push(line.to_string()));
        assert_eq!(lines, vec!["{\"a\":1}", "{\"b\":2}", "{\"c\":3}"]);
        // A line of exactly the bound is within it; the one past it is dropped whole.
        let mut lines = Vec::new();
        read_lines(&b"0123456789abcdef\n0123456789abcdefg\nx"[..], 16, |line| lines.push(line.to_string()));
        assert_eq!(lines, vec!["0123456789abcdef", "x"]);
    }
}
