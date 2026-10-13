//! The MCP server (docs/mcp.md): AI assistants' apps read the user's Telegram and act in it
//! through the running FinchGram, with the Model Context Protocol. FinchGram listens on a Unix
//! socket in its own folder, which only this user's processes may open; the assistant's app runs
//! `finchgram mcp` (bridge.rs), which carries the protocol's lines between its standard input and
//! output and the socket. Each connection gets a thread of its own, which reads a line, has the
//! protocol (protocol.rs) answer it and writes the answer back; a tool's work happens on the UI
//! thread (src/telegram/tools.rs), where the store and TDLib are, and its outcome comes back on a
//! channel. What an assistant may do is Settings → AI access (`settings::Mcp`, access.rs): nothing
//! until the user turns the server on, and nothing for a program the user has not allowed, which
//! the dialog asks about the first time it connects. Who connected and what it asked for go into
//! the log (mcp.log next to the socket) and onto the page, never the words of a message.

mod access;
pub mod bridge;
mod peer;
pub mod protocol;
pub mod tools;

use std::cell::{Cell, RefCell};
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use slint::ComponentHandle;

use crate::MainWindow;
use crate::settings::{Mcp, Settings};
use peer::Host;
use protocol::{Admission, Backend};
use tools::{Activity, ActivityKind, Outcome, ToolCall};

/// How long a tool may take on the UI thread (a download among them) before the assistant is told
/// that it did not finish.
const CALL_TIMEOUT: Duration = Duration::from_secs(180);
/// How long a connection waits for the user to answer the dialog about its program.
const ASK_TIMEOUT: Duration = Duration::from_secs(300);
/// The log starts over once it is this large.
const LOG_LIMIT: u64 = 1_000_000;
/// A socket's path may have about a hundred bytes (sockaddr_un); a longer one goes elsewhere.
const SOCKET_PATH_MAX: usize = 100;

thread_local! {
    /// The app's settings, for the policy an assistant is held to. UI thread.
    static SETTINGS: RefCell<Option<Rc<RefCell<Settings>>>> = const { RefCell::new(None) };
}

/// Show Settings → AI access and listen for assistants. Call once, on the UI thread, when the app
/// is up.
pub fn start(ui: &MainWindow, settings: Rc<RefCell<Settings>>) {
    access::connect(ui, settings.clone());
    SETTINGS.with(|slot| *slot.borrow_mut() = Some(settings));
    let path = socket_path();
    if let Some(folder) = path.parent() {
        let _ = fs::create_dir_all(folder);
    }
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            eprintln!("mcp: another FinchGram is listening at {} already", path.display());
            return;
        }
        // Left by a FinchGram that did not get to clean up.
        let _ = fs::remove_file(&path);
    }
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("mcp: cannot listen at {}: {err}", path.display());
            return;
        }
    };
    // The user's own processes only.
    if let Err(err) = fs::set_permissions(&path, fs::Permissions::from_mode(0o600)) {
        eprintln!("mcp: cannot restrict {}: {err}", path.display());
    }
    log(&format!("listening at {}", path.display()));
    let ui = ui.as_weak();
    let listening = std::thread::Builder::new().name("mcp-listener".into()).spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let ui = ui.clone();
                    if let Err(err) = std::thread::Builder::new().name("mcp-session".into()).spawn(move || serve(stream, ui)) {
                        eprintln!("mcp: cannot start a thread for a connection: {err}");
                    }
                }
                Err(err) => {
                    eprintln!("mcp: the socket failed: {err}");
                    break;
                }
            }
        }
    });
    if let Err(err) = listening {
        eprintln!("mcp: cannot start listening: {err}");
    }
}

/// The socket goes with the app. (A stale one is removed at the next start all the same.)
pub fn shut_down() {
    let _ = fs::remove_file(socket_path());
}

/// Where FinchGram listens: next to its settings, in the user's own folder; or, when that path is
/// too long for a socket, in the user's temporary folder. The test servers' FinchGram
/// (FINCHGRAM_TEST_DC=1) has a socket of its own. The bridge works this out the same way.
pub fn socket_path() -> PathBuf {
    let test_dc = std::env::var("FINCHGRAM_TEST_DC").is_ok_and(|value| value == "1");
    let name = if test_dc { "mcp-test.sock" } else { "mcp.sock" };
    choose_socket_path(settings_folder().map(|folder| folder.join(name)), std::env::temp_dir(), name)
}

fn choose_socket_path(in_settings: Option<PathBuf>, temporary: PathBuf, name: &str) -> PathBuf {
    match in_settings {
        Some(path) if path.as_os_str().len() <= SOCKET_PATH_MAX => path,
        _ => temporary.join(format!("finchgram-{name}")),
    }
}

/// Where settings.toml is (src/settings.rs); the log goes there too.
fn settings_folder() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("FinchGram"))
}

/// The log file, mcp.log next to settings.toml.
fn log_path() -> Option<PathBuf> {
    settings_folder().map(|folder| folder.join("mcp.log"))
}

