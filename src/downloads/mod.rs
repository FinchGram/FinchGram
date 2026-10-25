//! Downloads (the design's third round of the shared pages, "FinchGram Desktop 下载", the
//! protocols of the seventh, the changes of the ninth): servers over WebDAV, FTP, FTPS or SFTP,
//! their folders, and files fetched over several connections at once (docs/downloads.md); on a
//! server the user allows changes on, files deleted, moved, renamed and uploaded too. `remote.rs`
//! puts one face on the three clients (`webdav.rs` with `transfer.rs`, `ftp.rs`, `sftp.rs`);
//! this module keeps the page's state on the UI thread and fills the Downloads global
//! (ui/downloads.slint), which the page (ui/pages/downloads.slint) shows in every theme.
//!
//! Threads (docs/architecture.md): one worker lists folders, checks servers and tests the form's
//! server, one request at a time in the order asked, and answers through the event loop; a
//! listing that is not the current one any more is dropped when it comes back. Each transfer
//! runs on a thread of its own, two at a time, and reports its progress the same way; a batch of
//! changes (a delete, a move, a rename, a new folder) on a thread of its own as well. The
//! servers, with their passwords and whether changes are allowed on each, are the app's
//! database's (src/database.rs).

pub mod ftp;
pub mod http;
pub mod log;
pub mod remote;
pub mod sftp;
pub mod transfer;
pub mod webdav;

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, TimeZone};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use remote::{Changes, Protocol, Remote};

use crate::settings::Settings;
use crate::{AppState, ChangesSaid, DialogItem, Downloads, FolderEntry, FolderState, FormError, MainWindow, MoveFolder, Problem, RenameError, ServerRow, ServerStatus, TestState, TransferRow, TransferState, database, platform, telegram};

/// How many files are fetched at the same time; the rest wait.
const MOST_ACTIVE: usize = 2;
/// The speed shown is over this long.
const SPEED_WINDOW: Duration = Duration::from_secs(10);
/// "Copied" stays on the key dialog's link this long.
const COPIED_FOR: Duration = Duration::from_millis(1600);
/// A toast ("Deleted 3 items") stays this long.
const TOAST_FOR: Duration = Duration::from_millis(3200);
/// The Delete dialog counts what a folder holds by listing it, this many folders deep at most;
/// a larger tree shows the count so far.
const MOST_COUNTED_FOLDERS: usize = 200;

/// A server as the page keeps it.
#[derive(Clone)]
struct Server {
    name: String,
    protocol: Protocol,
    /// As typed: "https://nas.home/dav" for WebDAV, "host" or "host/folder" for the others;
    /// never with a port in it.
    address: String,
    /// 0 for the protocol's default.
    port: u16,
    username: String,
    /// "" for a server that asks for none.
    password: String,
    status: ServerStatus,
    /// The user lets FinchGram change files on the server (the ninth round): the form's switch.
    allow_changes: bool,
    /// What the server said about changes (remote::Changes), once asked.
    changes: Changes,
    changes_asked: bool,
}

impl Server {
    fn from_record(record: database::ServerRecord) -> Server {
        Server {
            name: record.name,
            protocol: Protocol::parse(&record.protocol).unwrap_or(Protocol::Webdav),
            address: record.address,
            port: record.port,
            username: record.username,
            password: record.password,
            status: ServerStatus::Unknown,
            allow_changes: record.allow_changes,
            changes: Changes::Unknown,
            changes_asked: false,
        }
    }

    /// The server as the database keeps it.
    fn record(&self) -> database::ServerRecord {
        database::ServerRecord {
            name: self.name.clone(),
            protocol: self.protocol.id().to_string(),
            address: self.address.clone(),
            port: self.port,
            username: self.username.clone(),
            password: self.password.clone(),
            allow_changes: self.allow_changes,
        }
    }

    /// The client for the worker and the downloads, by what the server speaks. An SFTP server
    /// gets the slot its trusted key is held in (`known_key_slot`), the database's key in it.
    fn remote(&self) -> Remote {
        match self.protocol {
            Protocol::Webdav => {
                let port = if self.port == 0 { String::new() } else { self.port.to_string() };
                let url = webdav::Server::normalized_url(&webdav::Server::with_port(&self.address, &port));
                Remote::WebDav(webdav::Server { url, username: self.username.clone(), password: self.password.clone() })
            }
            Protocol::Ftp | Protocol::Ftps => {
                let (host, folder) = remote::host_and_folder(&self.address);
                Remote::Ftp(ftp::Server { host, port: self.port, folder, tls: self.protocol == Protocol::Ftps, username: self.username.clone(), password: self.password.clone() })
            }
            Protocol::Sftp => {
                let (host, folder) = remote::host_and_folder(&self.address);
                let known_key = known_key_slot(&host, if self.port == 0 { 22 } else { self.port });
                Remote::Sftp(sftp::Server { host, port: self.port, folder, username: self.username.clone(), password: self.password.clone(), known_key })
            }
        }
    }

    /// The host, for the page: "nas.home" out of the address.
    fn host(&self) -> String {
        match self.protocol {
            Protocol::Webdav => webdav::Server::host(&self.address),
            _ => remote::host_and_folder(&self.address).0,
        }
    }

    /// An SFTP server's host and port: what its key is trusted by.
    fn ssh_host(&self) -> Option<(String, u16)> {
        (self.protocol == Protocol::Sftp).then(|| (remote::host_and_folder(&self.address).0, if self.port == 0 { 22 } else { self.port }))
    }

    fn row(&self) -> ServerRow {
        ServerRow { name: self.name.clone().into(), host: self.host().into(), protocol: self.protocol.name().into(), status: self.status }
    }
}

/// What the middle column shows of the folder.
enum Folder {
    Loading,
    Listed,
    Error(Problem, String),
    /// Nothing, until the SFTP server's key is trusted: not trusted yet, or `changed`.
    KeyWait { changed: bool },
}

/// What a transfer moves.
#[derive(Clone)]
enum Kind {
    /// The file `name` in the folder `path` of the server, into `dest`, under the download folder
    /// `root`.
    File { remote: Remote, path: Vec<String>, name: String, root: PathBuf, dest: PathBuf },
    /// A whole folder, file by file, into `dest`, under the download folder `root`.
    Folder { remote: Remote, path: Vec<String>, root: PathBuf, dest: PathBuf },
    /// The file at `local` on this Mac, `size` bytes, up into the folder `path` of the server as
    /// `name` (the ninth round); `dest` is the folder's name as the row says it.
    Upload { remote: Remote, path: Vec<String>, name: String, local: PathBuf, size: u64, dest: String },
}

impl Kind {
    /// What a transfer holds on this Mac while it runs: a download's file or folder, an upload's
    /// source. Two transfers on one path never run at once (start_next).
    fn dest(&self) -> &Path {
        match self {
            Kind::File { dest, .. } | Kind::Folder { dest, .. } => dest,
            Kind::Upload { local, .. } => local,
        }
    }

    /// A download's partial files, to remove when it is given up on: where they are, under the
    /// download folder as it was when the download was queued (`dest` is under it, and the
    /// engines write nothing outside it, transfer::Folder). An upload leaves nothing.
    fn partials(&self) -> Option<(&Path, &Path)> {
        match self {
            Kind::File { root, dest, .. } | Kind::Folder { root, dest, .. } => Some((root, dest)),
            Kind::Upload { .. } => None,
        }
    }

    fn remote(&self) -> &Remote {
        match self {
            Kind::File { remote, .. } | Kind::Folder { remote, .. } | Kind::Upload { remote, .. } => remote,
        }
    }

    fn is_upload(&self) -> bool {
        matches!(self, Kind::Upload { .. })
    }
}

/// A transfer in the queue: a download, or an upload.
struct Transfer {
    id: i32,
    name: String,
    icon: &'static str,
    kind: Kind,
    state: TransferState,
    done: u64,
    total: u64,
    /// (when, bytes done) over the last moments, for the speed.
    samples: VecDeque<(Instant, u64)>,
    /// The HTTP status a busy server keeps answering (503 …), shown while it does; 0 otherwise.
    waiting: u16,
    problem: Problem,
    detail: String,
    cancel: Arc<AtomicBool>,
    /// Where it ended up, once done.
    result: Option<PathBuf>,
    /// Shown (a photo or video) or revealed once done: "Open".
    open_when_done: bool,
    /// The key the SFTP server showed when the download stopped for it to be checked.
    key_shown: Option<String>,
}

impl Transfer {
    fn live(&self) -> bool {
        matches!(self.state, TransferState::Active | TransferState::Waiting)
    }

    /// Bytes per second over the last moments; none until there are enough samples.
    fn speed(&self) -> Option<f64> {
        let (first_at, first) = *self.samples.front()?;
        let (last_at, last) = *self.samples.back()?;
        let seconds = last_at.duration_since(first_at).as_secs_f64();
        if seconds < 0.5 || last <= first {
            return None;
        }
        Some((last - first) as f64 / seconds)
    }

    fn row(&self) -> TransferRow {
        let speed = if self.state == TransferState::Active { self.speed() } else { None };
        let seconds_left = speed.filter(|speed| *speed > 0.0).map(|speed| (self.total.saturating_sub(self.done) as f64 / speed).ceil() as i32);
        TransferRow {
            id: self.id,
            name: self.name.clone().into(),
            icon: self.icon.into(),
            upload: self.kind.is_upload(),
            dest: match &self.kind {
                Kind::Upload { dest, .. } => dest.clone().into(),
                _ => SharedString::new(),
            },
            state: self.state,
            fraction: if self.total > 0 { (self.done as f64 / self.total as f64).clamp(0.0, 1.0) as f32 } else { 0.0 },
            done: size_text(self.done).into(),
            total: size_text(self.total).into(),
            speed: speed.map(speed_text).unwrap_or_default().into(),
            waiting: self.waiting as i32,
            seconds_left: seconds_left.unwrap_or(-1),
            minutes_left: seconds_left.map(|seconds| (seconds + 59) / 60).unwrap_or(-1),
            problem: self.problem,
            detail: self.detail.clone().into(),
        }
    }

    /// The queue's order: on their way, waiting, stopped, done.
    fn rank(&self) -> u8 {
        match self.state {
            TransferState::Active => 0,
            TransferState::Waiting => 1,
            TransferState::Stopped => 2,
            TransferState::Done => 3,
        }
    }
}

/// What the worker thread does, one at a time.
enum Task {
    /// The folder's listing; with `ask_changes`, what the server says about changes as well, once.
    List { request: u64, remote: Remote, path: Vec<String>, ask_changes: bool },
    Check { index: usize, remote: Remote },
    Test { request: u64, remote: Remote },
    /// A folder's listing for the Move dialog.
    Browse { request: u64, remote: Remote, path: Vec<String> },
    /// How many entries a folder holds, all the way down, for the Delete dialog's `slot`.
    Count { request: u64, remote: Remote, path: Vec<String>, slot: usize },
}

/// What Test connection found: folders and files at the root, whether the server does ranges
/// (asked of the first file, when there is one), and what it says about changes.
struct TestResult {
    folders: i32,
    files: i32,
    ranges: Option<bool>,
    changes: Changes,
}

struct State {
    ui: slint::Weak<MainWindow>,
    settings: Rc<RefCell<Settings>>,
    servers: Vec<Server>,
    /// The server looked at.
    current: Option<usize>,
    /// The folder looked at, from the server's root.
    path: Vec<String>,
    entries: Vec<remote::Entry>,
    folder: Folder,
    /// The files chosen in the folder, by name.
    selected: BTreeSet<String>,
    filter: String,
    /// The listing asked for last; answers to earlier ones are dropped.
    request: u64,
    test_request: u64,
    queue: Vec<Transfer>,
    next_id: i32,
    /// The server being edited in the form; none while adding one.
    editing: Option<usize>,
    worker: Option<Sender<Task>>,
    /// The system is told the app is busy while a file is on its way (no App Nap, no sleep).
    activity: Option<platform::Activity>,
    /// The question about an SFTP server's key, while its dialog is open.
    key_question: Option<KeyQuestion>,
    /// Takes "Copied" off the dialog's link again.
    copied: slint::Timer,
    /// The listing asked for last is to replace the entries without a spinner first (after a
    /// change on the server).
    list_quietly: bool,
    /// The entry being renamed in place (the ninth round), by name.
    renaming: Option<String>,
    /// The Delete dialog, while it is open.
    deleting: Option<Deleting>,
    /// The Move dialog, while it is open.
    moving: Option<Moving>,
    browse_request: u64,
    /// The Same-name dialog, while it is open: what waits on the answer.
    conflict: Option<Conflict>,
    /// Files dragged over the window, for the drop box, and those dropped, taken together once
    /// they have all come.
    hovering: Vec<PathBuf>,
    dropped: Vec<PathBuf>,
    flush_asked: bool,
    /// Takes the toast off again.
    toast: slint::Timer,
    /// A finished upload's Show: the file to choose once its folder is listed.
    reveal: Option<String>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub fn connect(ui: &MainWindow, settings: Rc<RefCell<Settings>>) {
    let servers: Vec<Server> = database::servers().into_iter().map(Server::from_record).collect();
    STATE.with(|state| {
        *state.borrow_mut() = Some(State {
            ui: ui.as_weak(),
            settings: settings.clone(),
            servers,
            current: None,
            path: Vec::new(),
            entries: Vec::new(),
            folder: Folder::Loading,
            selected: BTreeSet::new(),
            filter: String::new(),
            request: 0,
            test_request: 0,
            queue: Vec::new(),
            next_id: 1,
            editing: None,
            worker: None,
            activity: None,
            key_question: None,
            copied: slint::Timer::default(),
            list_quietly: false,
            renaming: None,
            deleting: None,
            moving: None,
            browse_request: 0,
            conflict: None,
            hovering: Vec::new(),
            dropped: Vec::new(),
            flush_asked: false,
            toast: slint::Timer::default(),
            reveal: None,
        })
    });
    let downloads = ui.global::<Downloads>();
    {
        let settings = settings.borrow();
        downloads.set_queue_open(settings.downloads.queue_shown);
        downloads.set_connections(settings.downloads.connections as i32);
        downloads.set_folder_label(folder_label(&settings).into());
    }
    show_servers();
    downloads.on_opened(opened);
    downloads.on_select_server(select_server);
    downloads.on_add_server(add_server);
    downloads.on_edit_server(edit_server);
    downloads.on_remove_server(remove_server);
    downloads.on_copy_address(copy_address);
    downloads.on_open_folder(|name| open_folder(&name));
    downloads.on_go_to(go_to);
    downloads.on_refresh(list);
    downloads.on_set_filter(|text| set_filter(&text));
    downloads.on_toggle(|name| toggle(&name));
    downloads.on_toggle_all(toggle_all);
    downloads.on_clear_selection(clear_selection);
    downloads.on_download_selected(|| download_selected(false));
    downloads.on_open_selected(|| download_selected(true));
    downloads.on_download_file(|name| enqueue_entry(&name, false));
    downloads.on_open_file(|name| enqueue_entry(&name, true));
    downloads.on_download_folder(|name| enqueue_folder(&name));
    downloads.on_copy_link(|name| copy_link(&name));
    downloads.on_cancel(cancel);
    downloads.on_retry(retry);
    downloads.on_show_in_finder(show_in_finder);
    downloads.on_remove_transfer(remove_transfer);
    downloads.on_clear_finished(clear_finished);
    downloads.on_toggle_queue(toggle_queue);
    downloads.on_change_folder(change_folder);
    downloads.on_test_connection(test_connection);
    downloads.on_submit_form(submit_form);
    downloads.on_cancel_form(cancel_form);
    downloads.on_remove_from_form(remove_from_form);
    downloads.on_check_key(check_key);
    downloads.on_check_key_transfer(check_key_transfer);
    downloads.on_trust_key(trust_key);
    downloads.on_refuse_key(refuse_key);
    downloads.on_copy_fingerprint(|which| copy_fingerprint(&which));
    downloads.on_forget_key(forget_key);
    downloads.on_upload(upload);
    downloads.on_rename_entry(|name| rename_entry(&name));
    downloads.on_commit_rename(commit_rename);
    downloads.on_cancel_rename(cancel_rename);
    downloads.on_delete_entry(|name| delete_entry(&name));
    downloads.on_delete_selected(delete_selected);
    downloads.on_confirm_delete(confirm_delete);
    downloads.on_move_entry(|name| move_entry(&name));
    downloads.on_move_selected(move_selected);
    downloads.on_move_go_to(move_go_to);
    downloads.on_move_open_folder(|name| move_open_folder(&name));
    downloads.on_move_new_folder(move_new_folder);
    downloads.on_commit_new_folder(commit_new_folder);
    downloads.on_cancel_new_folder(cancel_new_folder);
    downloads.on_move_here(move_here);
    downloads.on_resolve_conflict(|answer| resolve_conflict(&answer));
    downloads.on_close_dialog(close_dialog);
    downloads.on_dismiss_change_failed(dismiss_change_failed);
    downloads.on_show_upload(show_upload);
}

// ---- reaching the state and the page ----------------------------------------------------------

/// Change the state (never from within another `with_state`).
fn with_state<R>(change: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|state| state.borrow_mut().as_mut().map(change))
}

