//! Downloads (the design's third round of the shared pages, "FinchGram Desktop 下载"): WebDAV
//! servers, their folders, and files fetched over several connections at once
//! (docs/downloads.md). `webdav.rs` speaks to a server and `transfer.rs` fetches a file; this
//! module keeps the page's state on the UI thread and fills the Downloads global
//! (ui/downloads.slint), which the page (ui/pages/downloads.slint) shows in every theme.
//!
//! Threads (docs/architecture.md): one worker lists folders, checks servers and tests the form's
//! server, one request at a time in the order asked, and answers through the event loop; a
//! listing that is not the current one any more is dropped when it comes back. Each download runs
//! on a thread of its own, two at a time, and reports its progress the same way. The servers,
//! with their passwords, are the app's database's (src/database.rs); versions up to 0.4.17 kept
//! them in settings.toml with the passwords in the Keychain, and such a list is moved into the
//! database once, at the start (`load_servers`).

pub mod http;
pub mod log;
pub mod transfer;
pub mod webdav;

use std::cell::RefCell;
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, TimeZone};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::settings::{self, Settings};
use crate::{AppState, Downloads, FolderEntry, FolderState, FormError, MainWindow, Problem, ServerRow, ServerStatus, TestState, TransferRow, TransferState, database, platform, telegram};

/// The Keychain's name for the WebDAV passwords of versions up to 0.4.17 (the account was
/// "user@address"): read once to move them into the database, then deleted.
const OLD_KEYCHAIN_SERVICE: &str = "FinchGram WebDAV";
/// How many files are fetched at the same time; the rest wait.
const MOST_ACTIVE: usize = 2;
/// The speed shown is over this long.
const SPEED_WINDOW: Duration = Duration::from_secs(10);
/// Where downloads go unless the settings say: this folder in the user's Downloads.
const DEFAULT_FOLDER: &str = "FinchGram";

/// A server as the page keeps it.
struct Server {
    name: String,
    url: String,
    username: String,
    /// "" for a server that asks for none.
    password: String,
    status: ServerStatus,
}

impl Server {
    fn from_record(record: database::WebdavServer) -> Server {
        Server { name: record.name, url: record.url, username: record.username, password: record.password, status: ServerStatus::Unknown }
    }

    /// The server as the database keeps it.
    fn record(&self) -> database::WebdavServer {
        database::WebdavServer { name: self.name.clone(), url: self.url.clone(), username: self.username.clone(), password: self.password.clone() }
    }

    /// The server for the worker.
    fn client(&self) -> webdav::Server {
        webdav::Server { url: self.url.clone(), username: self.username.clone(), password: self.password.clone() }
    }

    fn row(&self) -> ServerRow {
        ServerRow { name: self.name.clone().into(), host: webdav::Server::host(&self.url).into(), status: self.status }
    }
}

/// What the middle column shows of the folder.
enum Folder {
    Loading,
    Listed,
    Error(Problem, String),
}

/// What a download fetches.
#[derive(Clone)]
enum Kind {
    File { source: transfer::Source, dest: PathBuf },
    /// A whole folder, file by file, into `dest`.
    Folder { server: webdav::Server, path: Vec<String>, dest: PathBuf },
}

/// A download in the queue.
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
    List { request: u64, server: webdav::Server, path: Vec<String> },
    Check { index: usize, server: webdav::Server },
    Test { request: u64, server: webdav::Server },
}

/// What Test connection found: folders and files at the root, and whether the server does ranges
/// (asked of the first file, when there is one).
struct TestResult {
    folders: i32,
    files: i32,
    ranges: Option<bool>,
}