/// One connection, on its own thread: lines in, answers out, until the other end closes.
fn serve(stream: UnixStream, ui: slint::Weak<MainWindow>) {
    let host = peer::host_of(&stream);
    let backend = UiBackend { ui, host, live: Cell::new(false) };
    backend.log("connected");
    let reading = match stream.try_clone() {
        Ok(reading) => reading,
        Err(err) => {
            backend.log(&format!("cannot read the connection: {err}"));
            return;
        }
    };
    let mut writing = BufWriter::new(stream);
    let mut session = protocol::Session::new(&backend);
    for line in BufReader::new(reading).lines() {
        let Ok(line) = line else { break };
        if let Some(answer) = session.handle(&line)
            && (writeln!(writing, "{answer}").is_err() || writing.flush().is_err())
        {
            break;
        }
    }
    backend.log("disconnected");
    if backend.live.get() {
        let host = backend.host.clone();
        let _ = backend.ui.upgrade_in_event_loop(move |_| access::disconnected(&host));
    }
}

/// The app's side of a session: the settings, the tools and the page live on the UI thread, so
/// each question goes there and its answer comes back on a channel.
struct UiBackend {
    ui: slint::Weak<MainWindow>,
    host: Host,
    /// The program was admitted: the page shows it as connected until the connection ends.
    live: Cell<bool>,
}

impl UiBackend {
    /// Something for the UI thread: the page, the log.
    fn on_ui(&self, work: impl FnOnce(&Host) + Send + 'static) {
        let host = self.host.clone();
        let _ = self.ui.upgrade_in_event_loop(move |_| work(&host));
    }
}

impl Backend for UiBackend {
    fn policy(&self) -> Mcp {
        let (sender, receiver) = mpsc::channel();
        if self.ui.upgrade_in_event_loop(move |_| {
            let _ = sender.send(current_policy());
        }).is_err()
        {
            return Mcp::default();
        }
        receiver.recv_timeout(CALL_TIMEOUT).unwrap_or_default()
    }

    fn admit(&self) -> Admission {
        let (sender, receiver) = mpsc::channel();
        let host = self.host.clone();
        if self.ui.upgrade_in_event_loop(move |_| access::admit(host, sender)).is_err() {
            return Admission::Quitting;
        }
        let admission = receiver.recv_timeout(ASK_TIMEOUT).unwrap_or(Admission::Unanswered);
        let noted = match admission {
            Admission::Allowed => Some(ActivityKind::Connected),
            Admission::Denied => Some(ActivityKind::TurnedAway),
            Admission::Unanswered | Admission::Off | Admission::Quitting => None,
        };
        if let Some(kind) = noted {
            self.on_ui(move |host| access::record(host, Activity::new(kind)));
        }
        if admission == Admission::Allowed && !self.live.replace(true) {
            self.on_ui(access::connected);
        }
        admission
    }

    fn admitted(&self) -> bool {
        let (sender, receiver) = mpsc::channel();
        let host = self.host.clone();
        if self.ui.upgrade_in_event_loop(move |_| {
            let _ = sender.send(access::admitted(&host));
        }).is_err()
        {
            return false;
        }
        receiver.recv_timeout(CALL_TIMEOUT).unwrap_or(false)
    }

    fn call(&self, call: ToolCall) -> Outcome {
        let (sender, receiver) = mpsc::channel();
        let host = self.host.clone();
        let asked = self.ui.upgrade_in_event_loop(move |ui| {
            let policy = current_policy();
            crate::telegram::tools::call(&ui, call, policy, move |outcome, activity| {
                if let Some(activity) = activity {
                    access::record(&host, activity);
                }
                let _ = sender.send(outcome);
            });
        });
        if asked.is_err() {
            return Outcome::failed("FinchGram is quitting");
        }
        match receiver.recv_timeout(CALL_TIMEOUT) {
            Ok(outcome) => outcome,
            Err(_) => Outcome::failed("FinchGram did not finish in time"),
        }
    }

    fn log(&self, entry: &str) {
        log(&format!("{}: {entry}", self.host.describe()));
    }
}

/// Settings → AI access as they are now. UI thread. Everything off when the app has no settings
/// (it always has).
fn current_policy() -> Mcp {
    SETTINGS.with(|slot| slot.borrow().as_ref().map(|settings| settings.borrow().mcp.clone())).unwrap_or_default()
}

static LOG: Mutex<()> = Mutex::new(());

/// A line to stderr and to mcp.log: who connected, what it asked for, how it went. Never a
/// message's words.
fn log(entry: &str) {
    eprintln!("mcp: {entry}");
    let line = format!("{} {entry}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
    let _one_at_a_time = LOG.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(path) = log_path() else { return };
    if fs::metadata(&path).is_ok_and(|file| file.len() > LOG_LIMIT) {
        let _ = fs::rename(&path, path.with_file_name("mcp.log.1"));
    }
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_is_next_to_the_settings_unless_that_path_is_too_long() {
        let short = PathBuf::from("/Users/x/Library/Application Support/FinchGram/mcp.sock");
        assert_eq!(choose_socket_path(Some(short.clone()), PathBuf::from("/tmp"), "mcp.sock"), short);
        let long = PathBuf::from(format!("/Users/{}/Library/Application Support/FinchGram/mcp.sock", "x".repeat(80)));
        assert_eq!(choose_socket_path(Some(long), PathBuf::from("/var/folders/ab/T"), "mcp.sock"), PathBuf::from("/var/folders/ab/T/finchgram-mcp.sock"));
        assert_eq!(choose_socket_path(None, PathBuf::from("/tmp"), "mcp-test.sock"), PathBuf::from("/tmp/finchgram-mcp-test.sock"));
        assert!(socket_path().as_os_str().len() <= SOCKET_PATH_MAX + "finchgram-mcp-test.sock".len());
    }
}