/// Change the page, when the window still exists.
fn with_downloads(change: impl FnOnce(&Downloads)) {
    let ui = STATE.with(|state| state.borrow().as_ref().and_then(|state| state.ui.upgrade()));
    if let Some(ui) = ui {
        change(&ui.global::<Downloads>());
    }
}

/// The worker, started when first needed.
fn send(task: Task) {
    let worker = with_state(|state| state.worker.get_or_insert_with(start_worker).clone());
    if let Some(worker) = worker
        && worker.send(task).is_err()
    {
        eprintln!("downloads: the worker thread is gone");
        with_state(|state| state.worker = None);
    }
}

fn start_worker() -> Sender<Task> {
    let (sender, tasks) = mpsc::channel::<Task>();
    let started = std::thread::Builder::new().name("servers".into()).spawn(move || {
        let http = http::client();
        for task in tasks {
            match task {
                Task::List { request, remote, path, ask_changes } => {
                    let result = remote.list(http, &path);
                    let changes = (ask_changes && result.is_ok()).then(|| remote.changes(http));
                    let _ = slint::invoke_from_event_loop(move || listed(request, result, changes));
                }
                Task::Check { index, remote } => {
                    let result = remote.check(http);
                    let changes = result.is_ok().then(|| remote.changes(http));
                    let _ = slint::invoke_from_event_loop(move || checked(index, result, changes));
                }
                Task::Test { request, remote } => {
                    let result = test(http, &remote);
                    let _ = slint::invoke_from_event_loop(move || tested(request, result));
                }
                Task::Browse { request, remote, path } => {
                    let result = remote.list(http, &path);
                    let _ = slint::invoke_from_event_loop(move || browsed(request, result));
                }
                Task::Count { request, remote, path, slot } => {
                    let count = count_entries(http, &remote, &path);
                    let _ = slint::invoke_from_event_loop(move || counted(request, slot, count));
                }
            }
        }
    });
    if let Err(err) = started {
        eprintln!("downloads: cannot start the servers' thread: {err}");
    }
    sender
}

/// Test connection, on the worker: the root's listing, and whether the first file comes in ranges.
fn test(http: &http::Http, remote: &Remote) -> Result<TestResult, remote::Error> {
    let entries = remote.list(http, &[])?;
    let folders = entries.iter().filter(|entry| entry.is_dir).count() as i32;
    let files = entries.len() as i32 - folders;
    let ranges = entries.iter().find(|entry| !entry.is_dir && entry.size > 0).and_then(|file| remote.ranges(http, &file.name));
    Ok(TestResult { folders, files, ranges, changes: remote.changes(http) })
}

/// How many entries the folder holds, all the way down, for the Delete dialog: each folder
/// listed once, MOST_COUNTED_FOLDERS of them at most (a larger tree gives the count so far).
fn count_entries(http: &http::Http, remote: &Remote, path: &[String]) -> u64 {
    let mut pending: VecDeque<Vec<String>> = VecDeque::from([path.to_vec()]);
    let (mut count, mut listed) = (0u64, 0usize);
    while let Some(folder) = pending.pop_front() {
        if listed >= MOST_COUNTED_FOLDERS {
            break;
        }
        listed += 1;
        let Ok(entries) = remote.list(http, &folder) else { break };
        for entry in entries {
            count += 1;
            if entry.is_dir && webdav::is_plain_name(&entry.name) {
                let mut within = folder.clone();
                within.push(entry.name);
                pending.push_back(within);
            }
        }
    }
    count
}

// ---- the servers ------------------------------------------------------------------------------

/// The page is shown: every server is asked whether it answers, and the first is looked at when
/// none is yet.
fn opened() {
    let (current, checks) = with_state(|state| {
        let mut checks = Vec::new();
        for (index, server) in state.servers.iter_mut().enumerate() {
            if state.current != Some(index) {
                server.status = ServerStatus::Checking;
                checks.push(Task::Check { index, remote: server.remote() });
            }
        }
        (state.current, checks)
    })
    .unwrap_or((None, Vec::new()));
    for check in checks {
        send(check);
    }
    show_servers();
    if current.is_none() {
        let any = with_state(|state| !state.servers.is_empty()).unwrap_or(false);
        if any {
            select_server(0);
        } else {
            show_folder();
        }
    } else {
        show_folder();
    }
}

/// Look at a server, at its root.
fn select_server(index: i32) {
    let Ok(index) = usize::try_from(index) else { return };
    let known = with_state(|state| {
        if index >= state.servers.len() {
            return false;
        }
        state.current = Some(index);
        state.path.clear();
        state.entries.clear();
        state.selected.clear();
        state.filter.clear();
        state.editing = None;
        true
    })
    .unwrap_or(false);
    if !known {
        return;
    }
    with_downloads(|downloads| {
        downloads.set_form_open(false);
        downloads.set_filter(SharedString::new());
    });
    list();
}

fn checked(index: usize, result: Result<(), remote::Error>, changes: Option<Changes>) {
    with_state(|state| {
        if let Some(server) = state.servers.get_mut(index) {
            if server.status == ServerStatus::Checking {
                server.status = status_of(&result.err());
            }
            if let Some(changes) = changes {
                server.changes = changes;
                server.changes_asked = true;
            }
        }
    });
    show_servers();
    show_write_state();
}

/// What a server's answer says about it: nothing bad, the password, or no connection.
fn status_of(error: &Option<remote::Error>) -> ServerStatus {
    match error {
        None => ServerStatus::Connected,
        Some(remote::Error::Unauthorized) => ServerStatus::PasswordRejected,
        Some(remote::Error::Unreachable(_)) => ServerStatus::NotReachable,
        Some(remote::Error::KeyUnknown(_)) => ServerStatus::KeyUnchecked,
        Some(remote::Error::KeyChanged(_)) => ServerStatus::KeyChanged,
        // The root may be forbidden or odd while the server is there.
        Some(_) => ServerStatus::Connected,
    }
}

fn show_servers() {
    let (rows, current) = with_state(|state| (state.servers.iter().map(Server::row).collect::<Vec<_>>(), state.current)).unwrap_or_default();
    with_downloads(|downloads| {
        downloads.set_servers(ModelRc::new(VecModel::from(rows)));
        downloads.set_current(current.map_or(-1, |index| index as i32));
    });
    crate::toolbox::servers_changed();
}

fn copy_address(index: i32) {
    let url = with_state(|state| usize::try_from(index).ok().and_then(|index| state.servers.get(index)).map(|server| server.remote().folder_link(&[]))).flatten();
    if let Some(url) = url
        && let Err(err) = platform::copy_text(&url)
    {
        eprintln!("downloads: cannot copy: {err}");
    }
}

// ---- the folder -------------------------------------------------------------------------------

/// Ask the worker for the folder looked at: the spinner first, then its entries.
fn list() {
    list_with(false);
}

/// The same; `quietly` keeps the entries shown until the new ones come (after a change on the
/// server, so the listing does not blink).
fn list_with(quietly: bool) {
    let task = with_state(|state| {
        let index = state.current?;
        let server = state.servers.get_mut(index)?;
        if server.status != ServerStatus::Connected {
            server.status = ServerStatus::Checking;
        }
        state.request += 1;
        state.list_quietly = quietly && matches!(state.folder, Folder::Listed);
        if !state.list_quietly {
            state.folder = Folder::Loading;
        }
        state.renaming = None;
        let ask_changes = !server.changes_asked;
        Some(Task::List { request: state.request, remote: server.remote(), path: state.path.clone(), ask_changes })
    })
    .flatten();
    with_downloads(|downloads| downloads.set_renaming(SharedString::new()));
    let Some(task) = task else {
        show_folder();
        return;
    };
    send(task);
    show_servers();
    show_folder();
}

fn listed(request: u64, result: Result<Vec<remote::Entry>, remote::Error>, changes: Option<Changes>) {
    let language = STATE
        .with(|state| state.borrow().as_ref().and_then(|state| state.ui.upgrade()))
        .map(|ui| ui.global::<AppState>().get_language().to_string())
        .unwrap_or_else(|| "en".to_string());
    let order = platform::NameOrder::new(&language);
    let question = with_state(|state| {
        if request != state.request {
            return None;
        }
        let status = status_of(&result.as_ref().err().cloned());
        if let Some(server) = state.current.and_then(|index| state.servers.get_mut(index)) {
            server.status = status;
            if let Some(changes) = changes {
                server.changes = changes;
                server.changes_asked = true;
            }
        }
        match result {
            Ok(mut entries) => {
                entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| order.compare(&a.name, &b.name)));
                // What was chosen and is gone is not chosen any more; a finished upload's Show
                // chooses its file.
                state.selected.retain(|name| entries.iter().any(|entry| !entry.is_dir && entry.name == *name));
                if let Some(reveal) = state.reveal.take()
                    && entries.iter().any(|entry| !entry.is_dir && entry.name == reveal)
                {
                    state.selected.clear();
                    state.selected.insert(reveal);
                }
                if state.renaming.as_ref().is_some_and(|name| !entries.iter().any(|entry| entry.name == *name)) {
                    state.renaming = None;
                }
                state.entries = entries;
                state.folder = Folder::Listed;
                None
            }
            Err(err) => {
                state.entries.clear();
                state.renaming = None;
                let server = state.current.and_then(|index| state.servers.get(index));
                // The server's key is to be checked: the folder waits, and the user is asked.
                if let Some((changed, shown)) = key_problem(&err) {
                    state.folder = Folder::KeyWait { changed };
                    return server.and_then(|server| server.ssh_host().map(|(host, port)| (host, port, server.name.clone(), shown)));
                }
                let (problem, detail) = problem_of(&err, &server.map(Server::host).unwrap_or_default());
                state.folder = Folder::Error(problem, detail);
                None
            }
        }
    })
    .flatten();
    let renaming = with_state(|state| state.renaming.clone()).flatten();
    if renaming.is_none() {
        with_downloads(|downloads| downloads.set_renaming(SharedString::new()));
    }
    show_servers();
    show_folder();
    if let Some((host, port, name, shown)) = question {
        ask_key(host, port, name, shown, KeyThen::List);
    }
}

/// Why a server could not be read, as the page says it.
fn problem_of(error: &remote::Error, host: &str) -> (Problem, String) {
    match error {
        remote::Error::Unauthorized => (Problem::Password, String::new()),
        remote::Error::Forbidden => (Problem::Forbidden, String::new()),
        remote::Error::Status(code) => (Problem::Http, code.to_string()),
        remote::Error::Unreachable(_) => (Problem::Unreachable, host.to_string()),
        remote::Error::Unreadable(_) => (Problem::Unreadable, String::new()),
        remote::Error::Missing => (Problem::Missing, String::new()),
        remote::Error::KeyChanged(_) => (Problem::KeyChanged, String::new()),
        remote::Error::KeyUnknown(_) => (Problem::KeyUnchecked, String::new()),
        remote::Error::Refused(words) => (Problem::Http, words.clone()),
    }
}

/// The entries the filter lets through.
fn visible<'a>(state: &'a State) -> impl Iterator<Item = &'a remote::Entry> + 'a {
    let filter = state.filter.trim().to_lowercase();
    state.entries.iter().filter(move |entry| filter.is_empty() || entry.name.to_lowercase().contains(&filter))
}