struct State {
    ui: slint::Weak<MainWindow>,
    settings: Rc<RefCell<Settings>>,
    servers: Vec<Server>,
    /// The server looked at.
    current: Option<usize>,
    /// The folder looked at, from the server's root.
    path: Vec<String>,
    entries: Vec<webdav::Entry>,
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
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub fn connect(ui: &MainWindow, settings: Rc<RefCell<Settings>>) {
    let servers = load_servers(&settings);
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
    let started = std::thread::Builder::new().name("webdav".into()).spawn(move || {
        let http = http::client();
        for task in tasks {
            match task {
                Task::List { request, server, path } => {
                    let result = server.list(http, &path);
                    let _ = slint::invoke_from_event_loop(move || listed(request, result));
                }
                Task::Check { index, server } => {
                    let result = server.check(http);
                    let _ = slint::invoke_from_event_loop(move || checked(index, result));
                }
                Task::Test { request, server } => {
                    let result = test(http, &server);
                    let _ = slint::invoke_from_event_loop(move || tested(request, result));
                }
            }
        }
    });
    if let Err(err) = started {
        eprintln!("downloads: cannot start the WebDAV thread: {err}");
    }
    sender
}

/// Test connection, on the worker: the root's listing, and whether the first file comes in ranges.
fn test(http: &http::Http, server: &webdav::Server) -> Result<TestResult, webdav::Error> {
    let entries = server.list(http, &[])?;
    let folders = entries.iter().filter(|entry| entry.is_dir).count() as i32;
    let files = entries.len() as i32 - folders;
    let ranges = entries.iter().find(|entry| !entry.is_dir && entry.size > 0).and_then(|file| {
        let source = transfer::Source { url: server.file_url(&[], &file.name), authorization: server.authorization() };
        transfer::probe(http, &source).ok().map(|probe| probe.ranges)
    });
    Ok(TestResult { folders, files, ranges })
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
                checks.push(Task::Check { index, server: server.client() });
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

fn checked(index: usize, result: Result<(), webdav::Error>) {
    with_state(|state| {
        if let Some(server) = state.servers.get_mut(index)
            && server.status == ServerStatus::Checking
        {
            server.status = status_of(&result.err());
        }
    });
    show_servers();
}

/// What a server's answer says about it: nothing bad, the password, or no connection.
fn status_of(error: &Option<webdav::Error>) -> ServerStatus {
    match error {
        None => ServerStatus::Connected,
        Some(webdav::Error::Unauthorized) => ServerStatus::PasswordRejected,
        Some(webdav::Error::Unreachable(_)) => ServerStatus::NotReachable,
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
}

fn copy_address(index: i32) {
    let url = with_state(|state| usize::try_from(index).ok().and_then(|index| state.servers.get(index)).map(|server| format!("{}/", server.url))).flatten();
    if let Some(url) = url
        && let Err(err) = platform::copy_text(&url)
    {
        eprintln!("downloads: cannot copy: {err}");
    }
}

// ---- the folder -------------------------------------------------------------------------------

/// Ask the worker for the folder looked at.
fn list() {
    let task = with_state(|state| {
        let index = state.current?;
        let server = state.servers.get_mut(index)?;
        if server.status != ServerStatus::Connected {
            server.status = ServerStatus::Checking;
        }
        state.request += 1;
        state.folder = Folder::Loading;
        Some(Task::List { request: state.request, server: server.client(), path: state.path.clone() })
    })
    .flatten();
    let Some(task) = task else {
        show_folder();
        return;
    };
    send(task);
    show_servers();
    show_folder();
}

fn listed(request: u64, result: Result<Vec<webdav::Entry>, webdav::Error>) {
    let language = STATE
        .with(|state| state.borrow().as_ref().and_then(|state| state.ui.upgrade()))
        .map(|ui| ui.global::<AppState>().get_language().to_string())
        .unwrap_or_else(|| "en".to_string());
    let order = platform::NameOrder::new(&language);
    with_state(|state| {
        if request != state.request {
            return;
        }
        let status = status_of(&result.as_ref().err().cloned());
        if let Some(server) = state.current.and_then(|index| state.servers.get_mut(index)) {
            server.status = status;
        }
        match result {
            Ok(mut entries) => {
                entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| order.compare(&a.name, &b.name)));
                state.entries = entries;
                state.folder = Folder::Listed;
            }
            Err(err) => {
                let host = state.current.and_then(|index| state.servers.get(index)).map(|server| webdav::Server::host(&server.url)).unwrap_or_default();
                let (problem, detail) = problem_of(&err, &host);
                state.entries.clear();
                state.folder = Folder::Error(problem, detail);
            }
        }
    });
    show_servers();
    show_folder();
}

/// Why a server could not be read, as the page says it.
fn problem_of(error: &webdav::Error, host: &str) -> (Problem, String) {
    match error {
        webdav::Error::Unauthorized => (Problem::Password, String::new()),
        webdav::Error::Forbidden => (Problem::Forbidden, String::new()),
        webdav::Error::Status(code) => (Problem::Http, code.to_string()),
        webdav::Error::Unreachable(_) => (Problem::Unreachable, host.to_string()),
        webdav::Error::Unreadable(_) => (Problem::Unreadable, String::new()),
    }
}

/// The entries the filter lets through.
fn visible<'a>(state: &'a State) -> impl Iterator<Item = &'a webdav::Entry> + 'a {
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
        let files: Vec<&webdav::Entry> = visible(state).filter(|entry| !entry.is_dir).collect();
        let chosen: Vec<&webdav::Entry> = state.entries.iter().filter(|entry| !entry.is_dir && state.selected.contains(&entry.name)).collect();
        let chosen_shown = files.iter().filter(|entry| state.selected.contains(&entry.name)).count();
        Shown {
            crumbs,
            title,
            state: folder_state,
            problem,
            detail,
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
        downloads.set_crumbs(ModelRc::new(VecModel::from(shown.crumbs)));
        downloads.set_title(shown.title.into());
        downloads.set_folder_state(shown.state);
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
        let client = webdav::Server { url: server.url.clone(), username: String::new(), password: String::new() };
        Some(if entry.is_dir {
            let mut path = state.path.clone();
            path.push(entry.name.clone());
            client.folder_url(&path)
        } else {
            client.file_url(&state.path, &entry.name)
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
    dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from(".")).join(DEFAULT_FOLDER)
}

/// The folder as the page names it: "~/Downloads/FinchGram".
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
        let client = state.servers.get_mut(index)?.client();
        let source = transfer::Source { url: client.file_url(&state.path, &entry.name), authorization: client.authorization() };
        // One at a time per file: a second request only asks to open it.
        if let Some(existing) = state.queue.iter_mut().find(|transfer| transfer.live() && matches!(&transfer.kind, Kind::File { source: theirs, .. } if *theirs == source)) {
            existing.open_when_done |= open;
            return Some(false);
        }
        let dest = download_folder(&state.settings.borrow()).join(&entry.name);
        let id = state.next_id;
        state.next_id += 1;
        state.queue.push(Transfer {
            id,
            name: entry.name.clone(),
            icon: icon_for(&entry.name),
            kind: Kind::File { source, dest: dest.clone() },
            state: TransferState::Waiting,
            done: transfer::kept(&dest),
            total: entry.size,
            samples: VecDeque::new(),
            waiting: 0,
            problem: Problem::None,
            detail: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            result: None,
            open_when_done: open,
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
        let client = state.servers.get_mut(index)?.client();
        let mut path = state.path.clone();
        path.push(name.to_string());
        if state.queue.iter().any(|transfer| transfer.live() && matches!(&transfer.kind, Kind::Folder { path: theirs, server, .. } if *theirs == path && server.url == client.url)) {
            return Some(false);
        }
        let dest = download_folder(&state.settings.borrow()).join(name);
        let id = state.next_id;
        state.next_id += 1;
        state.queue.push(Transfer {
            id,
            name: name.to_string(),
            icon: "folder",
            kind: Kind::Folder { server: client, path, dest },
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

/// Start waiting downloads while fewer than MOST_ACTIVE are on their way.
fn start_next() {
    loop {
        let started = with_state(|state| {
            let active = state.queue.iter().filter(|transfer| transfer.state == TransferState::Active).count();
            if active >= MOST_ACTIVE {
                return false;
            }
            let connections = state.settings.borrow().downloads.connections.clamp(1, transfer::MOST_CONNECTIONS as u32) as usize;
            let Some(transfer) = state.queue.iter_mut().find(|transfer| transfer.state == TransferState::Waiting) else { return false };
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
            Kind::File { source, dest } => transfer::download(http, &source, &dest, &streams, &cancel, &progress),
            Kind::Folder { server, path, dest } => download_tree(http, &server, &path, &dest, &streams, &cancel, &progress),
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

/// A folder and everything in it, into `dest`: the tree listed first, several folders at a time,
/// the total growing on the page as files are found; then the files, several at a time over the
/// transfer's budget of streams. A file already there with its size is not fetched again: a
/// stopped folder goes on where it was.
fn download_tree(
    http: &http::Http,
    server: &webdav::Server,
    path: &[String],
    dest: &Path,
    streams: &transfer::Streams,
    cancel: &AtomicBool,
    progress: &(dyn Fn(transfer::Progress) + Sync),
) -> Result<PathBuf, transfer::Error> {
    let files = http.block_on(list_tree(http, server, path, cancel, progress))?;
    let total: u64 = files.iter().map(|file| file.size).sum();
    log::line(format!("folder {}: {} files, {:.1} MB", log::shown(&server.folder_url(path)), files.len(), total as f64 / (1024.0 * 1024.0)));
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
                    if !already {
                        let source = transfer::Source { url: server.file_url(&whole, &file.name), authorization: server.authorization() };
                        if let Err(err) = transfer::download(http, &source, &file_dest, streams, cancel, &|progress| report(slot, progress.done, progress.waiting)) {
                            let mut failed = failure.lock().unwrap();
                            if failed.is_none() {
                                *failed = Some(err);
                            }
                            return;
                        }
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
async fn list_tree(http: &http::Http, server: &webdav::Server, path: &[String], cancel: &AtomicBool, progress: &(dyn Fn(transfer::Progress) + Sync)) -> Result<Vec<TreeFile>, transfer::Error> {
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
            let (client, server) = (http.clone(), server.clone());
            let mut whole = path.to_vec();
            whole.extend(folder.iter().cloned());
            set.spawn_on(async move { (folder, server.list_in(&client, &whole).await) }, http.handle());
        }
        let Some(next) = set.join_next().await else { break };
        let (folder, listed) = next.map_err(|err| transfer::Error::Connection(format!("a listing's task failed: {err}")))?;
        for entry in listed.map_err(transfer_error)? {
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

/// A server's refusal, as the transfer engine words it.
fn transfer_error(error: webdav::Error) -> transfer::Error {
    match error {
        webdav::Error::Unauthorized => transfer::Error::Unauthorized,
        webdav::Error::Forbidden => transfer::Error::Forbidden,
        webdav::Error::Status(code) => transfer::Error::Status(code),
        webdav::Error::Unreachable(why) => transfer::Error::Connection(why),
        webdav::Error::Unreadable(why) => transfer::Error::Protocol(why),
    }
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
    let open = with_state(|state| {
        let transfer = state.queue.iter_mut().find(|transfer| transfer.id == id && transfer.state == TransferState::Active)?;
        match result {
            Ok(path) => {
                transfer.state = TransferState::Done;
                transfer.done = transfer.total;
                transfer.result = Some(path.clone());
                transfer.open_when_done.then_some((path, transfer.icon))
            }
            Err(transfer::Error::Cancelled) => None,
            Err(err) => {
                transfer.state = TransferState::Stopped;
                let (problem, detail) = match &err {
                    transfer::Error::Unauthorized => (Problem::Password, String::new()),
                    transfer::Error::Forbidden => (Problem::Forbidden, String::new()),
                    transfer::Error::Status(code) => (Problem::Http, code.to_string()),
                    transfer::Error::Connection(_) => (Problem::Connection, String::new()),
                    transfer::Error::Disk(_) => (Problem::Disk, String::new()),
                    transfer::Error::Protocol(_) => (Problem::Unreadable, String::new()),
                    transfer::Error::Cancelled => (Problem::None, String::new()),
                };
                eprintln!("downloads: {} stopped: {err}", transfer.name);
                transfer.problem = problem;
                transfer.detail = detail;
                None
            }
        }
    })
    .flatten();
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
    with_downloads(|downloads| {
        downloads.set_queue(ModelRc::new(VecModel::from(rows)));
        downloads.set_active_count(active);
        downloads.set_done_count(done);
    });
    // Busy while a file is on its way: a window in the background is no reason to slow down.
    with_state(|state| {
        if active > 0 && state.activity.is_none() {
            state.activity = Some(platform::begin_activity("Downloading files"));
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
            if let Kind::File { dest, .. } = &transfer.kind {
                transfer.done = transfer::kept(dest);
            }
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

fn remove_transfer(id: i32) {
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
        downloads.set_form_address(SharedString::new());
        downloads.set_form_username(SharedString::new());
        downloads.set_form_password(SharedString::new());
        downloads.set_form_error(FormError::None);
        downloads.set_test_state(TestState::None);
        downloads.set_form_open(true);
    });
}

fn edit_server(index: i32) {
    let Ok(index) = usize::try_from(index) else { return };
    let fields = with_state(|state| {
        let server = state.servers.get_mut(index)?;
        let client = server.client();
        state.editing = Some(index);
        state.current = Some(index);
        Some((server.name.clone(), format!("{}/", server.url), server.username.clone(), client.password))
    })
    .flatten();
    let Some((name, address, username, password)) = fields else { return };
    with_downloads(|downloads| {
        downloads.set_current(index as i32);
        downloads.set_form_editing(true);
        downloads.set_form_name(name.into());
        downloads.set_form_address(address.into());
        downloads.set_form_username(username.into());
        downloads.set_form_password(password.into());
        downloads.set_form_error(FormError::None);
        downloads.set_test_state(TestState::None);
        downloads.set_form_open(true);
    });
}

/// The form's fields, as typed.
fn form_fields() -> Option<(String, String, String, String)> {
    let ui = STATE.with(|state| state.borrow().as_ref().and_then(|state| state.ui.upgrade()))?;
    let downloads = ui.global::<Downloads>();
    Some((
        downloads.get_form_name().trim().to_string(),
        downloads.get_form_address().trim().to_string(),
        downloads.get_form_username().trim().to_string(),
        downloads.get_form_password().to_string(),
    ))
}

fn test_connection() {
    let Some((_, address, username, password)) = form_fields() else { return };
    if !webdav::Server::is_address(&address) {
        with_downloads(|downloads| downloads.set_form_error(FormError::Address));
        return;
    }
    let request = with_state(|state| {
        state.test_request += 1;
        state.test_request
    })
    .unwrap_or(0);
    with_downloads(|downloads| {
        downloads.set_form_error(FormError::None);
        downloads.set_test_state(TestState::Testing);
    });
    send(Task::Test { request, server: webdav::Server { url: webdav::Server::normalized_url(&address), username, password } });
}

fn tested(request: u64, result: Result<TestResult, webdav::Error>) {
    let current = with_state(|state| state.test_request).unwrap_or(0);
    if request != current {
        return;
    }
    let host = form_fields().map(|(_, address, _, _)| webdav::Server::host(&address)).unwrap_or_default();
    with_downloads(|downloads| match result {
        Ok(found) => {
            downloads.set_test_folders(found.folders);
            downloads.set_test_files(found.files);
            downloads.set_test_probed(found.ranges.is_some());
            downloads.set_test_ranges(found.ranges.unwrap_or(false));
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
    let Some((name, address, username, password)) = form_fields() else { return };
    if name.is_empty() {
        with_downloads(|downloads| downloads.set_form_error(FormError::Name));
        return;
    }
    if !webdav::Server::is_address(&address) {
        with_downloads(|downloads| downloads.set_form_error(FormError::Address));
        return;
    }
    let url = webdav::Server::normalized_url(&address);
    let index = with_state(|state| {
        let index = match state.editing.filter(|index| *index < state.servers.len()) {
            Some(index) => {
                let server = &mut state.servers[index];
                server.name = name.clone();
                server.url = url.clone();
                server.username = username.clone();
                server.password = password.clone();
                server.status = ServerStatus::Unknown;
                index
            }
            None => {
                state.servers.push(Server { name: name.clone(), url: url.clone(), username: username.clone(), password: password.clone(), status: ServerStatus::Unknown });
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
    let records: Vec<database::WebdavServer> = state.servers.iter().map(Server::record).collect();
    if let Err(err) = database::save_webdav_servers(&records) {
        eprintln!("downloads: cannot keep the servers in the database: {err}");
    }
}

/// The servers from the database; or, the first time after 0.4.17, from settings.toml with their
/// passwords from the Keychain, which are moved into the database and gone from where they were.
/// Nothing is moved, and nothing deleted, unless every password could be read.
fn load_servers(settings: &Rc<RefCell<Settings>>) -> Vec<Server> {
    let from_database = database::webdav_servers();
    let old: Vec<settings::WebdavServer> = settings.borrow().webdav.clone();
    if !from_database.is_empty() || old.is_empty() || !database::is_open() {
        return from_database.into_iter().map(Server::from_record).collect();
    }
    let account = |server: &settings::WebdavServer| format!("{}@{}", server.username, server.url);
    let mut all_read = true;
    let moved: Vec<Server> = old
        .iter()
        .map(|server| {
            let password = match platform::keychain_read(OLD_KEYCHAIN_SERVICE, &account(server)) {
                Ok(password) => password.unwrap_or_default(),
                Err(err) => {
                    eprintln!("downloads: cannot read the password of {} from the Keychain: {err}", server.name);
                    all_read = false;
                    String::new()
                }
            };
            Server { name: server.name.clone(), url: server.url.clone(), username: server.username.clone(), password, status: ServerStatus::Unknown }
        })
        .collect();
    if !all_read {
        return moved;
    }
    let records: Vec<database::WebdavServer> = moved.iter().map(Server::record).collect();
    match database::save_webdav_servers(&records) {
        Ok(()) => {
            for server in &old {
                if let Err(err) = platform::keychain_delete(OLD_KEYCHAIN_SERVICE, &account(server)) {
                    eprintln!("downloads: cannot forget the old Keychain item of {}: {err}", server.name);
                }
            }
            let mut settings = settings.borrow_mut();
            settings.webdav.clear();
            settings.save();
            eprintln!("downloads: {} server(s) moved from settings.toml and the Keychain into the database", moved.len());
        }
        Err(err) => eprintln!("downloads: cannot move the servers into the database: {err}"),
    }
    moved
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_of_a_tree_goes_under_the_folder_and_nowhere_else() {
        let dest = Path::new("/Users/x/Downloads/FinchGram/Movies");
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
