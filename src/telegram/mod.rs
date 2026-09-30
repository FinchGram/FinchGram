//! Telegram, through TDLib (docs/architecture.md). FinchGram never speaks Telegram's protocol
//! itself: everything goes through finchgram-tdlib, a separate program next to our executable that
//! runs the TDLib pinned in vendor/tdlib/build.sh (process.rs), and this module is its only
//! user.
//!
//! Requests are TDLib's own JSON objects ([`send`]). Each gets an "@extra" number, which TDLib
//! copies into its answer, and the answer goes to the callback given with the request. Updates are
//! parsed on the reader thread and reach the UI thread in batches, where they change the app's
//! state: for now how far logging in is and the connection; chats and messages follow with the
//! pages.
//!
//! When finchgram-tdlib ends unexpectedly, the requests still waiting fail and it is started
//! again: TDLib's database is on disk, so it carries on where it was. After a log out TDLib closes
//! itself, and a fresh one is started for the next login.
//!
//! Everything here runs on the UI thread.

mod api;
mod process;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use slint::ComponentHandle;

use crate::{AppState, Connection, MainWindow, TelegramState};
use api::{AuthorizationState, ConnectionState, OptionValue, Update};
use process::{Inbox, Output, Process};

/// The TDLib version api.rs is written for. finchgram-tdlib must be exactly this version, or the
/// app is packaged wrongly. Pinned in vendor/tdlib/build.sh and scripts/fetch-tdlib.sh.
pub const TDLIB_VERSION: &str = "1.8.67";

/// This build's own api_id and api_hash from my.telegram.org, given at compile time: the release
/// workflow takes them from its secrets, developers from their own environment (README). They are
/// never in the repository.
const API_ID: Option<&str> = option_env!("FINCHGRAM_API_ID");
const API_HASH: Option<&str> = option_env!("FINCHGRAM_API_HASH");

/// How long TDLib may take to write its database out when the app quits.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
/// A finchgram-tdlib that ran at least this long before it ended unexpectedly is started again;
/// one that ends sooner is broken, and the app says so instead of starting it over and over.
const RESTART_AFTER: Duration = Duration::from_secs(30);
const RESTART_DELAY: Duration = Duration::from_secs(1);

/// An answer that is not what was asked for.
#[derive(Debug, Clone)]
pub enum Error {
    /// TDLib's own error: its code and message ("PHONE_NUMBER_INVALID", …).
    Telegram { code: i32, message: String },
    /// finchgram-tdlib is not running, or ended before it answered.
    Stopped,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Telegram { code, message } => write!(f, "{message} ({code})"),
            Error::Stopped => f.write_str("finchgram-tdlib is not running"),
        }
    }
}

type Answer = Box<dyn FnOnce(Result<Value, Error>)>;

struct Client {
    ui: slint::Weak<MainWindow>,
    process: Option<Process>,
    /// Which start of finchgram-tdlib this is. Batches from an earlier one are dropped.
    run: u64,
    started: Instant,
    /// TDLib answered with the version api.rs is written for. Until then it gets no parameters.
    version_checked: bool,
    /// TDLib asked for its parameters before its version was checked.
    parameters_wanted: bool,
    /// TDLib said authorizationStateClosed in this run: it ends because it closed (a log out).
    closed: bool,
    /// Something is wrong that starting again would not fix; see `fail`.
    failed: bool,
    quitting: bool,
    next_extra: u64,
    waiting: HashMap<u64, Answer>,
}

thread_local! {
    static CLIENT: RefCell<Option<Client>> = const { RefCell::new(None) };
}

/// Start finchgram-tdlib and show how far it is in `ui`. Call once.
pub fn start(ui: &MainWindow) {
    CLIENT.with(|client| {
        *client.borrow_mut() = Some(Client {
            ui: ui.as_weak(),
            process: None,
            run: 0,
            started: Instant::now(),
            version_checked: false,
            parameters_wanted: false,
            closed: false,
            failed: false,
            quitting: false,
            next_extra: 1,
            waiting: HashMap::new(),
        })
    });
    launch();
}

/// The app is quitting: let TDLib write its database out and finchgram-tdlib end, for at most
/// [`CLOSE_TIMEOUT`]. Call once the event loop has ended.
pub fn shut_down() {
    let process = CLIENT.with(|client| {
        let mut client = client.borrow_mut();
        let client = client.as_mut()?;
        client.quitting = true;
        client.process.take()
    });
    if let Some(process) = process {
        process.shut_down(CLOSE_TIMEOUT);
    }
}

/// Send `request`, one of TDLib's functions as a JSON object (`{"@type": "getMe"}`), and hand its
/// answer to `on_answer`. When finchgram-tdlib is not running, or ends before it answers,
/// `on_answer` gets [`Error::Stopped`].
pub fn send(mut request: Value, on_answer: impl FnOnce(Result<Value, Error>) + 'static) {
    let mut on_answer: Option<Answer> = Some(Box::new(on_answer));
    CLIENT.with(|client| {
        let mut client = client.borrow_mut();
        let Some(client) = client.as_mut() else { return };
        if client.process.is_none() {
            return;
        }
        let extra = client.next_extra;
        client.next_extra += 1;
        request["@extra"] = extra.into();
        client.waiting.insert(extra, on_answer.take().expect("taken once"));
        if let Some(process) = &client.process {
            process.send(request.to_string());
        }
    });
    if let Some(on_answer) = on_answer {
        on_answer(Err(Error::Stopped));
    }
}