/// The folder looked at, as the page shows it: the way here, its entries, what is chosen.
fn show_folder() {
    struct Shown {
        crumbs: Vec<SharedString>,
        title: String,
        state: FolderState,
        problem: Problem,
        detail: String,
        key_wait_changed: bool,
        rows: Vec<FolderEntry>,
        has_entries: bool,
        selected_count: i32,
        selected_size: u64,
        can_open: bool,
        all_selected: bool,
        some_selected: bool,
    }
    let shown = with_state(|state| {
        let server_name = state.current.and_then(|index| state.servers.get(index)).map(|server| server.name.clone()).unwrap_or_default();
        // The path as the page shows it, "Home NAS /Movies/2026": the server, then every folder.
        let mut crumbs = vec![SharedString::from(server_name.as_str())];
        crumbs.extend(state.path.iter().map(|folder| SharedString::from(folder.as_str())));
        let title = state.path.last().cloned().unwrap_or(server_name);
        let (folder_state, problem, detail) = match &state.folder {
            Folder::Loading => (FolderState::Loading, Problem::None, String::new()),
            Folder::Listed => (FolderState::Listed, Problem::None, String::new()),
            Folder::Error(problem, detail) => (FolderState::Error, *problem, detail.clone()),
            Folder::KeyWait { .. } => (FolderState::KeyWait, Problem::None, String::new()),
        };
        let now = Local::now();
        let rows: Vec<FolderEntry> = visible(state)
            .map(|entry| FolderEntry {
                name: entry.name.clone().into(),
                folder: entry.is_dir,
                icon: (if entry.is_dir { "folder" } else { icon_for(&entry.name) }).into(),
                date: entry.modified.map(|unix| date_text(unix, now.year())).unwrap_or_default().into(),
                size: (if entry.is_dir { String::new() } else { size_text(entry.size) }).into(),
                selected: !entry.is_dir && state.selected.contains(&entry.name),
            })
            .collect();
        let files: Vec<&remote::Entry> = visible(state).filter(|entry| !entry.is_dir).collect();
        let chosen: Vec<&remote::Entry> = state.entries.iter().filter(|entry| !entry.is_dir && state.selected.contains(&entry.name)).collect();
        let chosen_shown = files.iter().filter(|entry| state.selected.contains(&entry.name)).count();
        Shown {
            crumbs,
            title,
            state: folder_state,
            problem,
            detail,
            key_wait_changed: matches!(state.folder, Folder::KeyWait { changed: true }),
            rows,
            has_entries: !state.entries.is_empty(),
            selected_count: chosen.len() as i32,
            selected_size: chosen.iter().map(|entry| entry.size).sum(),
            can_open: chosen.len() == 1 && is_media(icon_for(&chosen[0].name)),
            all_selected: !files.is_empty() && chosen_shown == files.len(),
            some_selected: chosen_shown > 0 && chosen_shown < files.len(),
        }
    });
    let Some(shown) = shown else { return };
    with_downloads(|downloads| {
        downloads.set_path_label(shown.crumbs.iter().map(SharedString::as_str).collect::<Vec<_>>().join(" › ").into());
        downloads.set_crumbs(ModelRc::new(VecModel::from(shown.crumbs)));
        downloads.set_title(shown.title.into());
        downloads.set_folder_state(shown.state);
        downloads.set_key_wait_changed(shown.key_wait_changed);
        downloads.set_problem(shown.problem);
        downloads.set_problem_detail(shown.detail.into());
        downloads.set_entries(ModelRc::new(VecModel::from(shown.rows)));
        downloads.set_has_entries(shown.has_entries);
        downloads.set_selected_count(shown.selected_count);
        downloads.set_selected_size(size_text(shown.selected_size).into());
        downloads.set_can_open(shown.can_open);
        downloads.set_all_selected(shown.all_selected);
        downloads.set_some_selected(shown.some_selected);
    });
    show_write_state();
}

fn open_folder(name: &str) {
    let known = with_state(|state| {
        if !state.entries.iter().any(|entry| entry.is_dir && entry.name == name) {
            return false;
        }
        state.path.push(name.to_string());
        state.selected.clear();
        state.filter.clear();
        true
    })
    .unwrap_or(false);
    if known {
        with_downloads(|downloads| downloads.set_filter(SharedString::new()));
        list();
    }
}

/// A crumb: 0 is the server's root, then the folders above the one looked at.
fn go_to(index: i32) {
    let Ok(index) = usize::try_from(index) else { return };
    with_state(|state| {
        state.path.truncate(index);
        state.selected.clear();
        state.filter.clear();
    });
    with_downloads(|downloads| downloads.set_filter(SharedString::new()));
    list();
}

fn set_filter(text: &str) {
    with_state(|state| state.filter = text.to_string());
    show_folder();
}

fn toggle(name: &str) {
    with_state(|state| {
        if !state.selected.remove(name) && state.entries.iter().any(|entry| !entry.is_dir && entry.name == name) {
            state.selected.insert(name.to_string());
        }
    });
    show_folder();
}

/// Every file shown chosen; when they all are already, none of them.
fn toggle_all() {
    with_state(|state| {
        let files: Vec<String> = visible(state).filter(|entry| !entry.is_dir).map(|entry| entry.name.clone()).collect();
        if !files.is_empty() && files.iter().all(|name| state.selected.contains(name)) {
            for name in &files {
                state.selected.remove(name);
            }
        } else {
            state.selected.extend(files);
        }
    });
    show_folder();
}

fn clear_selection() {
    with_state(|state| state.selected.clear());
    show_folder();
}

fn copy_link(name: &str) {
    let url = with_state(|state| {
        let server = state.servers.get(state.current?)?;
        let entry = state.entries.iter().find(|entry| entry.name == name)?;
        let remote = server.remote();
        Some(if entry.is_dir {
            let mut path = state.path.clone();
            path.push(entry.name.clone());
            remote.folder_link(&path)
        } else {
            remote.file_link(&state.path, &entry.name)
        })
    })
    .flatten();
    if let Some(url) = url
        && let Err(err) = platform::copy_text(&url)
    {
        eprintln!("downloads: cannot copy: {err}");
    }
}

// ---- the queue --------------------------------------------------------------------------------

/// Where downloads go: the settings' folder, or FinchGram's in the user's Downloads.
fn download_folder(settings: &Settings) -> PathBuf {
    if !settings.downloads.folder.trim().is_empty() {
        return PathBuf::from(settings.downloads.folder.trim());
    }
    dirs::document_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from(".")).join("FinchGram").join("webdav")
}

/// Where a server's files go: a folder named after the server, in small letters, under the
/// download folder.
fn server_folder(settings: &Settings, server_name: &str) -> PathBuf {
    let name: String = server_name.trim().to_lowercase().chars().map(|c| if c == '/' { '-' } else { c }).collect();
    download_folder(settings).join(if name.is_empty() { "server".to_string() } else { name })
}

/// The folder as the page names it: "~/Documents/FinchGram/webdav".
fn folder_label(settings: &Settings) -> String {
    let folder = download_folder(settings);
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = folder.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    folder.display().to_string()
}

/// The chosen files, or with `open` the one chosen photo or video, which is shown once it is here.
fn download_selected(open: bool) {
    let names: Vec<String> = with_state(|state| state.entries.iter().filter(|entry| !entry.is_dir && state.selected.contains(&entry.name)).map(|entry| entry.name.clone()).collect()).unwrap_or_default();
    if open && names.len() != 1 {
        return;
    }
    for name in names {
        enqueue_entry(&name, open);
    }
    clear_selection();
}

/// A file of the folder looked at into the queue; with `open`, shown or revealed once it is here.
fn enqueue_entry(name: &str, open: bool) {
    let added = with_state(|state| {
        let index = state.current?;
        let entry = state.entries.iter().find(|entry| !entry.is_dir && entry.name == name)?.clone();
        // A listing lets no name through that is a path of its own; said again where the path is made.
        if !webdav::is_plain_name(&entry.name) {
            return None;
        }
        let server = state.servers.get(index)?;
        let (remote, server_name) = (server.remote(), server.name.clone());
        // One at a time per file: a second request only asks to open it.
        if let Some(existing) = state
            .queue
            .iter_mut()
            .find(|transfer| transfer.live() && matches!(&transfer.kind, Kind::File { remote: theirs, path, name, .. } if theirs.same_as(&remote) && *path == state.path && *name == entry.name))
        {
            existing.open_when_done |= open;
            return Some(false);
        }
        let (root, dest) = {
            let settings = state.settings.borrow();
            (download_folder(&settings), server_folder(&settings, &server_name).join(&entry.name))
        };
        let done = remote.kept(&dest, &state.path, &entry.name);
        let id = state.next_id;
        state.next_id += 1;
        state.queue.push(Transfer {
            id,
            name: entry.name.clone(),
            icon: icon_for(&entry.name),
            kind: Kind::File { remote, path: state.path.clone(), name: entry.name.clone(), root, dest: dest.clone() },
            state: TransferState::Waiting,
            done,
            total: entry.size,
            samples: VecDeque::new(),
            waiting: 0,
            problem: Problem::None,
            detail: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            result: None,
            open_when_done: open,
            key_shown: None,
        });
        Some(true)
    })
    .flatten();
    if added == Some(true) {
        queued();
    }
}

/// A folder of the folder looked at, with everything in it, into the queue.
fn enqueue_folder(name: &str) {
    let added = with_state(|state| {
        let index = state.current?;
        if !webdav::is_plain_name(name) || !state.entries.iter().any(|entry| entry.is_dir && entry.name == name) {
            return None;
        }
        let server = state.servers.get(index)?;
        let (remote, server_name) = (server.remote(), server.name.clone());
        let mut path = state.path.clone();
        path.push(name.to_string());
        if state.queue.iter().any(|transfer| transfer.live() && matches!(&transfer.kind, Kind::Folder { path: theirs, remote: that, .. } if *theirs == path && that.same_as(&remote))) {
            return Some(false);
        }
        let (root, dest) = {
            let settings = state.settings.borrow();
            (download_folder(&settings), server_folder(&settings, &server_name).join(name))
        };
        let id = state.next_id;
        state.next_id += 1;
        state.queue.push(Transfer {
            id,
            name: name.to_string(),
            icon: "folder",
            kind: Kind::Folder { remote, path, root, dest },
            state: TransferState::Waiting,
            done: 0,
            total: 0,
            samples: VecDeque::new(),
            waiting: 0,
            problem: Problem::None,
            detail: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            result: None,
            open_when_done: false,
            key_shown: None,
        });
        Some(true)
    })
    .flatten();
    if added == Some(true) {
        queued();
    }
}

/// Something joined the queue: it starts when a slot is free, and the queue column comes out.
fn queued() {
    let opened = with_state(|state| {
        let mut settings = state.settings.borrow_mut();
        if settings.downloads.queue_shown {
            return false;
        }
        settings.downloads.queue_shown = true;
        settings.save();
        true
    })
    .unwrap_or(false);
    if opened {
        with_downloads(|downloads| downloads.set_queue_open(true));
    }
    start_next();
    show_queue();
}

/// Start waiting downloads while fewer than MOST_ACTIVE are on their way. One whose destination
/// is in use by a download under way (a file of the same name from elsewhere; a folder and a
/// file within it) waits for that one to finish, so that two never write one `.part` file.
fn start_next() {
    loop {
        let started = with_state(|state| {
            let active = state.queue.iter().filter(|transfer| transfer.state == TransferState::Active).count();
            if active >= MOST_ACTIVE {
                return false;
            }
            let connections = state.settings.borrow().downloads.connections.clamp(1, transfer::MOST_CONNECTIONS as u32) as usize;
            let taken: Vec<PathBuf> = state.queue.iter().filter(|transfer| transfer.state == TransferState::Active).map(|transfer| transfer.kind.dest().to_path_buf()).collect();
            let free = |dest: &Path| !taken.iter().any(|other| other == dest || other.starts_with(dest) || dest.starts_with(other));
            let Some(transfer) = state.queue.iter_mut().find(|transfer| transfer.state == TransferState::Waiting && free(transfer.kind.dest())) else { return false };
            transfer.state = TransferState::Active;
            transfer.samples.clear();
            transfer.cancel = Arc::new(AtomicBool::new(false));
            run(transfer.id, transfer.kind.clone(), transfer.cancel.clone(), connections);
            true
        })
        .unwrap_or(false);
        if !started {
            break;
        }
    }
}

/// The download's own thread: the file, or the folder's files one after another.
fn run(id: i32, kind: Kind, cancel: Arc<AtomicBool>, connections: usize) {
    let started = std::thread::Builder::new().name(format!("download {id}")).spawn(move || {
        let http = http::client();
        let streams = transfer::Streams::new(connections);
        let progress = move |progress: transfer::Progress| {
            let _ = slint::invoke_from_event_loop(move || progressed(id, progress));
        };
        let result = match kind {
            Kind::File { remote, path, name, root, dest } => remote.download(http, &path, &name, &root, &dest, &streams, &cancel, &progress),
            Kind::Folder { remote, path, root, dest } => download_tree(http, &remote, &path, &root, &dest, &streams, &cancel, &progress),
            Kind::Upload { remote, path, name, local, size, .. } => remote
                .upload(http, &local, size, &path, &name, &cancel, &|sent| progress(transfer::Progress { done: sent, total: size, protocol: transfer::Protocol::Unknown, waiting: 0 }))
                .map(|()| local),
        };
        let _ = slint::invoke_from_event_loop(move || finished(id, result));
    });
    if let Err(err) = started {
        eprintln!("downloads: cannot start a download thread: {err}");
        let _ = slint::invoke_from_event_loop(move || finished(id, Err(transfer::Error::Disk(err.to_string()))));
    }
}

/// Files of a folder fetched at once: enough to hide a server's half second per request behind
/// one another when the files are small; the transfer's budget of streams caps the large ones.
const FILES_AT_ONCE: usize = 6;
/// Folders listed at once while a tree is walked.
const LISTINGS_AT_ONCE: usize = 8;
/// A folder download lists at most this many files, and goes at most this deep under the
/// folder: a server that answers every listing with more folders would otherwise be followed,
/// and its entries kept, for as long as it likes.
const MOST_TREE_FILES: usize = 100_000;
const MOST_TREE_DEPTH: usize = 64;

/// A file found in a folder's tree: where it is within the folder, its name, its size.
struct TreeFile {
    folder: Vec<String>,
    name: String,
    size: u64,
}

impl TreeFile {
    /// Where the file goes under `dest`: its folders and its name, each one component of the
    /// path. A listing lets no name through that is a path of its own (webdav::is_plain_name);
    /// this says so again before anything is written, since a name that is not a name would put
    /// the file wherever the server chose.
    fn path_under(&self, dest: &Path) -> Option<PathBuf> {
        let mut path = dest.to_path_buf();
        for part in self.folder.iter().chain(std::iter::once(&self.name)) {
            if !webdav::is_plain_name(part) {
                return None;
            }
            path.push(part);
        }
        Some(path)
    }
}

