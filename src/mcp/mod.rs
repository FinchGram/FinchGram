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
use std::io::{BufRead, BufReader, BufWriter, ErrorKind, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

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
/// A line longer than this is no message of this protocol: the connection is closed. Any
/// process of the user's may connect to the socket before the user has allowed it, and a reader
/// that took bytes until a newline came would hold them all in memory.
const LINE_LIMIT: u64 = 1024 * 1024;
/// How many connections may be open at once: an assistant's app holds one or two, and each costs
/// a thread; many more is a program misbehaving, and the rest are turned away at the door.
const MOST_CONNECTIONS: usize = 32;
/// A connection that has not said `initialize` within this long is closed, whether it says
/// nothing, anything else, or a line it never ends: a program that connects and never initializes
/// would otherwise hold its thread, and its place among MOST_PENDING, for good. Once initialized it
/// may be quiet as long as it likes: an assistant's app keeps its connection open for hours.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(120);
/// How many connections may wait for `initialize` at once (one asks the user, the others wait
/// their turn at the dialog): more than this is a program opening connections and not using them,
/// and would hold every one of MOST_CONNECTIONS.
const MOST_PENDING: usize = 8;
/// An answer the client does not read within this long ends the connection: a program that
/// asks and never reads would otherwise hold its thread on the write for good.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// How many connections are open now, and how many of them have not initialized yet.
static CONNECTIONS: AtomicUsize = AtomicUsize::new(0);
static PENDING: AtomicUsize = AtomicUsize::new(0);

/// One connection's place among `MOST_CONNECTIONS`, given back when dropped.
struct Counted;

impl Counted {
    fn take() -> Option<Counted> {
        let before = CONNECTIONS.fetch_add(1, Ordering::Relaxed);
        if before >= MOST_CONNECTIONS {
            CONNECTIONS.fetch_sub(1, Ordering::Relaxed);
            return None;
        }
        Some(Counted)
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        CONNECTIONS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// One connection's place among the MOST_PENDING that have not initialized, given back when it
/// has, or when it ends.
struct Pending;

impl Pending {
    fn take() -> Option<Pending> {
        let before = PENDING.fetch_add(1, Ordering::Relaxed);
        if before >= MOST_PENDING {
            PENDING.fetch_sub(1, Ordering::Relaxed);
            return None;
        }
        Some(Pending)
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        PENDING.fetch_sub(1, Ordering::Relaxed);
    }
}

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
                    let Some(counted) = Counted::take() else {
                        log(&format!("a connection turned away: {MOST_CONNECTIONS} are open already"));
                        continue;
                    };
                    let Some(pending) = Pending::take() else {
                        log(&format!("a connection turned away: {MOST_PENDING} are waiting to initialize already"));
                        continue;
                    };
                    let ui = ui.clone();
                    let session = std::thread::Builder::new().name("mcp-session".into()).spawn(move || {
                        let _counted = counted;
                        serve(stream, ui, pending);
                    });
                    if let Err(err) = session {
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

/// The account with `id` has signed in (src/telegram/account.rs): what the user allowed is bound
/// to it, and another account's allowances are dropped (access.rs). UI thread.
pub fn signed_in(id: i64) {
    access::signed_in(id);
}

/// The Telegram account the settings are for, by its user id; 0 until one has signed in.
pub fn bound_account() -> i64 {
    access::bound_account()
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
/// `pending` is its place among the connections waiting for `initialize`, given back once it has.
fn serve(stream: UnixStream, ui: slint::Weak<MainWindow>, pending: Pending) {
    let host = peer::host_of(&stream);
    let backend = UiBackend { ui, host, live: Cell::new(false) };
    backend.log("connected");
    // A client that asks and never reads does not hold the thread on the write.
    if let Err(err) = stream.set_write_timeout(Some(WRITE_TIMEOUT)) {
        backend.log(&format!("cannot set a write timeout: {err}"));
    }
    let reading = match stream.try_clone() {
        Ok(reading) => reading,
        Err(err) => {
            backend.log(&format!("cannot read the connection: {err}"));
            return;
        }
    };
    let mut writing = BufWriter::new(stream);
    let mut session = protocol::Session::new(&backend);
    let mut reader = BufReader::new(reading);
    let mut line = Vec::new();
    // Until `initialize` the client has HANDSHAKE_TIMEOUT in all, however it spends it; after
    // it, as long as it likes.
    let initialize_by = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut pending = Some(pending);
    loop {
        match read_line(&mut reader, &mut line, (!session.ready()).then_some(initialize_by)) {
            Ok(Line::Whole) => {}
            Ok(Line::Closed) | Err(_) => break,
            Ok(Line::Late) => {
                backend.log("did not initialize within two minutes: the connection is closed");
                break;
            }
            Ok(Line::TooLong) => {
                backend.log("sent a line over the limit of 1 MB: the connection is closed");
                break;
            }
        }
        if let Some(answer) = session.handle(&String::from_utf8_lossy(&line))
            && (writeln!(writing, "{answer}").is_err() || writing.flush().is_err())
        {
            break;
        }
        if session.ready() {
            pending = None;
        }
    }
    drop(pending);
    backend.log("disconnected");
    if backend.live.get() {
        let host = backend.host.clone();
        let _ = backend.ui.upgrade_in_event_loop(move |_| access::disconnected(&host));
    }
}

/// How a line of a client's came (`read_line`).
#[derive(Debug, PartialEq, Eq)]
enum Line {
    /// A line, in the buffer, its newline with it (or the last one of a client that then closed).
    Whole,
    /// The client closed the connection.
    Closed,
    /// The line went over LINE_LIMIT and still had not ended.
    TooLong,
    /// The moment given passed before the line had ended.
    Late,
}

/// The client's next line into `line`. With `until`, the whole line must have come by that
/// moment: a read waits no longer than what is left of it, however the bytes come, so that one
/// byte now and then (each within any timeout of a single read) does not keep the line, and the
/// connection, going. Without it a read waits as long as the client is quiet. A line still going
/// past LINE_LIMIT is given up: any process of the user's may connect before the user has allowed
/// it, and a reader that took bytes until a newline came would hold them all in memory.
fn read_line(reader: &mut BufReader<UnixStream>, line: &mut Vec<u8>, until: Option<Instant>) -> std::io::Result<Line> {
    line.clear();
    loop {
        let patience = match until {
            Some(until) => {
                let left = until.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Ok(Line::Late);
                }
                Some(left)
            }
            None => None,
        };
        // Only a read that goes to the socket waits: what is in the buffer is there already.
        // (macOS refuses to set a timeout on a socket the other end has closed; for a client
        // that may be as quiet as it likes that is no matter, its next read says it is closed.)
        if reader.buffer().is_empty() {
            let set = reader.get_ref().set_read_timeout(patience);
            if until.is_some() {
                set?;
            }
        }
        let arrived = match reader.fill_buf() {
            Ok(arrived) => arrived,
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) if until.is_some() && matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => return Ok(Line::Late),
            Err(err) => return Err(err),
        };
        if arrived.is_empty() {
            return Ok(if line.is_empty() { Line::Closed } else { Line::Whole });
        }
        let (taken, ended) = match arrived.iter().position(|byte| *byte == b'\n') {
            Some(at) => (at + 1, true),
            None => (arrived.len(), false),
        };
        line.extend_from_slice(&arrived[..taken]);
        reader.consume(taken);
        if line.len() as u64 > LINE_LIMIT {
            return Ok(Line::TooLong);
        }
        if ended {
            return Ok(Line::Whole);
        }
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
            // The settings may change while the call waits on Telegram: it asks again before each
            // side effect, and before anything is handed over (tools::Guard).
            let guard = crate::telegram::tools::Guard::new(
                {
                    let host = host.clone();
                    move || access::admitted(&host)
                },
                current_policy,
            );
            crate::telegram::tools::call(&ui, call, policy, guard, move |outcome, activity| {
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

/// Settings → AI access as they are now (a call under way asks again before each side effect,
/// tools::Guard). UI thread. Everything off when the app has no settings (it always has).
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

    #[test]
    fn lines_come_one_at_a_time_and_the_last_one_of_a_client_that_closed() {
        let (ours, mut theirs) = UnixStream::pair().expect("a pair");
        let mut reader = BufReader::new(ours);
        let mut line = Vec::new();
        theirs.write_all(b"one\ntwo\nthree").expect("written");
        assert_eq!((read_line(&mut reader, &mut line, None).expect("read"), line.as_slice()), (Line::Whole, &b"one\n"[..]));
        assert_eq!((read_line(&mut reader, &mut line, None).expect("read"), line.as_slice()), (Line::Whole, &b"two\n"[..]));
        drop(theirs);
        assert_eq!((read_line(&mut reader, &mut line, None).expect("read"), line.as_slice()), (Line::Whole, &b"three"[..]));
        assert_eq!(read_line(&mut reader, &mut line, None).expect("read"), Line::Closed);
    }

    #[test]
    fn a_line_that_dribbles_in_does_not_outlast_its_moment() {
        // A byte every 20 ms and never a newline: each read is well within any timeout of its
        // own, and the line goes on for as long as the client likes (four seconds here).
        let (ours, mut theirs) = UnixStream::pair().expect("a pair");
        let dribbling = std::thread::spawn(move || {
            for _ in 0..200 {
                if theirs.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let mut reader = BufReader::new(ours);
        let mut line = Vec::new();
        let started = Instant::now();
        let outcome = read_line(&mut reader, &mut line, Some(started + Duration::from_millis(200))).expect("read");
        let took = started.elapsed();
        // Given up at its moment, long before the client stops: with room for a runner that
        // stalls (GitHub's do), which is no reason for the check to turn red.
        assert_eq!(outcome, Line::Late);
        assert!(took >= Duration::from_millis(200) && took < Duration::from_secs(3), "{took:?}");
        assert!(!line.is_empty() && line.len() < 150, "{} bytes", line.len());
        // A client that says nothing at all is given up at the same moment; one whose moment
        // has passed, at once.
        let started = Instant::now();
        assert_eq!(read_line(&mut reader, &mut line, Some(started)).expect("read"), Line::Late);
        assert!(started.elapsed() < Duration::from_millis(100));
        drop(reader);
        dribbling.join().expect("the client's thread");
    }

    #[test]
    fn a_line_over_the_limit_is_given_up() {
        let (ours, mut theirs) = UnixStream::pair().expect("a pair");
        let writing = std::thread::spawn(move || {
            let _ = theirs.write_all(&vec![b'x'; LINE_LIMIT as usize + 4096]);
        });
        let mut reader = BufReader::new(ours);
        let mut line = Vec::new();
        assert_eq!(read_line(&mut reader, &mut line, None).expect("read"), Line::TooLong);
        assert!(line.len() as u64 > LINE_LIMIT && (line.len() as u64) < LINE_LIMIT + 64 * 1024);
        drop(reader);
        writing.join().expect("the client's thread");
    }
}