/// Start finchgram-tdlib (again), as a new run.
fn launch() {
    let started = CLIENT.with(|client| {
        let mut client = client.borrow_mut();
        let client = client.as_mut().expect("started");
        client.run += 1;
        let run = client.run;
        let inbox = Inbox::default();
        let wake = {
            let inbox = inbox.clone();
            move || {
                let inbox = inbox.clone();
                // Fails only once the event loop has ended, when nothing waits for TDLib any more.
                let _ = slint::invoke_from_event_loop(move || deliver(run, inbox.take()));
            }
        };
        let process = Process::start(inbox, wake)?;
        client.process = Some(process);
        client.started = Instant::now();
        client.version_checked = false;
        client.parameters_wanted = false;
        client.closed = false;
        Ok::<(), String>(())
    });
    match started {
        Ok(()) => {
            with_state(|app| app.set_telegram_state(TelegramState::Starting));
            check_version();
        }
        Err(err) => fail(err),
    }
}

/// A batch from finchgram-tdlib.
fn deliver(run: u64, batch: Vec<Output>) {
    for output in batch {
        // Looked at before every message: handling one may have started a new run.
        if CLIENT.with(|client| client.borrow().as_ref().map(|client| client.run)) != Some(run) {
            return;
        }
        match output {
            Output::Answer { extra, result } => {
                let on_answer =
                    CLIENT.with(|client| client.borrow_mut().as_mut().and_then(|client| client.waiting.remove(&extra)));
                if let Some(on_answer) = on_answer {
                    on_answer(result);
                }
            }
            Output::Update(update) => on_update(update),
            Output::Ended => on_ended(),
        }
    }
}

fn on_update(update: Update) {
    match update {
        Update::AuthorizationState { authorization_state } => on_authorization_state(authorization_state),
        Update::ConnectionState { state } => with_state(|app| app.set_connection(connection(state))),
        Update::Other => {}
    }
}

fn on_authorization_state(state: AuthorizationState) {
    let shown = match state {
        AuthorizationState::WaitTdlibParameters => {
            let checked = CLIENT.with(|client| {
                let mut client = client.borrow_mut();
                let client = client.as_mut()?;
                client.parameters_wanted = !client.version_checked;
                Some(client.version_checked)
            });
            if checked == Some(true) {
                set_parameters();
            }
            return;
        }
        AuthorizationState::WaitPhoneNumber => TelegramState::WaitPhoneNumber,
        AuthorizationState::WaitPremiumPurchase => TelegramState::WaitPremiumPurchase,
        AuthorizationState::WaitEmailAddress => TelegramState::WaitEmailAddress,
        AuthorizationState::WaitEmailCode => TelegramState::WaitEmailCode,
        AuthorizationState::WaitCode => TelegramState::WaitCode,
        AuthorizationState::WaitOtherDeviceConfirmation { .. } => TelegramState::WaitOtherDevice,
        AuthorizationState::WaitRegistration => TelegramState::WaitRegistration,
        AuthorizationState::WaitPassword { .. } => TelegramState::WaitPassword,
        AuthorizationState::Ready => TelegramState::Ready,
        AuthorizationState::LoggingOut => TelegramState::LoggingOut,
        AuthorizationState::Closing => TelegramState::Closing,
        AuthorizationState::Closed => {
            // finchgram-tdlib ends right after this; on_ended decides what follows.
            CLIENT.with(|client| {
                if let Some(client) = client.borrow_mut().as_mut() {
                    client.closed = true;
                }
            });
            return;
        }
    };
    with_state(|app| app.set_telegram_state(shown));
}

/// The first request of every run: which TDLib this is. Anything but the version api.rs is
/// written for is a packaging error. (The first request is also what starts TDLib's client.)
fn check_version() {
    send(json!({ "@type": "getOption", "name": "version" }), |answer| {
        let version = match answer.map(serde_json::from_value::<OptionValue>) {
            Ok(Ok(OptionValue::String { value })) => value,
            Err(Error::Stopped) => return, // on_ended tells what happened
            other => return fail(format!("cannot read the TDLib version of finchgram-tdlib: {other:?}")),
        };
        if version != TDLIB_VERSION {
            return fail(format!(
                "finchgram-tdlib runs TDLib {version}, this build is made for {TDLIB_VERSION}: the app is packaged wrongly"
            ));
        }
        with_state(|app| app.set_tdlib_version(version.into()));
        let wanted = CLIENT.with(|client| {
            let mut client = client.borrow_mut();
            let client = client.as_mut()?;
            client.version_checked = true;
            Some(std::mem::take(&mut client.parameters_wanted))
        });
        if wanted == Some(true) {
            set_parameters();
        }
    });
}