/// A folder and everything in it, into `dest`, under the download folder `root`: the tree listed
/// first, several folders at a time, the total growing on the page as files are found; then the
/// files, several at a time over the transfer's budget of streams. A file already there with its
/// size is not fetched again: a stopped folder goes on where it was.
#[allow(clippy::too_many_arguments)]
fn download_tree(
    http: &'static http::Http,
    remote: &Remote,
    path: &[String],
    root: &Path,
    dest: &Path,
    streams: &transfer::Streams,
    cancel: &AtomicBool,
    progress: &(dyn Fn(transfer::Progress) + Sync),
) -> Result<PathBuf, transfer::Error> {
    let files = http.block_on(list_tree(http, remote, path, cancel, progress))?;
    let total: u64 = files.iter().map(|file| file.size).sum();
    log::line(format!("folder {}: {} files, {:.1} MB", log::shown(&remote.folder_link(path)), files.len(), total as f64 / (1024.0 * 1024.0)));
    transfer::check_space(dest, total)?;
    progress(transfer::Progress { done: 0, total, protocol: transfer::Protocol::Unknown, waiting: 0 });
    let workers = FILES_AT_ONCE.min(files.len().max(1));
    let queue = Mutex::new(VecDeque::from(files));
    // Bytes of the files done or skipped, and of each worker's file so far: their sum is where
    // the folder is.
    let finished = AtomicU64::new(0);
    let in_flight: Mutex<Vec<(u64, u16)>> = Mutex::new(vec![(0, 0); workers]);
    let failure: Mutex<Option<transfer::Error>> = Mutex::new(None);
    let report = |slot: usize, done: u64, waiting: u16| {
        let mut flight = in_flight.lock().unwrap();
        flight[slot] = (done, waiting);
        let under_way: u64 = flight.iter().map(|(done, _)| *done).sum();
        let waiting = flight.iter().map(|(_, waiting)| *waiting).max().unwrap_or(0);
        drop(flight);
        progress(transfer::Progress { done: finished.load(Ordering::Relaxed) + under_way, total, protocol: transfer::Protocol::Unknown, waiting });
    };
    std::thread::scope(|scope| {
        for slot in 0..workers {
            let (queue, finished, failure, report) = (&queue, &finished, &failure, &report);
            scope.spawn(move || {
                loop {
                    if cancel.load(Ordering::Relaxed) || failure.lock().unwrap().is_some() {
                        return;
                    }
                    let Some(file) = queue.lock().unwrap().pop_front() else { return };
                    let mut whole = path.to_vec();
                    whole.extend(file.folder.iter().cloned());
                    let Some(file_dest) = file.path_under(dest) else {
                        let mut failed = failure.lock().unwrap();
                        if failed.is_none() {
                            *failed = Some(transfer::Error::Protocol(format!("the server named a file in a way that is not a name: {:?}", file.name)));
                        }
                        return;
                    };
                    let already = std::fs::metadata(&file_dest).map(|meta| meta.is_file() && meta.len() == file.size).unwrap_or(false);
                    if !already
                        && let Err(err) = remote.download(http, &whole, &file.name, root, &file_dest, streams, cancel, &|progress| report(slot, progress.done, progress.waiting))
                    {
                        let mut failed = failure.lock().unwrap();
                        if failed.is_none() {
                            *failed = Some(err);
                        }
                        return;
                    }
                    finished.fetch_add(file.size, Ordering::Relaxed);
                    report(slot, 0, 0);
                }
            });
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return Err(transfer::Error::Cancelled);
    }
    if let Some(err) = failure.lock().unwrap().take() {
        return Err(err);
    }
    Ok(dest.to_path_buf())
}

/// The files of the tree under `path`, listed several folders at a time; the total so far goes to
/// the page as it grows.
async fn list_tree(http: &'static http::Http, remote: &Remote, path: &[String], cancel: &AtomicBool, progress: &(dyn Fn(transfer::Progress) + Sync)) -> Result<Vec<TreeFile>, transfer::Error> {
    let mut files = Vec::new();
    let mut pending: VecDeque<Vec<String>> = VecDeque::from([Vec::new()]);
    // Each folder once, whatever a server lists twice.
    let mut seen: BTreeSet<Vec<String>> = BTreeSet::from([Vec::new()]);
    let mut set = tokio::task::JoinSet::new();
    let mut total: u64 = 0;
    loop {
        while set.len() < LISTINGS_AT_ONCE
            && let Some(folder) = pending.pop_front()
        {
            let remote = remote.clone();
            let mut whole = path.to_vec();
            whole.extend(folder.iter().cloned());
            set.spawn_on(async move { (folder, remote.list_in(http, &whole).await) }, http.handle());
        }
        let Some(next) = set.join_next().await else { break };
        let (folder, listed) = next.map_err(|err| transfer::Error::Connection(format!("a listing's task failed: {err}")))?;
        for entry in listed.map_err(remote::transfer_error)? {
            let mut within = folder.clone();
            if entry.is_dir {
                within.push(entry.name);
                if within.len() > MOST_TREE_DEPTH {
                    return Err(transfer::Error::Protocol(format!("the folder's folders go more than {MOST_TREE_DEPTH} deep; download them one at a time")));
                }
                if seen.insert(within.clone()) {
                    pending.push_back(within);
                }
            } else {
                if files.len() >= MOST_TREE_FILES {
                    return Err(transfer::Error::Protocol(format!("the folder holds more than {MOST_TREE_FILES} files; download its folders one at a time")));
                }
                total += entry.size;
                files.push(TreeFile { folder: within, name: entry.name, size: entry.size });
            }
        }
        progress(transfer::Progress { done: 0, total, protocol: transfer::Protocol::Unknown, waiting: 0 });
        if cancel.load(Ordering::Relaxed) {
            return Err(transfer::Error::Cancelled);
        }
    }
    Ok(files)
}

fn progressed(id: i32, progress: transfer::Progress) {
    let row = with_state(|state| {
        let transfer = state.queue.iter_mut().find(|transfer| transfer.id == id && transfer.state == TransferState::Active)?;
        let done = progress.done;
        transfer.done = done;
        if progress.total > 0 {
            transfer.total = progress.total;
        }
        transfer.waiting = progress.waiting;
        let now = Instant::now();
        transfer.samples.push_back((now, done));
        while transfer.samples.len() > 2 && transfer.samples.front().is_some_and(|(at, _)| now.duration_since(*at) > SPEED_WINDOW) {
            transfer.samples.pop_front();
        }
        Some(transfer.row())
    })
    .flatten();
    let Some(row) = row else { return };
    with_downloads(|downloads| {
        let rows = downloads.get_queue();
        if let Some(index) = (0..rows.row_count()).find(|index| rows.row_data(*index).is_some_and(|shown| shown.id == id)) {
            rows.set_row_data(index, row);
        }
        downloads.set_active_count(active_count());
    });
}

fn finished(id: i32, result: Result<PathBuf, transfer::Error>) {
    let (open, key_stop, uploaded) = with_state(|state| {
        let transfer = state.queue.iter_mut().find(|transfer| transfer.id == id && transfer.state == TransferState::Active)?;
        // An upload that is done, or stopped, changes what its folder shows: it is listed again
        // when it is the one looked at.
        let uploaded = match &transfer.kind {
            Kind::Upload { remote, path, .. } if !matches!(result, Err(transfer::Error::Cancelled)) => Some((remote.clone(), path.clone())),
            _ => None,
        };
        match result {
            Ok(path) => {
                transfer.state = TransferState::Done;
                transfer.done = transfer.total;
                transfer.result = Some(path.clone());
                Some((transfer.open_when_done.then_some((path, transfer.icon)), None, uploaded))
            }
            Err(transfer::Error::Cancelled) => Some((None, None, None)),
            Err(err) => {
                transfer.state = TransferState::Stopped;
                let (problem, mut detail) = match &err {
                    transfer::Error::Unauthorized => (Problem::Password, String::new()),
                    transfer::Error::Forbidden => (Problem::Forbidden, String::new()),
                    transfer::Error::Status(code) => (Problem::Http, code.to_string()),
                    transfer::Error::Connection(_) => (Problem::Connection, String::new()),
                    transfer::Error::Disk(_) => (Problem::Disk, String::new()),
                    transfer::Error::Protocol(_) => (Problem::Unreadable, String::new()),
                    transfer::Error::Missing => (Problem::Missing, String::new()),
                    transfer::Error::KeyChanged(_) => (Problem::KeyChanged, String::new()),
                    transfer::Error::KeyUnknown(_) => (Problem::KeyUnchecked, String::new()),
                    transfer::Error::Refused(words) => (Problem::Http, words.clone()),
                    transfer::Error::Cancelled => (Problem::None, String::new()),
                };
                // An upload's row says what the server refused in the server's words.
                if transfer.kind.is_upload()
                    && let Some(refusal) = remote::remote_error(&err).filter(remote::Error::is_refusal)
                {
                    detail = refusal.refusal(transfer.kind.remote().protocol());
                }
                eprintln!("downloads: {} stopped: {err}", transfer.name);
                transfer.problem = problem;
                transfer.detail = detail;
                // Stopped for the server's key to be checked: the key it showed is kept for the
                // row's Check key, and the server's row says so too. No dialog comes up on its
                // own: the download started in the background.
                transfer.key_shown = match &err {
                    transfer::Error::KeyChanged(shown) | transfer::Error::KeyUnknown(shown) => Some(shown.clone()),
                    _ => None,
                };
                let key_stop = transfer.key_shown.as_ref().and_then(|_| transfer.kind.remote().ssh_host().map(|(host, port)| (host.to_string(), port, problem == Problem::KeyChanged)));
                Some((None, key_stop, uploaded))
            }
        }
    })
    .flatten()
    .unwrap_or((None, None, None));
    if let Some((remote, path)) = uploaded
        && looking_at(&remote, &path)
    {
        list_with(true);
    }
    if let Some((host, port, changed)) = key_stop {
        with_state(|state| {
            for index in servers_at(state, &host, port) {
                state.servers[index].status = if changed { ServerStatus::KeyChanged } else { ServerStatus::KeyUnchecked };
            }
        });
        show_servers();
    }
    start_next();
    show_queue();
    if let Some((path, icon)) = open {
        open_downloaded(&path, icon);
    }
}

/// A downloaded photo or video in the media viewer; anything else in the Finder.
fn open_downloaded(path: &Path, icon: &str) {
    let path = path.to_string_lossy().into_owned();
    if is_media(icon) {
        telegram::view_downloaded_file(&path, icon == "film-strip");
    } else {
        platform::reveal_file(&path);
    }
}

fn active_count() -> i32 {
    with_state(|state| state.queue.iter().filter(|transfer| transfer.state == TransferState::Active).count() as i32).unwrap_or(0)
}

fn show_queue() {
    let (rows, done) = with_state(|state| {
        let mut transfers: Vec<&Transfer> = state.queue.iter().collect();
        transfers.sort_by_key(|transfer| transfer.rank());
        let done = transfers.iter().filter(|transfer| transfer.state == TransferState::Done).count() as i32;
        (transfers.iter().map(|transfer| transfer.row()).collect::<Vec<_>>(), done)
    })
    .unwrap_or_default();
    let active = active_count();
    let uploads = with_state(|state| state.queue.iter().filter(|transfer| transfer.state == TransferState::Active && transfer.kind.is_upload()).count() as i32).unwrap_or(0);
    with_downloads(|downloads| {
        downloads.set_queue(ModelRc::new(VecModel::from(rows)));
        downloads.set_active_count(active);
        downloads.set_active_uploads(uploads);
        downloads.set_done_count(done);
    });
    // Busy while a file is on its way: a window in the background is no reason to slow down.
    with_state(|state| {
        if active > 0 && state.activity.is_none() {
            state.activity = Some(platform::begin_activity("Transferring files"));
        } else if active == 0 {
            state.activity = None;
        }
    });
}

fn cancel(id: i32) {
    with_state(|state| {
        if let Some(index) = state.queue.iter().position(|transfer| transfer.id == id && transfer.live()) {
            state.queue[index].cancel.store(true, Ordering::Relaxed);
            state.queue.remove(index);
        }
    });
    start_next();
    show_queue();
}

fn retry(id: i32) {
    with_state(|state| {
        if let Some(transfer) = state.queue.iter_mut().find(|transfer| transfer.id == id && transfer.state == TransferState::Stopped) {
            transfer.state = TransferState::Waiting;
            transfer.problem = Problem::None;
            transfer.detail.clear();
            transfer.key_shown = None;
            // A download goes on from what is kept; an upload starts over.
            transfer.done = match &transfer.kind {
                Kind::File { remote, path, name, dest, .. } => remote.kept(dest, path, name),
                _ => 0,
            };
        }
    });
    start_next();
    show_queue();
}

fn show_in_finder(id: i32) {
    let path = with_state(|state| state.queue.iter().find(|transfer| transfer.id == id).and_then(|transfer| transfer.result.clone())).flatten();
    if let Some(path) = path {
        platform::reveal_file(&path.to_string_lossy());
    }
}

/// A download off the list. A stopped one's partial files go with it: Cancel keeps them for a
/// Retry, Remove is giving up on it.
fn remove_transfer(id: i32) {
    let partial = with_state(|state| {
        let transfer = state.queue.iter().find(|transfer| transfer.id == id && !transfer.live())?;
        if transfer.state != TransferState::Stopped {
            return None;
        }
        transfer.kind.partials().map(|(root, dest)| (root.to_path_buf(), dest.to_path_buf()))
    })
    .flatten();
    if let Some((root, dest)) = partial {
        transfer::remove_partials(&root, &dest);
    }
    with_state(|state| state.queue.retain(|transfer| transfer.id != id || transfer.live()));
    show_queue();
}

/// Every finished download off the list at once; the files stay where they are.
fn clear_finished() {
    with_state(|state| state.queue.retain(|transfer| transfer.state != TransferState::Done));
    show_queue();
}

fn toggle_queue() {
    let shown = with_state(|state| {
        let mut settings = state.settings.borrow_mut();
        settings.downloads.queue_shown = !settings.downloads.queue_shown;
        settings.save();
        settings.downloads.queue_shown
    });
    if let Some(shown) = shown {
        with_downloads(|downloads| downloads.set_queue_open(shown));
    }
}

fn change_folder() {
    platform::choose_folder(|folder| {
        let Some(folder) = folder else { return };
        let label = with_state(|state| {
            let mut settings = state.settings.borrow_mut();
            settings.downloads.folder = folder.to_string_lossy().into_owned();
            settings.save();
            folder_label(&settings)
        });
        if let Some(label) = label {
            with_downloads(|downloads| downloads.set_folder_label(label.into()));
        }
    });
}

// ---- the form ---------------------------------------------------------------------------------

fn add_server() {
    with_state(|state| state.editing = None);
    with_downloads(|downloads| {
        downloads.set_form_editing(false);
        downloads.set_form_name(SharedString::new());
        downloads.set_form_protocol("webdav".into());
        downloads.set_form_address(SharedString::new());
        downloads.set_form_port(SharedString::new());
        downloads.set_form_username(SharedString::new());
        downloads.set_form_password(SharedString::new());
        downloads.set_form_error(FormError::None);
        downloads.set_test_state(TestState::None);
        downloads.set_test_changes(ChangesSaid::Unknown);
        downloads.set_form_allow_changes(false);
        downloads.set_form_read_only(false);
        downloads.set_form_key_forgotten(false);
        downloads.set_form_open(true);
    });
    show_form_key();
}

fn edit_server(index: i32) {
    let Ok(index) = usize::try_from(index) else { return };
    let server = with_state(|state| {
        let server = state.servers.get(index)?.clone();
        state.editing = Some(index);
        state.current = Some(index);
        Some(server)
    })
    .flatten();
    let Some(server) = server else { return };
    let address = match server.protocol {
        Protocol::Webdav => format!("{}/", server.address),
        _ => server.address.clone(),
    };
    with_downloads(|downloads| {
        downloads.set_current(index as i32);
        downloads.set_form_editing(true);
        downloads.set_form_name(server.name.clone().into());
        downloads.set_form_protocol(server.protocol.id().into());
        downloads.set_form_address(address.into());
        downloads.set_form_port(if server.port == 0 { SharedString::new() } else { server.port.to_string().into() });
        downloads.set_form_username(server.username.clone().into());
        downloads.set_form_password(server.password.clone().into());
        downloads.set_form_error(FormError::None);
        downloads.set_test_state(TestState::None);
        downloads.set_test_changes(ChangesSaid::Unknown);
        downloads.set_form_allow_changes(server.allow_changes);
        downloads.set_form_read_only(server.changes == Changes::ReadOnly);
        downloads.set_form_key_forgotten(false);
        downloads.set_form_open(true);
    });
    show_form_key();
}

/// The form's fields, as typed.
struct Form {
    name: String,
    /// "webdav", "ftp", "ftps", "sftp": the chip chosen.
    protocol: String,
    address: String,
    /// "" for the protocol's default.
    port: String,
    username: String,
    password: String,
    /// The switch: changes on the server allowed.
    allow_changes: bool,
}

fn form_fields() -> Option<Form> {
    let ui = STATE.with(|state| state.borrow().as_ref().and_then(|state| state.ui.upgrade()))?;
    let downloads = ui.global::<Downloads>();
    Some(Form {
        name: downloads.get_form_name().trim().to_string(),
        protocol: downloads.get_form_protocol().to_string(),
        address: downloads.get_form_address().trim().to_string(),
        port: downloads.get_form_port().trim().to_string(),
        username: downloads.get_form_username().trim().to_string(),
        password: downloads.get_form_password().to_string(),
        allow_changes: downloads.get_form_allow_changes(),
    })
}

/// What the form names, checked: the protocol, the address as it is kept (a port written in it
/// taken out), the port (0 for the protocol's default); or what is wrong: a WebDAV address that
/// is not http(s), no host for the others, a port that is not one.
fn form_address(form: &Form) -> Result<(Protocol, String, u16), FormError> {
    let protocol = Protocol::parse(&form.protocol).unwrap_or(Protocol::Webdav);
    let (address, inline) = match protocol {
        Protocol::Webdav => {
            if !webdav::Server::is_address(&form.address) {
                return Err(FormError::Address);
            }
            let inline = webdav::Server::port(&form.address);
            let address = if inline.is_empty() { form.address.clone() } else { form.address.replacen(&format!(":{inline}"), "", 1) };
            (webdav::Server::normalized_url(&address), inline)
        }
        _ => {
            if !remote::is_host_address(&form.address) {
                return Err(FormError::Host);
            }
            let (host, folder) = remote::host_and_folder(&form.address);
            let address = if folder.is_empty() { host } else { format!("{host}/{}", folder.join("/")) };
            (address, remote::inline_port(&form.address))
        }
    };
    let typed = if form.port.is_empty() { inline } else { form.port.clone() };
    let port = if typed.is_empty() {
        0
    } else {
        match typed.parse::<u32>() {
            Ok(port @ 1..=65535) => port as u16,
            _ => return Err(FormError::Port),
        }
    };
    Ok((protocol, address, port))
}

/// The form as a server, checked, for a test or for keeping.
fn form_server(form: Form) -> Result<Server, FormError> {
    let (protocol, address, port) = form_address(&form)?;
    Ok(Server {
        name: form.name,
        protocol,
        address,
        port,
        username: form.username,
        password: form.password,
        status: ServerStatus::Unknown,
        allow_changes: form.allow_changes,
        changes: Changes::Unknown,
        changes_asked: false,
    })
}

fn test_connection() {
    let Some(form) = form_fields() else { return };
    let server = match form_server(form) {
        Ok(server) => server,
        Err(error) => {
            with_downloads(|downloads| downloads.set_form_error(error));
            return;
        }
    };
    let request = with_state(|state| {
        state.test_request += 1;
        state.test_request
    })
    .unwrap_or(0);
    with_downloads(|downloads| {
        downloads.set_form_error(FormError::None);
        downloads.set_test_state(TestState::Testing);
    });
    // The Server key row follows the address as it is typed now.
    show_form_key();
    send(Task::Test { request, remote: server.remote() });
}

fn tested(request: u64, result: Result<TestResult, remote::Error>) {
    let current = with_state(|state| state.test_request).unwrap_or(0);
    if request != current {
        return;
    }
    // The server's key is to be checked first: the user is asked, and the test runs again once
    // they trust it (or says why not, when they do not).
    if let Err(err) = &result
        && let Some((_, shown)) = key_problem(err)
        && let Some(server) = form_fields().and_then(|form| form_server(form).ok())
        && let Some((host, port)) = server.ssh_host()
    {
        with_downloads(|downloads| downloads.set_test_state(TestState::None));
        let name = if server.name.is_empty() { host.clone() } else { server.name.clone() };
        ask_key(host, port, name, shown, KeyThen::Test);
        return;
    }
    let host = form_fields().and_then(|form| form_server(form).ok()).map(|server| server.host()).unwrap_or_default();
    with_downloads(|downloads| match result {
        Ok(found) => {
            downloads.set_test_folders(found.folders);
            downloads.set_test_files(found.files);
            downloads.set_test_probed(found.ranges.is_some());
            downloads.set_test_ranges(found.ranges.unwrap_or(false));
            downloads.set_test_changes(match found.changes {
                Changes::Unknown => ChangesSaid::Unknown,
                Changes::Allowed => ChangesSaid::Allowed,
                Changes::ReadOnly => ChangesSaid::ReadOnly,
            });
            downloads.set_form_read_only(found.changes == Changes::ReadOnly);
            downloads.set_test_state(TestState::Ok);
        }
        Err(err) => {
            let (problem, detail) = problem_of(&err, &host);
            downloads.set_test_problem(problem);
            downloads.set_test_detail(detail.into());
            downloads.set_test_state(TestState::Failed);
        }
    });
}

/// Add, or Save: the server goes into the settings, its password into the Keychain, and it is
/// looked at.
fn submit_form() {
    let Some(form) = form_fields() else { return };
    if form.name.is_empty() {
        with_downloads(|downloads| downloads.set_form_error(FormError::Name));
        return;
    }
    let server = match form_server(form) {
        Ok(server) => server,
        Err(error) => {
            with_downloads(|downloads| downloads.set_form_error(error));
            return;
        }
    };
    let index = with_state(|state| {
        let index = match state.editing.filter(|index| *index < state.servers.len()) {
            Some(index) => {
                // What the server said about changes holds while it is the same server.
                let before = &state.servers[index];
                let same = before.protocol == server.protocol && before.address == server.address && before.port == server.port;
                let (changes, changes_asked) = if same { (before.changes, before.changes_asked) } else { (Changes::Unknown, false) };
                state.servers[index] = Server { changes, changes_asked, ..server };
                index
            }
            None => {
                state.servers.push(server);
                state.servers.len() - 1
            }
        };
        state.editing = None;
        save_servers(state);
        index
    });
    with_downloads(|downloads| {
        downloads.set_form_open(false);
        downloads.set_form_password(SharedString::new());
    });
    if let Some(index) = index {
        select_server(index as i32);
    }
}

fn cancel_form() {
    with_state(|state| state.editing = None);
    with_downloads(|downloads| {
        downloads.set_form_open(false);
        downloads.set_form_password(SharedString::new());
    });
    let current = with_state(|state| state.current).flatten();
    let any = with_state(|state| !state.servers.is_empty()).unwrap_or(false);
    if current.is_none() && any {
        select_server(0);
    } else {
        show_folder();
    }
}

fn remove_from_form() {
    let editing = with_state(|state| state.editing).flatten();
    if let Some(index) = editing {
        remove_server(index as i32);
    }
}

/// The server goes, with its password; the one before it is looked at instead.
fn remove_server(index: i32) {
    let Ok(index) = usize::try_from(index) else { return };
    let next = with_state(|state| {
        if index >= state.servers.len() {
            return None;
        }
        state.servers.remove(index);
        state.editing = None;
        save_servers(state);
        state.current = match state.current {
            Some(current) if current == index => None,
            Some(current) if current > index => Some(current - 1),
            other => other,
        };
        Some(if state.servers.is_empty() { None } else { Some(state.current.unwrap_or(index.min(state.servers.len() - 1)) as i32) })
    })
    .flatten();
    with_downloads(|downloads| {
        downloads.set_form_open(false);
        downloads.set_form_password(SharedString::new());
    });
    match next {
        Some(Some(next)) => select_server(next),
        Some(None) => {
            with_state(|state| {
                state.path.clear();
                state.entries.clear();
                state.selected.clear();
            });
            show_servers();
            show_folder();
        }
        None => {}
    }
}

/// The servers into the database, as they are now.
fn save_servers(state: &State) {
    let records: Vec<database::ServerRecord> = state.servers.iter().map(Server::record).collect();
    if let Err(err) = database::save_servers(&records) {
        eprintln!("downloads: cannot keep the servers in the database: {err}");
    }
}

// ---- the SSH servers' keys --------------------------------------------------------------------
// The design's eighth round: an SFTP server's key is shown to the user before the password goes
// out, the first time, and whenever it is not the one trusted. The key trusted is the database's
// (`ssh_hosts`); the sessions hold it through one slot per server.

thread_local! {
    /// The slot each SFTP server's trusted key is held in (sftp::KnownKey), by host and port,
    /// shared by every `Remote` made for the server, so that trusting or forgetting a key reaches
    /// the sessions under way and the downloads queued. Filled from the database when the server
    /// is first reached for.
    static KNOWN_KEYS: RefCell<HashMap<(String, u16), sftp::KnownKey>> = RefCell::new(HashMap::new());
}

/// The slot of the server at `host`:`port`.
fn known_key_slot(host: &str, port: u16) -> sftp::KnownKey {
    KNOWN_KEYS.with(|slots| {
        slots
            .borrow_mut()
            .entry((host.to_string(), port))
            .or_insert_with(|| Arc::new(Mutex::new(database::ssh_host(host, port).map(|known| known.key))))
            .clone()
    })
}

/// The question the dialog asks: whose key, what the server showed, and what goes on once it is
/// trusted.
struct KeyQuestion {
    host: String,
    port: u16,
    shown: String,
    then: KeyThen,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyThen {
    /// The folder looked at is listed.
    List,
    /// The form's Test connection runs again.
    Test,
    /// Nothing more than what every trust does: the downloads paused on the key go on.
    Queue,
}

/// A refusal that is about the server's key, and the key it showed: not trusted yet (`false`),
/// or another than the one trusted (`true`).
fn key_problem(error: &remote::Error) -> Option<(bool, String)> {
    match error {
        remote::Error::KeyUnknown(shown) => Some((false, shown.clone())),
        remote::Error::KeyChanged(shown) => Some((true, shown.clone())),
        _ => None,
    }
}

/// The servers of the list at `host`:`port` (two entries may name one server).
fn servers_at(state: &State, host: &str, port: u16) -> Vec<usize> {
    state
        .servers
        .iter()
        .enumerate()
        .filter(|(_, server)| server.ssh_host().is_some_and(|(theirs, their_port)| theirs == host && their_port == port))
        .map(|(index, _)| index)
        .collect()
}

/// Ask the user about the key the server at `host`:`port` showed: the dialog, over the page, with
/// the key's fingerprint, and the one trusted before when there is one and it is another.
fn ask_key(host: String, port: u16, name: String, shown: String, then: KeyThen) {
    let known = database::ssh_host(&host, port);
    let changed = known.as_ref().is_some_and(|known| known.key != shown);
    let facts = sftp::key_facts(&shown);
    let old = known.as_ref().and_then(|known| sftp::key_facts(&known.key));
    with_downloads(|downloads| {
        downloads.set_key_ask_changed(changed);
        downloads.set_key_ask_name(name.into());
        downloads.set_key_ask_address(format!("{host}:{port}").into());
        downloads.set_key_ask_type(facts.as_ref().map(|facts| facts.kind.clone()).unwrap_or_default().into());
        downloads.set_key_ask_fingerprint(facts.as_ref().map(|facts| facts.fingerprint.clone()).unwrap_or_else(|| shown.clone()).into());
        downloads.set_key_ask_old_type(old.as_ref().map(|facts| facts.kind.clone()).unwrap_or_default().into());
        downloads.set_key_ask_old_fingerprint(old.as_ref().map(|facts| facts.fingerprint.clone()).or_else(|| known.as_ref().map(|known| known.key.clone())).unwrap_or_default().into());
        downloads.set_key_ask_old_since(known.as_ref().filter(|known| known.first_seen > 0).map(|known| full_date_text(known.first_seen)).unwrap_or_default().into());
        downloads.set_key_copied(SharedString::new());
        downloads.set_key_ask_open(true);
    });
    with_state(|state| state.key_question = Some(KeyQuestion { host, port, shown, then }));
}

/// The user trusts the key asked about: kept, from now, as the server's; the sessions get it,
/// the rows that waited for it go on.
fn trust_key() {
    let Some(question) = with_state(|state| state.key_question.take()).flatten() else { return };
    with_downloads(|downloads| downloads.set_key_ask_open(false));
    if let Err(err) = database::trust_ssh_host(&question.host, question.port, &question.shown) {
        eprintln!("downloads: cannot keep the server's key: {err}");
    }
    *known_key_slot(&question.host, question.port).lock().unwrap() = Some(question.shown.clone());
    // The servers at that address: their rows no longer say the key is in question; those not
    // looked at are asked whether they answer now.
    let checks = with_state(|state| {
        let mut checks = Vec::new();
        for index in servers_at(state, &question.host, question.port) {
            let current = state.current == Some(index);
            let server = &mut state.servers[index];
            if matches!(server.status, ServerStatus::KeyUnchecked | ServerStatus::KeyChanged) {
                server.status = ServerStatus::Checking;
                if !current {
                    checks.push(Task::Check { index, remote: server.remote() });
                }
            }
        }
        checks
    })
    .unwrap_or_default();
    for check in checks {
        send(check);
    }
    show_servers();
    resume_key_paused(&question.host, question.port);
    show_form_key();
    match question.then {
        KeyThen::List => list(),
        KeyThen::Test => test_connection(),
        KeyThen::Queue => {}
    }
}

/// The user does not trust the key: nothing was sent, nothing goes on. The folder keeps saying
/// to check the key, a test says why it did not connect, a download stays paused.
fn refuse_key() {
    let Some(question) = with_state(|state| state.key_question.take()).flatten() else { return };
    with_downloads(|downloads| {
        downloads.set_key_ask_open(false);
        if question.then == KeyThen::Test {
            downloads.set_test_problem(Problem::KeyUnchecked);
            downloads.set_test_detail(SharedString::new());
            downloads.set_test_state(TestState::Failed);
        }
    });
}

/// The folder's Check key: the server is asked for its key again, and the user about it.
fn check_key() {
    list();
}

/// A download's Check key: the user is asked about the key the server showed it.
fn check_key_transfer(id: i32) {
    let asked = with_state(|state| {
        let transfer = state.queue.iter().find(|transfer| transfer.id == id && transfer.state == TransferState::Stopped)?;
        let (host, port) = transfer.kind.remote().ssh_host()?;
        let shown = transfer.key_shown.clone()?;
        let name = servers_at(state, host, port).first().map(|index| state.servers[*index].name.clone()).unwrap_or_else(|| host.to_string());
        Some((host.to_string(), port, name, shown))
    })
    .flatten();
    match asked {
        Some((host, port, name, shown)) => ask_key(host, port, name, shown, KeyThen::Queue),
        None => retry(id),
    }
}

/// The downloads paused on the key of the server at `host`:`port` go on, from where they were.
fn resume_key_paused(host: &str, port: u16) {
    with_state(|state| {
        for transfer in state.queue.iter_mut() {
            let paused = transfer.state == TransferState::Stopped && matches!(transfer.problem, Problem::KeyUnchecked | Problem::KeyChanged);
            if !paused || transfer.kind.remote().ssh_host() != Some((host, port)) {
                continue;
            }
            transfer.state = TransferState::Waiting;
            transfer.problem = Problem::None;
            transfer.detail.clear();
            transfer.key_shown = None;
            if let Kind::File { remote, path, name, dest, .. } = &transfer.kind {
                transfer.done = remote.kept(dest, path, name);
            }
        }
    });
    start_next();
    show_queue();
}

/// Copy a fingerprint of the dialog: "new", the key shown, or "old", the one trusted before. The
/// link says "Copied" for a moment.
fn copy_fingerprint(which: &str) {
    let Some(ui) = STATE.with(|state| state.borrow().as_ref().and_then(|state| state.ui.upgrade())) else { return };
    let downloads = ui.global::<Downloads>();
    let text = if which == "old" { downloads.get_key_ask_old_fingerprint() } else { downloads.get_key_ask_fingerprint() };
    if let Err(err) = platform::copy_text(&text) {
        eprintln!("downloads: cannot copy: {err}");
        return;
    }
    downloads.set_key_copied(which.into());
    with_state(|state| {
        state.copied.start(slint::TimerMode::SingleShot, COPIED_FOR, || with_downloads(|downloads| downloads.set_key_copied(SharedString::new())));
    });
}

/// The form's Forget this key: the server's key goes from the database and from the sessions;
/// the next connection asks the user again.
fn forget_key() {
    let Some((host, port)) = form_ssh_host() else { return };
    if let Err(err) = database::forget_ssh_host(&host, port) {
        eprintln!("downloads: cannot forget the server's key: {err}");
    }
    *known_key_slot(&host, port).lock().unwrap() = None;
    with_downloads(|downloads| {
        downloads.set_form_key_known(false);
        downloads.set_form_key_forgotten(true);
        downloads.set_test_state(TestState::None);
    });
}

/// The SFTP server the form names, by host and port; none for another protocol, or an address
/// that is not one.
fn form_ssh_host() -> Option<(String, u16)> {
    form_server(form_fields()?).ok()?.ssh_host()
}

/// The form's Server key row: the key trusted for the server the form names, if any.
fn show_form_key() {
    let known = form_ssh_host().and_then(|(host, port)| database::ssh_host(&host, port));
    let facts = known.as_ref().and_then(|known| sftp::key_facts(&known.key));
    with_downloads(|downloads| {
        downloads.set_form_key_known(known.is_some());
        downloads.set_form_key_type(facts.as_ref().map(|facts| facts.kind.clone()).unwrap_or_default().into());
        downloads.set_form_key_fingerprint(facts.as_ref().map(|facts| facts.fingerprint.clone()).or_else(|| known.as_ref().map(|known| known.key.clone())).unwrap_or_default().into());
        downloads.set_form_key_since(known.as_ref().filter(|known| known.first_seen > 0).map(|known| full_date_text(known.first_seen)).unwrap_or_default().into());
    });
}

// ---- changes on the server --------------------------------------------------------------------
// The design's ninth round ("写操作"): where the user allows changes on a server (its form's
// switch, in the database), the folder's menu and foot offer Rename, Move to… and Delete…, the
// toolbar Upload, and files dropped on the window go up into the folder looked at. Deleting asks
// first; moving opens a dialog over the server's folders; a name taken at the destination asks
// Replace, Skip or Keep both. Every change goes to the server on a thread of its own, one batch
// at a time, and the folder is listed again when it is done; what the server would not do is
// said under the folder's title. A server that said it only reads (remote::Changes) has the
// items greyed and is not asked.

/// What a batch of changes does.
enum Change {
    /// The entries, from the folder `path`.
    Delete { path: Vec<String>, entries: Vec<remote::Entry> },
    /// The entries from `from` into `to`, each under its new name, replacing what stands there
    /// when told to.
    Move { from: Vec<String>, to: Vec<String>, items: Vec<Moved> },
    /// The entry in `path`, under another name.
    Rename { path: Vec<String>, entry: remote::Entry, new_name: String },
    /// A new folder in `path`.
    MakeFolder { path: Vec<String>, name: String },
}

/// One entry of a move: under what name it arrives, and what it replaces there (a file, `Some(false)`;
/// a folder, `Some(true)`), if anything.
struct Moved {
    entry: remote::Entry,
    new_name: String,
    replace: Option<bool>,
}

impl Change {
    /// The change's name for the page: "delete", "move", "rename", "mkdir".
    fn op(&self) -> &'static str {
        match self {
            Change::Delete { .. } => "delete",
            Change::Move { .. } => "move",
            Change::Rename { .. } => "rename",
            Change::MakeFolder { .. } => "mkdir",
        }
    }

    fn count(&self) -> usize {
        match self {
            Change::Delete { entries, .. } => entries.len(),
            Change::Move { items, .. } => items.len(),
            Change::Rename { .. } | Change::MakeFolder { .. } => 1,
        }
    }

    /// The name the page says the change was about: the one entry's, or the first's.
    fn first_name(&self) -> String {
        match self {
            Change::Delete { entries, .. } => entries.first().map(|entry| entry.name.clone()).unwrap_or_default(),
            Change::Move { items, .. } => items.first().map(|item| item.entry.name.clone()).unwrap_or_default(),
            Change::Rename { entry, .. } => entry.name.clone(),
            Change::MakeFolder { name, .. } => name.clone(),
        }
    }

    /// The folders whose listing the change alters.
    fn touched(&self) -> Vec<&[String]> {
        match self {
            Change::Delete { path, .. } | Change::Rename { path, .. } | Change::MakeFolder { path, .. } => vec![path.as_slice()],
            Change::Move { from, to, .. } => vec![from.as_slice(), to.as_slice()],
        }
    }
}

/// The Delete dialog: what goes, from where, and what each folder among them holds (counted on
/// the worker; none until it is).
struct Deleting {
    path: Vec<String>,
    entries: Vec<remote::Entry>,
    counts: Vec<Option<u64>>,
    request: u64,
}

/// The Move dialog: what moves, from where, and the folder shown in it, with its listing once
/// that came.
struct Moving {
    entries: Vec<remote::Entry>,
    from: Vec<String>,
    at: Vec<String>,
    listed: Option<Result<Vec<remote::Entry>, remote::Error>>,
    request: u64,
}

impl Moving {
    /// Whether the folder `name` of the one shown is one of the folders moving, or under one: a
    /// folder cannot move into itself.
    fn forbidden(&self, name: &str) -> bool {
        let mut full = self.at.clone();
        full.push(name.to_string());
        self.entries.iter().filter(|entry| entry.is_dir).any(|entry| {
            let mut moving = self.from.clone();
            moving.push(entry.name.clone());
            full.starts_with(&moving)
        })
    }
}

/// A file of this Mac to upload: where it is, what it is called, how large it is.
#[derive(Clone)]
struct LocalFile {
    path: PathBuf,
    name: String,
    size: u64,
}

/// The Same-name dialog: what waits on its answer, and which names are taken at the destination.
enum Conflict {
    Move { entries: Vec<remote::Entry>, from: Vec<String>, to: Vec<String>, target: Vec<remote::Entry>, clashes: Vec<String> },
    Upload { files: Vec<LocalFile>, clashes: Vec<String> },
}

/// The server looked at, when the user allows changes on it and it is reached with its folder
/// listed: what the write items need.
fn writable(state: &State) -> Option<&Server> {
    let server = state.servers.get(state.current?)?;
    (server.allow_changes && server.status == ServerStatus::Connected && matches!(state.folder, Folder::Listed)).then_some(server)
}

/// The page's write state: whether the items show, whether they are greyed (the server said it
/// only reads), and the drop box while files are dragged over the window.
fn show_write_state() {
    let (can_write, read_only, dropping) = with_state(|state| {
        let server = writable(state);
        let allowed = state.current.and_then(|index| state.servers.get(index)).is_some_and(|server| server.allow_changes && server.status == ServerStatus::Connected && !matches!(state.folder, Folder::KeyWait { .. }));
        (server.is_some(), server.is_some_and(|server| server.changes == Changes::ReadOnly), allowed && !state.hovering.is_empty())
    })
    .unwrap_or((false, false, false));
    with_downloads(|downloads| {
        downloads.set_can_write(can_write);
        downloads.set_server_read_only(read_only);
        downloads.set_dropping(dropping);
    });
}

/// Whether the folder `path` of the server `remote` is the one looked at.
fn looking_at(remote: &Remote, path: &[String]) -> bool {
    with_state(|state| state.current.and_then(|index| state.servers.get(index)).is_some_and(|server| server.remote().same_as(remote)) && state.path == path).unwrap_or(false)
}

/// The server's name, for the page's words about a change: the one at `remote`.
fn server_name_of(state: &State, remote: &Remote) -> String {
    state.servers.iter().find(|server| server.remote().same_as(remote)).map(|server| server.name.clone()).unwrap_or_default()
}

/// A folder as the page names it: the folder's own name, or the server's at the root.
fn dest_label(state: &State, remote: &Remote, path: &[String]) -> String {
    path.last().cloned().unwrap_or_else(|| server_name_of(state, remote))
}

/// `change` on the server, on a thread of its own, one entry after another; the outcome comes
/// back through the event loop with how many were done, or what the server would not do.
fn perform(remote: Remote, change: Change) {
    let started = std::thread::Builder::new().name("changes".into()).spawn(move || {
        let http = http::client();
        let outcome: Result<usize, (String, remote::Error)> = match &change {
            Change::Delete { path, entries } => entries.iter().try_fold(0, |done, entry| remote.delete(http, path, &entry.name, entry.is_dir).map(|()| done + 1).map_err(|err| (entry.name.clone(), err))),
            Change::Move { from, to, items } => items.iter().try_fold(0, |done, item| {
                remote.move_to(http, from, &item.entry.name, item.entry.is_dir, to, &item.new_name, item.replace).map(|()| done + 1).map_err(|err| (item.entry.name.clone(), err))
            }),
            Change::Rename { path, entry, new_name } => remote.move_to(http, path, &entry.name, entry.is_dir, path, new_name, None).map(|()| 1).map_err(|err| (entry.name.clone(), err)),
            Change::MakeFolder { path, name } => remote.make_folder(http, path, name).map(|()| 1).map_err(|err| (name.clone(), err)),
        };
        if let Err((name, err)) = &outcome {
            log::line(format!("{} {name:?} failed: {err}", change.op()));
        }
        let _ = slint::invoke_from_event_loop(move || changed(remote, change, outcome));
    });
    if let Err(err) = started {
        eprintln!("downloads: cannot start the changes' thread: {err}");
    }
}

/// A batch of changes came back: the toast, the folder listed again, or what went wrong.
fn changed(remote: Remote, change: Change, outcome: Result<usize, (String, remote::Error)>) {
    let looking = change.touched().iter().any(|path| looking_at(&remote, path));
    match outcome {
        Ok(done) => match &change {
            Change::Delete { .. } => toast("deleted", &change.first_name(), done, ""),
            Change::Move { to, .. } => {
                let dest = with_state(|state| dest_label(state, &remote, to)).unwrap_or_default();
                if done == 0 {
                    toast("nothing-moved", "", 0, "");
                } else {
                    toast("moved", &change.first_name(), done, &dest);
                }
            }
            Change::Rename { entry, new_name, .. } => {
                with_state(|state| {
                    if state.selected.remove(&entry.name) {
                        state.selected.insert(new_name.clone());
                    }
                });
            }
            Change::MakeFolder { path, name } => made_folder(&remote, path, name),
        },
        Err((name, err)) => show_change_failed(&change, &name, &err, remote.protocol()),
    }
    if looking {
        list_with(true);
    }
}

/// What the server would not do, under the folder's title until dismissed.
fn show_change_failed(change: &Change, name: &str, error: &remote::Error, protocol: Protocol) {
    let (problem, detail) = problem_of(error, "");
    with_downloads(|downloads| {
        downloads.set_change_failed(change.op().into());
        downloads.set_change_name(name.into());
        downloads.set_change_count(change.count() as i32);
        downloads.set_change_refused(error.is_refusal());
        downloads.set_change_problem(problem);
        downloads.set_change_detail(if error.is_refusal() { error.refusal(protocol).into() } else { detail.into() });
    });
}

fn dismiss_change_failed() {
    with_downloads(|downloads| downloads.set_change_failed(SharedString::new()));
}

/// A word at the foot of the page for a moment: "Deleted 3 items", "Moved “x” to 2024".
fn toast(kind: &str, name: &str, count: usize, dest: &str) {
    with_downloads(|downloads| {
        downloads.set_toast_name(name.into());
        downloads.set_toast_count(count as i32);
        downloads.set_toast_dest(dest.into());
        downloads.set_toast(kind.into());
    });
    with_state(|state| {
        state.toast.start(slint::TimerMode::SingleShot, TOAST_FOR, || with_downloads(|downloads| downloads.set_toast(SharedString::new())));
    });
}

/// "name (2).ext", "name (3).ext" … when `taken` says the name is: a folder's name is whole, a
/// file's keeps its extension.
fn free_name(taken: impl Fn(&str) -> bool, name: &str, is_dir: bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if !is_dir && dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    (2..)
        .map(|n| format!("{stem} ({n}){extension}"))
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| name.to_string())
}

/// How much of a name is chosen when it is renamed: all of it but a file's extension, in bytes.
fn stem_length(name: &str, is_dir: bool) -> usize {
    match name.rfind('.') {
        Some(dot) if !is_dir && dot > 0 => dot,
        _ => name.len(),
    }
}

// -- renaming in place --

/// The menu's Rename: the entry's row becomes a field with its name, the name but the extension
/// chosen.
fn rename_entry(name: &str) {
    let entry = with_state(|state| {
        let server = writable(state)?;
        if server.changes == Changes::ReadOnly {
            return None;
        }
        let entry = state.entries.iter().find(|entry| entry.name == name)?.clone();
        state.renaming = Some(entry.name.clone());
        Some(entry)
    })
    .flatten();
    let Some(entry) = entry else { return };
    with_downloads(|downloads| {
        downloads.set_rename_text(entry.name.clone().into());
        downloads.set_rename_stem(stem_length(&entry.name, entry.is_dir) as i32);
        downloads.set_rename_error(RenameError::None);
        downloads.set_renaming(entry.name.into());
    });
}

/// ⏎ in the field, or (`from_click`) a click elsewhere: the new name goes to the server when it
/// is a name and not taken; the same name, or none, is nothing to do. A bad name stays in the
/// field with a word on it after ⏎, and is let go after a click.
fn commit_rename(from_click: bool) {
    let Some(ui) = STATE.with(|state| state.borrow().as_ref().and_then(|state| state.ui.upgrade())) else { return };
    let new_name = ui.global::<Downloads>().get_rename_text().trim().to_string();
    let outcome = with_state(|state| {
        let old = state.renaming.clone()?;
        let entry = state.entries.iter().find(|entry| entry.name == old)?.clone();
        if new_name.is_empty() || new_name == old {
            state.renaming = None;
            return None;
        }
        let error = if !webdav::is_plain_name(&new_name) {
            Some(RenameError::Slash)
        } else if state.entries.iter().any(|entry| entry.name == new_name) {
            Some(RenameError::Taken)
        } else {
            None
        };
        if let Some(error) = error {
            if from_click {
                state.renaming = None;
                return None;
            }
            return Some(Err(error));
        }
        state.renaming = None;
        let remote = writable(state)?.remote();
        Some(Ok((remote, Change::Rename { path: state.path.clone(), entry, new_name: new_name.clone() })))
    })
    .flatten();
    match outcome {
        Some(Err(error)) => with_downloads(|downloads| downloads.set_rename_error(error)),
        Some(Ok((remote, change))) => {
            with_downloads(|downloads| downloads.set_renaming(SharedString::new()));
            perform(remote, change);
        }
        None => with_downloads(|downloads| downloads.set_renaming(SharedString::new())),
    }
}

fn cancel_rename() {
    with_state(|state| state.renaming = None);
    with_downloads(|downloads| downloads.set_renaming(SharedString::new()));
}

// -- deleting --

/// The menu's Delete… on one entry: the dialog.
fn delete_entry(name: &str) {
    let entry = with_state(|state| state.entries.iter().find(|entry| entry.name == name).cloned()).flatten();
    if let Some(entry) = entry {
        open_delete(vec![entry]);
    }
}

/// The foot's Delete, or the menu's on the chosen files: the dialog.
fn delete_selected() {
    let entries: Vec<remote::Entry> = with_state(|state| state.entries.iter().filter(|entry| !entry.is_dir && state.selected.contains(&entry.name)).cloned().collect()).unwrap_or_default();
    if !entries.is_empty() {
        open_delete(entries);
    }
}

/// The Delete dialog for `entries`: the first four listed with their sizes; a folder's count
/// comes from the worker, which lists it.
fn open_delete(entries: Vec<remote::Entry>) {
    let counts = with_state(|state| {
        let server = writable(state)?;
        if server.changes == Changes::ReadOnly {
            return None;
        }
        let remote = server.remote();
        state.browse_request += 1;
        let request = state.browse_request;
        let mut tasks = Vec::new();
        for (slot, entry) in entries.iter().enumerate().take(4) {
            if entry.is_dir {
                let mut path = state.path.clone();
                path.push(entry.name.clone());
                tasks.push(Task::Count { request, remote: remote.clone(), path, slot });
            }
        }
        state.deleting = Some(Deleting { path: state.path.clone(), counts: vec![None; entries.len()], entries, request });
        Some(tasks)
    })
    .flatten();
    let Some(tasks) = counts else { return };
    for task in tasks {
        send(task);
    }
    show_delete();
}

fn counted(request: u64, slot: usize, count: u64) {
    let shown = with_state(|state| {
        let deleting = state.deleting.as_mut().filter(|deleting| deleting.request == request)?;
        *deleting.counts.get_mut(slot)? = Some(count);
        Some(())
    })
    .flatten();
    if shown.is_some() {
        show_delete();
    }
}

fn show_delete() {
    let shown = with_state(|state| {
        let deleting = state.deleting.as_ref()?;
        let items: Vec<DialogItem> = deleting
            .entries
            .iter()
            .zip(&deleting.counts)
            .take(4)
            .map(|(entry, count)| DialogItem {
                name: entry.name.clone().into(),
                icon: (if entry.is_dir { "folder" } else { icon_for(&entry.name) }).into(),
                size: (if entry.is_dir { String::new() } else { size_text(entry.size) }).into(),
                count: count.map_or(-1, |count| count.min(i32::MAX as u64) as i32),
            })
            .collect();
        Some((items, deleting.entries.len() as i32, deleting.entries.iter().any(|entry| entry.is_dir)))
    })
    .flatten();
    let Some((items, count, folders)) = shown else { return };
    with_downloads(|downloads| {
        downloads.set_delete_items(ModelRc::new(VecModel::from(items)));
        downloads.set_delete_count(count);
        downloads.set_delete_folders(folders);
        downloads.set_delete_open(true);
    });
}

/// The dialog's Delete: the entries go.
fn confirm_delete() {
    let change = with_state(|state| {
        let deleting = state.deleting.take()?;
        let remote = writable(state)?.remote();
        Some((remote, Change::Delete { path: deleting.path, entries: deleting.entries }))
    })
    .flatten();
    close_dialog();
    if let Some((remote, change)) = change {
        perform(remote, change);
    }
}

// -- moving --

/// The menu's Move to… on one entry: the dialog.
fn move_entry(name: &str) {
    let entry = with_state(|state| state.entries.iter().find(|entry| entry.name == name).cloned()).flatten();
    if let Some(entry) = entry {
        open_move(vec![entry]);
    }
}

/// The foot's Move…, or the menu's on the chosen files: the dialog.
fn move_selected() {
    let entries: Vec<remote::Entry> = with_state(|state| state.entries.iter().filter(|entry| !entry.is_dir && state.selected.contains(&entry.name)).cloned().collect()).unwrap_or_default();
    if !entries.is_empty() {
        open_move(entries);
    }
}

/// The Move dialog for `entries`, opened at the folder above the one looked at.
fn open_move(entries: Vec<remote::Entry>) {
    let opened = with_state(|state| {
        let server = writable(state)?;
        if server.changes == Changes::ReadOnly {
            return None;
        }
        let from = state.path.clone();
        let at = from[..from.len().saturating_sub(1)].to_vec();
        state.moving = Some(Moving { entries, from, at, listed: None, request: 0 });
        Some(())
    })
    .flatten();
    if opened.is_none() {
        return;
    }
    with_downloads(|downloads| {
        downloads.set_move_creating(false);
        downloads.set_move_new_name(SharedString::new());
        downloads.set_move_new_error(RenameError::None);
        downloads.set_move_open(true);
    });
    browse();
}

/// The folder the Move dialog shows, asked of the worker.
fn browse() {
    let task = with_state(|state| {
        state.browse_request += 1;
        let request = state.browse_request;
        let remote = state.servers.get(state.current?)?.remote();
        let moving = state.moving.as_mut()?;
        moving.request = request;
        moving.listed = None;
        Some(Task::Browse { request, remote, path: moving.at.clone() })
    })
    .flatten();
    if let Some(task) = task {
        send(task);
    }
    show_move();
}

fn browsed(request: u64, result: Result<Vec<remote::Entry>, remote::Error>) {
    let language = STATE
        .with(|state| state.borrow().as_ref().and_then(|state| state.ui.upgrade()))
        .map(|ui| ui.global::<AppState>().get_language().to_string())
        .unwrap_or_else(|| "en".to_string());
    let order = platform::NameOrder::new(&language);
    let shown = with_state(|state| {
        let moving = state.moving.as_mut().filter(|moving| moving.request == request)?;
        moving.listed = Some(result.map(|mut entries| {
            entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| order.compare(&a.name, &b.name)));
            entries
        }));
        Some(())
    })
    .flatten();
    if shown.is_some() {
        show_move();
    }
}

fn show_move() {
    let shown = with_state(|state| {
        let moving = state.moving.as_ref()?;
        let server_name = state.current.and_then(|index| state.servers.get(index)).map(|server| server.name.clone()).unwrap_or_default();
        let mut crumbs = vec![SharedString::from(server_name.as_str())];
        crumbs.extend(moving.at.iter().map(|folder| SharedString::from(folder.as_str())));
        let folders: Vec<MoveFolder> = match &moving.listed {
            Some(Ok(entries)) => entries
                .iter()
                .filter(|entry| entry.is_dir)
                .map(|entry| {
                    let mut full = moving.at.clone();
                    full.push(entry.name.clone());
                    MoveFolder { name: entry.name.clone().into(), current: full == moving.from, forbidden: moving.forbidden(&entry.name) }
                })
                .collect(),
            _ => Vec::new(),
        };
        let (problem, detail) = match &moving.listed {
            Some(Err(err)) => problem_of(err, &state.current.and_then(|index| state.servers.get(index)).map(Server::host).unwrap_or_default()),
            _ => (Problem::None, String::new()),
        };
        let one = if moving.entries.len() == 1 { moving.entries[0].name.clone() } else { String::new() };
        Some((crumbs, folders, moving.listed.is_none(), problem, detail, moving.at != moving.from, moving.entries.len() as i32, one))
    })
    .flatten();
    let Some((crumbs, folders, loading, problem, detail, allowed, count, name)) = shown else { return };
    with_downloads(|downloads| {
        downloads.set_move_path_label(crumbs.iter().map(SharedString::as_str).collect::<Vec<_>>().join(" › ").into());
        downloads.set_move_crumbs(ModelRc::new(VecModel::from(crumbs)));
        downloads.set_move_folders(ModelRc::new(VecModel::from(folders)));
        downloads.set_move_loading(loading);
        downloads.set_move_problem(problem);
        downloads.set_move_problem_detail(detail.into());
        downloads.set_move_here_allowed(allowed);
        downloads.set_move_count(count);
        downloads.set_move_name(name.into());
    });
}