/// TDLib's first question: where its database is, and who is asking.
fn set_parameters() {
    let (Some(api_id), Some(api_hash)) = (API_ID.and_then(|id| id.trim().parse::<i32>().ok()), API_HASH) else {
        eprintln!("telegram: this build has no api_id: build with FINCHGRAM_API_ID and FINCHGRAM_API_HASH set (README)");
        with_state(|app| app.set_telegram_state(TelegramState::MissingApiId));
        return;
    };
    let test_dc = use_test_dc();
    let database = match database_directory(test_dc) {
        Ok(database) => database,
        Err(err) => return fail(err),
    };
    let parameters = json!({
        "@type": "setTdlibParameters",
        "use_test_dc": test_dc,
        "database_directory": database,
        "files_directory": "", // downloaded files are kept next to the database
        "database_encryption_key": "",
        "use_file_database": true,
        "use_chat_info_database": true,
        "use_message_database": true,
        "use_secret_chats": true,
        "api_id": api_id,
        "api_hash": api_hash,
        "system_language_code": sys_locale::get_locale().unwrap_or_else(|| "en".to_string()),
        "device_model": crate::platform::device_model(),
        "system_version": "", // detected by TDLib
        "application_version": env!("CARGO_PKG_VERSION"),
    });
    send(parameters, |answer| {
        if let Err(err @ Error::Telegram { .. }) = answer {
            fail(format!("TDLib did not accept its parameters: {err}"));
        }
    });
}

/// finchgram-tdlib has ended: because the app asked it to (quitting, or after a failure), because
/// TDLib closed itself (a log out), or unexpectedly.
fn on_ended() {
    let ended = CLIENT.with(|client| {
        let mut client = client.borrow_mut();
        let client = client.as_mut()?;
        let status = client.process.take().and_then(Process::reap);
        let waiting: Vec<Answer> = client.waiting.drain().map(|(_, on_answer)| on_answer).collect();
        let next = if client.quitting || client.failed {
            Next::Nothing
        } else if client.closed {
            Next::StartAgain
        } else if client.started.elapsed() >= RESTART_AFTER {
            Next::StartAgainLater
        } else {
            Next::Fail
        };
        Some((status, waiting, next))
    });
    let Some((status, waiting, next)) = ended else { return };
    let status = status.map_or_else(|| "unknown".to_string(), |status| status.to_string());
    eprintln!("telegram: {} ended ({status})", process::PROGRAM);
    for on_answer in waiting {
        on_answer(Err(Error::Stopped));
    }
    match next {
        Next::Nothing => {}
        Next::StartAgain => launch(),
        Next::StartAgainLater => {
            with_state(|app| app.set_telegram_state(TelegramState::Starting));
            slint::Timer::single_shot(RESTART_DELAY, launch);
        }
        Next::Fail => fail(format!("{} stopped soon after it started ({status})", process::PROGRAM)),
    }
}

enum Next {
    Nothing,
    StartAgain,
    StartAgainLater,
    Fail,
}

/// Something is wrong that starting again would not fix: show it, and let finchgram-tdlib end.
fn fail(message: String) {
    eprintln!("telegram: {message}");
    CLIENT.with(|client| {
        if let Some(client) = client.borrow_mut().as_mut() {
            client.failed = true;
            if let Some(process) = &mut client.process {
                process.close_input();
            }
        }
    });
    with_state(|app| {
        app.set_telegram_error(message.into());
        app.set_telegram_state(TelegramState::Failed);
    });
}

/// Change the app's state, when the window still exists.
fn with_state(change: impl FnOnce(&AppState)) {
    let ui = CLIENT.with(|client| client.borrow().as_ref().and_then(|client| client.ui.upgrade()));
    if let Some(ui) = ui {
        change(&ui.global::<AppState>());
    }
}

fn connection(state: ConnectionState) -> Connection {
    match state {
        ConnectionState::WaitingForNetwork => Connection::WaitingForNetwork,
        ConnectionState::ConnectingToProxy => Connection::ConnectingToProxy,
        ConnectionState::Connecting => Connection::Connecting,
        ConnectionState::Updating => Connection::Updating,
        ConnectionState::Ready => Connection::Ready,
    }
}

/// Telegram's test servers instead of the real ones, for development: FINCHGRAM_TEST_DC=1 when
/// starting the app. They have their own accounts, so their own database.
fn use_test_dc() -> bool {
    std::env::var("FINCHGRAM_TEST_DC").is_ok_and(|value| value == "1")
}

/// TDLib's database and downloaded files, in the platform's data folder
/// (`~/Library/Application Support/FinchGram/tdlib` on macOS).
fn database_directory(test_dc: bool) -> Result<PathBuf, String> {
    let base = dirs::data_dir().ok_or("the system has no data folder for TDLib's database")?;
    let database = base.join("FinchGram").join(if test_dc { "tdlib-test" } else { "tdlib" });
    std::fs::create_dir_all(&database).map_err(|err| format!("cannot create {}: {err}", database.display()))?;
    Ok(database)
}