/// A crumb of the dialog: 0 is the server's root.
fn move_go_to(index: i32) {
    let Ok(index) = usize::try_from(index) else { return };
    let changed = with_state(|state| {
        let moving = state.moving.as_mut()?;
        moving.at.truncate(index);
        Some(())
    })
    .flatten();
    if changed.is_some() {
        with_downloads(|downloads| downloads.set_move_creating(false));
        browse();
    }
}

/// A folder of the dialog's list: into it, unless it is one of the folders moving.
fn move_open_folder(name: &str) {
    let changed = with_state(|state| {
        let moving = state.moving.as_mut()?;
        if moving.forbidden(name) || !matches!(&moving.listed, Some(Ok(entries)) if entries.iter().any(|entry| entry.is_dir && entry.name == name)) {
            return None;
        }
        moving.at.push(name.to_string());
        Some(())
    })
    .flatten();
    if changed.is_some() {
        with_downloads(|downloads| downloads.set_move_creating(false));
        browse();
    }
}

/// New folder: its name is typed at the top of the list.
fn move_new_folder() {
    with_downloads(|downloads| {
        downloads.set_move_new_name(SharedString::new());
        downloads.set_move_new_error(RenameError::None);
        downloads.set_move_creating(true);
    });
}

/// ⏎ in that field: the folder is made on the server, and the dialog goes into it once it is.
fn commit_new_folder() {
    let Some(ui) = STATE.with(|state| state.borrow().as_ref().and_then(|state| state.ui.upgrade())) else { return };
    let name = ui.global::<Downloads>().get_move_new_name().trim().to_string();
    if name.is_empty() {
        cancel_new_folder();
        return;
    }
    let outcome = with_state(|state| {
        let moving = state.moving.as_ref()?;
        if !webdav::is_plain_name(&name) {
            return Some(Err(RenameError::Slash));
        }
        if matches!(&moving.listed, Some(Ok(entries)) if entries.iter().any(|entry| entry.name == name)) {
            return Some(Err(RenameError::Taken));
        }
        let remote = writable(state)?.remote();
        Some(Ok((remote, Change::MakeFolder { path: moving.at.clone(), name: name.clone() })))
    })
    .flatten();
    match outcome {
        Some(Err(error)) => with_downloads(|downloads| downloads.set_move_new_error(error)),
        Some(Ok((remote, change))) => {
            with_downloads(|downloads| {
                downloads.set_move_creating(false);
                downloads.set_move_loading(true);
            });
            perform(remote, change);
        }
        None => cancel_new_folder(),
    }
}

fn cancel_new_folder() {
    with_downloads(|downloads| {
        downloads.set_move_creating(false);
        downloads.set_move_new_error(RenameError::None);
    });
}

/// The folder asked for in the Move dialog was made: the dialog goes into it.
fn made_folder(remote: &Remote, path: &[String], name: &str) {
    let into = with_state(|state| {
        let same = state.current.and_then(|index| state.servers.get(index)).is_some_and(|server| server.remote().same_as(remote));
        let moving = state.moving.as_mut()?;
        if !same || moving.at != path {
            return None;
        }
        moving.at.push(name.to_string());
        Some(())
    })
    .flatten();
    if into.is_some() {
        browse();
    }
}

/// Move here: the entries into the folder shown; a name taken there asks first.
fn move_here() {
    let next = with_state(|state| {
        let remote = writable(state)?.remote();
        let moving = state.moving.as_ref()?;
        if moving.at == moving.from || !matches!(&moving.listed, Some(Ok(_))) {
            return None;
        }
        let moving = state.moving.take()?;
        let Some(Ok(target)) = moving.listed else { return None };
        let clashes: Vec<String> = moving.entries.iter().filter(|entry| target.iter().any(|there| there.name == entry.name)).map(|entry| entry.name.clone()).collect();
        if clashes.is_empty() {
            let items = moving.entries.into_iter().map(|entry| Moved { new_name: entry.name.clone(), entry, replace: None }).collect();
            return Some(Ok((remote, Change::Move { from: moving.from, to: moving.at, items })));
        }
        state.conflict = Some(Conflict::Move { entries: moving.entries, from: moving.from, to: moving.at, target, clashes });
        Some(Err(()))
    })
    .flatten();
    match next {
        Some(Ok((remote, change))) => {
            close_dialog();
            perform(remote, change);
        }
        Some(Err(())) => {
            with_downloads(|downloads| downloads.set_move_open(false));
            show_conflict();
        }
        None => {}
    }
}

// -- the same name at the destination --

fn show_conflict() {
    let shown = with_state(|state| {
        let remote = state.servers.get(state.current?)?.remote();
        let conflict = state.conflict.as_ref()?;
        let (upload, dest, items, total, folders) = match conflict {
            Conflict::Move { entries, to, target, clashes, .. } => {
                let items: Vec<DialogItem> = clashes
                    .iter()
                    .take(4)
                    .map(|name| {
                        let is_dir = target.iter().any(|there| there.name == *name && there.is_dir);
                        DialogItem { name: name.clone().into(), icon: (if is_dir { "folder" } else { icon_for(name) }).into(), size: SharedString::new(), count: -1 }
                    })
                    .collect();
                let folders = clashes.iter().any(|name| target.iter().any(|there| there.name == *name && there.is_dir));
                (false, dest_label(state, &remote, to), items, entries.len(), folders)
            }
            Conflict::Upload { files, clashes } => {
                let items: Vec<DialogItem> = clashes
                    .iter()
                    .take(4)
                    .map(|name| {
                        let is_dir = state.entries.iter().any(|there| there.name == *name && there.is_dir);
                        DialogItem { name: name.clone().into(), icon: (if is_dir { "folder" } else { icon_for(name) }).into(), size: SharedString::new(), count: -1 }
                    })
                    .collect();
                let folders = clashes.iter().any(|name| state.entries.iter().any(|there| there.name == *name && there.is_dir));
                (true, dest_label(state, &remote, &state.path), items, files.len(), folders)
            }
        };
        let clashes = match conflict {
            Conflict::Move { clashes, .. } | Conflict::Upload { clashes, .. } => clashes.len(),
        };
        Some((upload, dest, items, clashes as i32, (total - clashes) as i32, folders))
    })
    .flatten();
    let Some((upload, dest, items, count, rest, folders)) = shown else { return };
    with_downloads(|downloads| {
        downloads.set_conflict_upload(upload);
        downloads.set_conflict_dest(dest.into());
        downloads.set_conflict_items(ModelRc::new(VecModel::from(items)));
        downloads.set_conflict_count(count);
        downloads.set_conflict_rest(rest);
        downloads.set_conflict_folders(folders);
        downloads.set_conflict_open(true);
    });
}

/// The dialog's answer: "replace" what is there, "skip" what clashes, or "keep" both, the new
/// one as "name (2).ext".
fn resolve_conflict(answer: &str) {
    let next = with_state(|state| {
        let conflict = state.conflict.take()?;
        let remote = writable(state)?.remote();
        match conflict {
            Conflict::Move { entries, from, to, target, clashes } => {
                let mut taken: Vec<String> = target.iter().map(|entry| entry.name.clone()).collect();
                let mut items = Vec::new();
                for entry in entries {
                    let clashing = clashes.contains(&entry.name);
                    let item = match (clashing, answer) {
                        (false, _) => Moved { new_name: entry.name.clone(), entry, replace: None },
                        (true, "replace") => {
                            let replace = Some(target.iter().any(|there| there.name == entry.name && there.is_dir));
                            Moved { new_name: entry.name.clone(), entry, replace }
                        }
                        (true, "keep") => {
                            let new_name = free_name(|candidate| taken.iter().any(|name| name == candidate), &entry.name, entry.is_dir);
                            Moved { new_name, entry, replace: None }
                        }
                        (true, _) => continue,
                    };
                    taken.push(item.new_name.clone());
                    items.push(item);
                }
                Some(Ok((remote, Change::Move { from, to, items })))
            }
            Conflict::Upload { files, clashes } => {
                let mut taken: Vec<String> = state.entries.iter().map(|entry| entry.name.clone()).collect();
                let mut kept = Vec::new();
                for mut file in files {
                    let clashing = clashes.contains(&file.name);
                    match (clashing, answer) {
                        (false, _) | (true, "replace") => {}
                        (true, "keep") => file.name = free_name(|candidate| taken.iter().any(|name| name == candidate), &file.name, false),
                        (true, _) => continue,
                    }
                    taken.push(file.name.clone());
                    kept.push(file);
                }
                Some(Err(kept))
            }
        }
    })
    .flatten();
    close_dialog();
    match next {
        Some(Ok((remote, change))) => {
            if change.count() == 0 {
                toast("nothing-moved", "", 0, "");
            } else {
                perform(remote, change);
            }
        }
        Some(Err(files)) => enqueue_uploads(files),
        None => {}
    }
}

/// Cancel, or Esc, on the Delete, Move and Same-name dialogs: nothing happens.
fn close_dialog() {
    with_state(|state| {
        state.deleting = None;
        state.moving = None;
        state.conflict = None;
    });
    with_downloads(|downloads| {
        downloads.set_delete_open(false);
        downloads.set_move_open(false);
        downloads.set_conflict_open(false);
        downloads.set_move_creating(false);
    });
}

// -- uploading --

/// The toolbar's Upload: the file chooser, then the files into the folder looked at. On a server
/// that said it only reads, the button is greyed, and a word says why.
fn upload() {
    match with_state(|state| writable(state).map(|server| server.changes != Changes::ReadOnly)).flatten() {
        Some(true) => platform::choose_files(false, upload_files),
        Some(false) => toast("read-only", "", 0, ""),
        None => {}
    }
}

/// A file dragged over the window (winit's HoveredFile, one per file) while this page is in
/// front: the drop box, where the server takes changes.
pub fn hovering(path: PathBuf) {
    with_state(|state| state.hovering.push(path));
    show_write_state();
}

/// The drag left the window without dropping.
pub fn hover_ended() {
    with_state(|state| state.hovering.clear());
    show_write_state();
}

/// A file dropped on the window (winit's DroppedFile, one per file): taken with the others of
/// the same drop once they have all come.
pub fn dropped(path: PathBuf) {
    let first = with_state(|state| {
        state.dropped.push(path);
        !std::mem::replace(&mut state.flush_asked, true)
    })
    .unwrap_or(false);
    if first {
        slint::Timer::single_shot(Duration::ZERO, flush_dropped);
    }
}

fn flush_dropped() {
    let paths = with_state(|state| {
        state.flush_asked = false;
        state.hovering.clear();
        std::mem::take(&mut state.dropped)
    })
    .unwrap_or_default();
    show_write_state();
    if paths.is_empty() {
        return;
    }
    match with_state(|state| writable(state).map(|server| server.changes != Changes::ReadOnly)).flatten() {
        Some(true) => upload_files(paths),
        Some(false) => toast("read-only", "", 0, ""),
        None => {}
    }
}

/// Files of this Mac into the folder looked at: the regular files among them (a folder dropped is
/// left where it is, docs/downloads.md), each under its own name; a name taken in the folder asks
/// first.
fn upload_files(paths: Vec<PathBuf>) {
    let files: Vec<LocalFile> = paths
        .into_iter()
        .filter_map(|path| {
            let meta = std::fs::metadata(&path).ok()?;
            let name = path.file_name()?.to_string_lossy().into_owned();
            if !meta.is_file() || !webdav::is_plain_name(&name) {
                log::line(format!("not uploaded, not a file: {}", path.display()));
                return None;
            }
            Some(LocalFile { path, name, size: meta.len() })
        })
        .collect();
    if files.is_empty() {
        return;
    }
    let clashes = with_state(|state| {
        writable(state).filter(|server| server.changes != Changes::ReadOnly)?;
        Some(files.iter().filter(|file| state.entries.iter().any(|entry| entry.name == file.name)).map(|file| file.name.clone()).collect::<Vec<_>>())
    })
    .flatten();
    let Some(clashes) = clashes else { return };
    if clashes.is_empty() {
        enqueue_uploads(files);
        return;
    }
    with_state(|state| state.conflict = Some(Conflict::Upload { files, clashes }));
    show_conflict();
}

/// The files into the queue, up to the folder looked at.
fn enqueue_uploads(files: Vec<LocalFile>) {
    let added = with_state(|state| {
        let server = writable(state)?;
        let remote = server.remote();
        let dest = dest_label(state, &remote, &state.path);
        let mut added = false;
        for file in files {
            // One at a time per name and folder.
            if state.queue.iter().any(|transfer| transfer.live() && matches!(&transfer.kind, Kind::Upload { remote: theirs, path, name, .. } if theirs.same_as(&remote) && *path == state.path && *name == file.name)) {
                continue;
            }
            let id = state.next_id;
            state.next_id += 1;
            state.queue.push(Transfer {
                id,
                name: file.name.clone(),
                icon: icon_for(&file.name),
                kind: Kind::Upload { remote: remote.clone(), path: state.path.clone(), name: file.name, local: file.path, size: file.size, dest: dest.clone() },
                state: TransferState::Waiting,
                done: 0,
                total: file.size,
                samples: VecDeque::new(),
                waiting: 0,
                problem: Problem::None,
                detail: String::new(),
                cancel: Arc::new(AtomicBool::new(false)),
                result: None,
                open_when_done: false,
                key_shown: None,
            });
            added = true;
        }
        Some(added)
    })
    .flatten();
    if added == Some(true) {
        queued();
    }
}

/// A finished upload's Show: the folder it went to, with the file chosen.
fn show_upload(id: i32) {
    let target = with_state(|state| {
        let transfer = state.queue.iter().find(|transfer| transfer.id == id && transfer.state == TransferState::Done)?;
        let Kind::Upload { remote, path, name, .. } = &transfer.kind else { return None };
        let index = state.servers.iter().position(|server| server.remote().same_as(remote))?;
        state.current = Some(index);
        state.path = path.clone();
        state.selected.clear();
        state.filter.clear();
        state.editing = None;
        state.reveal = Some(name.clone());
        Some(())
    })
    .flatten();
    if target.is_none() {
        return;
    }
    with_downloads(|downloads| {
        downloads.set_form_open(false);
        downloads.set_filter(SharedString::new());
    });
    show_servers();
    list();
}

// ---- words and pictures -----------------------------------------------------------------------

/// The icon for a file, by its extension, as the design has them.
fn icon_for(name: &str) -> &'static str {
    let extension = name.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase()).unwrap_or_default();
    match extension.as_str() {
        "mkv" | "mp4" | "m4v" | "mov" | "avi" | "webm" | "ts" | "wmv" | "flv" => "film-strip",
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "bmp" | "tif" | "tiff" => "file-image",
        "pdf" => "file-pdf",
        "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "dmg" | "iso" => "file-zip",
        "txt" | "srt" | "ass" | "md" | "nfo" | "log" | "json" | "xml" | "csv" => "file-text",
        _ => "file",
    }
}

/// A photo or a video: the viewer can show it.
fn is_media(icon: &str) -> bool {
    matches!(icon, "film-strip" | "file-image")
}

/// "4.4 GB", "48 MB", "2.4 MB", "90 KB": as the design sizes files.
fn size_text(bytes: u64) -> String {
    let megabytes = bytes as f64 / (1024.0 * 1024.0);
    if megabytes >= 1024.0 {
        format!("{:.1} GB", megabytes / 1024.0)
    } else if megabytes >= 10.0 {
        format!("{} MB", megabytes.round() as u64)
    } else if megabytes >= 1.0 {
        format!("{megabytes:.1} MB")
    } else {
        format!("{} KB", ((bytes as f64) / 1024.0).round().max(1.0) as u64)
    }
}

/// "24.3 MB/s", "640 KB/s".
fn speed_text(bytes_per_second: f64) -> String {
    let megabytes = bytes_per_second / (1024.0 * 1024.0);
    if megabytes >= 1.0 { format!("{megabytes:.1} MB/s") } else { format!("{} KB/s", (bytes_per_second / 1024.0).round() as u64) }
}

/// "Sep 30" this year, "Sep 30, 2023" another.
fn date_text(unix: i64, this_year: i32) -> String {
    match Local.timestamp_opt(unix, 0).single() {
        Some(time) if time.year() == this_year => time.format("%b %-d").to_string(),
        Some(time) => time.format("%b %-d, %Y").to_string(),
        None => String::new(),
    }
}

/// "Oct 3, 2026": the day a server's key was trusted.
fn full_date_text(unix: i64) -> String {
    Local.timestamp_opt(unix, 0).single().map(|time| time.format("%b %-d, %Y").to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_taken_name_gets_a_number_before_its_extension() {
        let taken = |names: &[&str]| {
            let names: Vec<String> = names.iter().map(|name| name.to_string()).collect();
            move |candidate: &str| names.iter().any(|name| name == candidate)
        };
        assert_eq!(free_name(taken(&[]), "Dune.mkv", false), "Dune.mkv");
        assert_eq!(free_name(taken(&["Dune.mkv"]), "Dune.mkv", false), "Dune (2).mkv");
        assert_eq!(free_name(taken(&["Dune.mkv", "Dune (2).mkv"]), "Dune.mkv", false), "Dune (3).mkv");
        assert_eq!(free_name(taken(&["Extras"]), "Extras", true), "Extras (2)");
        assert_eq!(free_name(taken(&["v2.0"]), "v2.0", true), "v2.0 (2)");
        assert_eq!(free_name(taken(&[".hidden"]), ".hidden", false), ".hidden (2)");
        assert_eq!(free_name(taken(&["README"]), "README", false), "README (2)");
        assert_eq!(stem_length("Dune Part Two (2024) 2160p.mkv", false), "Dune Part Two (2024) 2160p".len());
        assert_eq!(stem_length("Extras", true), "Extras".len());
        assert_eq!(stem_length("v2.0", true), "v2.0".len());
        assert_eq!(stem_length(".hidden", false), ".hidden".len());
    }

    #[test]
    fn a_folder_cannot_move_into_itself() {
        let entry = |name: &str, is_dir: bool| remote::Entry { name: name.into(), is_dir, size: 0, modified: None };
        let path = |parts: &[&str]| parts.iter().map(|part| part.to_string()).collect::<Vec<_>>();
        let moving = Moving { entries: vec![entry("Extras", true), entry("film.mkv", false)], from: path(&["Media", "2024"]), at: path(&["Media", "2024"]), listed: None, request: 0 };
        assert!(moving.forbidden("Extras"));
        assert!(!moving.forbidden("Subtitles"));
        let deeper = Moving { at: path(&["Media", "2024", "Extras"]), ..moving };
        assert!(deeper.forbidden("Behind the scenes"));
        let elsewhere = Moving { at: path(&["Media"]), ..deeper };
        assert!(!elsewhere.forbidden("Extras"));
    }

    #[test]
    fn a_file_of_a_tree_goes_under_the_folder_and_nowhere_else() {
        let dest = Path::new("/Users/x/Documents/FinchGram/webdav/home nas/Movies");
        let file = |folder: &[&str], name: &str| TreeFile { folder: folder.iter().map(|part| part.to_string()).collect(), name: name.to_string(), size: 0 };
        assert_eq!(file(&["2026 Films"], "Dune.mkv").path_under(dest), Some(dest.join("2026 Films").join("Dune.mkv")));
        assert_eq!(file(&[], "notes.txt").path_under(dest), Some(dest.join("notes.txt")));
        assert_eq!(file(&[".."], "x").path_under(dest), None);
        assert_eq!(file(&["a/b"], "x").path_under(dest), None);
        assert_eq!(file(&[], "/tmp/x").path_under(dest), None);
        assert_eq!(file(&[], "..").path_under(dest), None);
        assert_eq!(file(&[], "").path_under(dest), None);
    }
}
