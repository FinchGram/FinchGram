//! Music (the maintainer's word of 2026-10-30, the canvas "FinchGram 音乐" of 2026-10-31;
//! docs/music.md): a music player of the toolbox's own, a tool like Downloads. Its sources are
//! servers of the user's own that speak the Subsonic API (`subsonic`): Navidrome, gonic, LMS,
//! Ampache, Nextcloud Music, Koel and the rest, each a library of its own, looked at one at a
//! time (never merged: the maintainer's word). Songs stream through the app's own HTTP client
//! into a cache file that the music mpv reads as it arrives (`fetch`, src/player/music.rs,
//! src/player/stream.rs): the vendored FFmpeg has no network code, by the project's decision, and
//! keeps none. This module keeps the page's state on the UI thread and fills the Music global
//! (ui/music.slint), which the page (ui/pages/music.slint) and the player bar show in every theme.
//!
//! Threads (docs/architecture.md): one worker asks the server, one request at a time in the order
//! asked, and answers through the event loop; an answer that is not the current question any more
//! is dropped when it comes back. A second worker fetches covers, which are many and slow, so
//! that a listing never waits behind them. Each song on its way has a fetching thread of its own
//! (`fetch`). The sources, with their passwords, are the app's database's (src/database.rs); the
//! volume, the order and the queue's column are settings (src/settings.rs).

pub mod fetch;
pub mod subsonic;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use sha2::Digest as _;
use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};

use crate::player::music as player;
use crate::settings::Settings;
use crate::{AlbumRow, MainWindow, Music, MusicFormError, MusicSection, MusicState, MusicView, NameRow, QueueRow, RepeatMode, SourceRow, SourceStatus, SourceTest, TrackRow, database, images};
use subsonic::{AlbumList, Client, Counts, Starrable};

/// Albums fetched at a time; "More albums" fetches the next page.
const ALBUM_PAGE: u32 = 100;
/// Songs fetched at a time, for the list of every song.
const SONG_PAGE: u32 = 200;
/// Covers asked for at once; the rest wait their turn.
const COVERS_AT_ONCE: usize = 3;
/// A cover's longer side, in pixels, as asked of the server.
const COVER_SIZE: u32 = 300;
/// A song counts as listened to past this much of it, or this long, whichever comes first
/// (Last.fm's rule, which Subsonic servers follow for their play counts).
const LISTENED_PART: f64 = 0.5;
const LISTENED_SECONDS: f64 = 240.0;
/// The next song is fetched ahead once the one playing is this far, when its own fetch is not
/// done yet: both would share the connection otherwise.
const PREFETCH_PART: f64 = 0.6;
/// The volume is written to settings.toml this long after the slider last moved.
const VOLUME_SETTLE: Duration = Duration::from_millis(800);

/// A source as the page keeps it.
#[derive(Clone)]
struct Source {
    name: String,
    /// "subsonic".
    kind: String,
    /// As typed, without a port.
    address: String,
    /// 0 for the address's own.
    port: u16,
    username: String,
    password: String,
    status: SourceStatus,
}

impl Source {
    fn from_record(record: database::MusicSourceRecord) -> Source {
        Source { name: record.name, kind: record.kind, address: record.address, port: record.port, username: record.username, password: record.password, status: SourceStatus::Unknown }
    }

    fn record(&self) -> database::MusicSourceRecord {
        database::MusicSourceRecord { name: self.name.clone(), kind: self.kind.clone(), address: self.address.clone(), port: self.port, username: self.username.clone(), password: self.password.clone() }
    }

    fn client(&self, agent: &ureq::Agent) -> Client {
        Client::new(&self.address, self.port, &self.username, &self.password, agent.clone())
    }

    fn row(&self) -> SourceRow {
        SourceRow { name: self.name.clone().into(), host: subsonic::host_of(&self.address).into(), kind: "Subsonic".into(), status: self.status }
    }

    /// The folder of the source's cached songs and covers: a hash of its address and account,
    /// so that two sources never share a file.
    fn key(&self) -> String {
        let mut hasher = sha2::Sha256::new();
        hasher.update(self.address.as_bytes());
        hasher.update(b"\0");
        hasher.update(self.username.as_bytes());
        hasher.finalize().iter().take(8).map(|byte| format!("{byte:02x}")).collect()
    }
}

/// What the middle column shows, in the page's own terms.
#[derive(Debug, Clone, PartialEq)]
enum Shown {
    /// No source yet.
    Nothing,
    /// The form for a source.
    Form,
    /// A grid of albums: a section's (ordered), an artist's or a genre's (`origin`).
    Albums,
    /// One album with its songs.
    Album,
    /// Artists, genres or playlists.
    Names,
    /// Songs: a playlist's, the favourites, every song.
    Tracks,
}

/// Where a grid or a list of songs came from, when not a section itself: the way back.
#[derive(Debug, Clone, PartialEq)]
enum Origin {
    Artist { name: String },
    Genre { name: String },
    Playlist { name: String },
}

/// A song as the queue holds it.
#[derive(Debug, Clone, PartialEq)]
struct Track {
    id: String,
    title: String,
    artist: String,
    album: String,
    album_id: String,
    cover_art: String,
    /// Seconds.
    duration: u64,
    size: u64,
    suffix: String,
    starred: bool,
    /// The source it is on.
    source: usize,
    /// Put in the queue by the user, after what an album or a list brought.
    added: bool,
}

impl Track {
    fn from_song(song: &subsonic::Song, source: usize, album_fallback: &subsonic::Album) -> Track {
        Track {
            id: song.id.clone(),
            title: song.title.clone(),
            artist: if song.artist.is_empty() { album_fallback.artist.clone() } else { song.artist.clone() },
            album: if song.album.is_empty() { album_fallback.name.clone() } else { song.album.clone() },
            album_id: if song.album_id.is_empty() { album_fallback.id.clone() } else { song.album_id.clone() },
            cover_art: if song.cover_art.is_empty() { album_fallback.cover_art.clone() } else { song.cover_art.clone() },
            duration: song.duration,
            size: song.size,
            suffix: song.suffix.clone(),
            starred: song.starred.is_some(),
            source,
            added: false,
        }
    }
}

/// A song mpv plays or is about to: where from, and the fetch bringing it when it is not whole
/// on the disk yet.
struct Playing {
    /// Its place in the queue.
    at: usize,
    /// What mpv was given: a "finchgram://" stream, or the cache file's path.
    url: String,
    fetch: Option<fetch::Fetch>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Repeat {
    Off,
    All,
    One,
}

impl Repeat {
    fn from_setting(text: &str) -> Repeat {
        match text {
            "all" => Repeat::All,
            "one" => Repeat::One,
            _ => Repeat::Off,
        }
    }

    fn setting(self) -> &'static str {
        match self {
            Repeat::Off => "off",
            Repeat::All => "all",
            Repeat::One => "one",
        }
    }

    fn mode(self) -> RepeatMode {
        match self {
            Repeat::Off => RepeatMode::Off,
            Repeat::All => RepeatMode::All,
            Repeat::One => RepeatMode::One,
        }
    }
}

/// What plays and what follows.
struct Playback {
    queue: Vec<Track>,
    /// The order the queue plays in: its indexes, shuffled or not.
    order: Vec<usize>,
    /// Where in `order` the song playing is.
    at: Option<usize>,
    current: Option<Playing>,
    /// The next song, opened ahead by mpv.
    next: Option<Playing>,
    shuffle: bool,
    repeat: Repeat,
    volume: i32,
    paused: bool,
    position: f64,
    duration: f64,
    /// The song playing was told to the server as listened to.
    scrobbled: bool,
    /// "Kind of Blue": what the queue came from.
    from: String,
}

/// What the worker does, one at a time.
enum Task {
    Check { index: usize, client: Client },
    Test { request: u64, client: Client },
    Albums { request: u64, client: Client, list: AlbumList, offset: u32 },
    Album { request: u64, client: Client, id: String },
    Artists { request: u64, client: Client },
    Artist { request: u64, client: Client, id: String },
    Genres { request: u64, client: Client },
    Playlists { request: u64, client: Client },
    Playlist { request: u64, client: Client, id: String },
    Starred { request: u64, client: Client },
    Songs { request: u64, client: Client, offset: u32 },
    /// The songs of an album, to play or queue them.
    Songs0f { request: u64, client: Client, id: String, then: Then },
    Star { client: Client, what: Starrable, on: bool },
    Scrobble { client: Client, id: String, submission: bool },
    /// The cache measured, and trimmed to `limit` bytes (0 for none) when it has grown past it.
    Cache { folder: PathBuf, limit: u64 },
}

/// What to do with an album's songs once they come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Then {
    Play,
    PlayShuffled,
    Queue,
}

/// What the cover worker does.
struct CoverTask {
    client: Client,
    id: String,
    path: PathBuf,
}

/// What a test found.
struct TestResult {
    info: subsonic::ServerInfo,
    extensions: usize,
}

struct State {
    ui: slint::Weak<MainWindow>,
    settings: Rc<RefCell<Settings>>,
    agent: ureq::Agent,
    sources: Vec<Source>,
    /// The source looked at.
    current: Option<usize>,
    /// The source the form edits.
    editing: Option<usize>,
    section: MusicSection,
    shown: Shown,
    /// What the middle column held before the form opened, to come back to.
    before_form: Option<Shown>,
    origin: Option<Origin>,
    /// The grid's albums, every page fetched so far.
    albums: Vec<subsonic::Album>,
    /// The album open.
    album: Option<subsonic::Album>,
    /// The songs shown: the album's, the list's.
    tracks: Vec<Track>,
    names: Vec<NameRow>,
    filter: String,
    sort: AlbumList,
    /// There may be another page of albums or songs.
    more: bool,
    loading_more: bool,
    /// The middle column's current question; an older answer is dropped.
    request: u64,
    test_request: u64,
    worker: Option<Sender<Task>>,
    cover_worker: Option<Sender<CoverTask>>,
    /// Covers decoded, by the source's key and the cover's id; none for one the server has not.
    covers: HashMap<(String, String), Option<Image>>,
    /// Covers asked for, in order, and how many are on their way.
    cover_queue: VecDeque<(usize, String)>,
    covers_asked: HashSet<(String, String)>,
    covers_in_flight: usize,
    playback: Playback,
    volume_timer: slint::Timer,
    /// The models the page shows, kept to change a row in place when its cover comes.
    album_rows: Rc<VecModel<AlbumRow>>,
    track_rows: Rc<VecModel<TrackRow>>,
    queue_rows: Rc<VecModel<QueueRow>>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub fn connect(ui: &MainWindow, settings: Rc<RefCell<Settings>>) {
    let sources: Vec<Source> = database::music_sources().into_iter().map(Source::from_record).collect();
    let (volume, shuffle, repeat, queue_shown) = {
        let settings = settings.borrow();
        (settings.music.volume.min(100) as i32, settings.music.shuffle, Repeat::from_setting(&settings.music.repeat), settings.music.queue_shown)
    };
    let album_rows = Rc::new(VecModel::default());
    let track_rows = Rc::new(VecModel::default());
    let queue_rows = Rc::new(VecModel::default());
    STATE.with(|state| {
        *state.borrow_mut() = Some(State {
            ui: ui.as_weak(),
            settings: settings.clone(),
            agent: subsonic::agent(),
            sources,
            current: None,
            editing: None,
            section: MusicSection::Albums,
            shown: Shown::Nothing,
            before_form: None,
            origin: None,
            albums: Vec::new(),
            album: None,
            tracks: Vec::new(),
            names: Vec::new(),
            filter: String::new(),
            sort: AlbumList::Newest,
            more: false,
            loading_more: false,
            request: 0,
            test_request: 0,
            worker: None,
            cover_worker: None,
            covers: HashMap::new(),
            cover_queue: VecDeque::new(),
            covers_asked: HashSet::new(),
            covers_in_flight: 0,
            playback: Playback { queue: Vec::new(), order: Vec::new(), at: None, current: None, next: None, shuffle, repeat, volume, paused: false, position: 0.0, duration: 0.0, scrobbled: false, from: String::new() },
            volume_timer: slint::Timer::default(),
            album_rows: album_rows.clone(),
            track_rows: track_rows.clone(),
            queue_rows: queue_rows.clone(),
        })
    });
    let music = ui.global::<Music>();
    music.set_albums(ModelRc::from(album_rows));
    music.set_tracks(ModelRc::from(track_rows));
    music.set_queue(ModelRc::from(queue_rows));
    music.set_volume(volume);
    music.set_shuffle(shuffle);
    music.set_repeat(repeat.mode());
    music.set_queue_open(queue_shown);
    player::listen(heard);
    show_sources();
    music.on_opened(opened);
    music.on_select_source(select_source);
    music.on_add_source(add_source);
    music.on_edit_source(edit_source);
    music.on_remove_source(remove_source);
    music.on_submit_form(submit_form);
    music.on_cancel_form(cancel_form);
    music.on_test_connection(test_connection);
    music.on_remove_from_form(remove_from_form);
    music.on_choose_section(choose_section);
    music.on_set_sort(|sort| set_sort(&sort));
    music.on_set_filter(|text| set_filter(&text));
    music.on_load_more(load_more);
    music.on_refresh(refresh);
    music.on_open_album(|id| open_album(&id));
    music.on_open_name(|id| open_name(&id));
    music.on_go_back(go_back);
    music.on_play_album(|id, shuffled| album_songs(&id, if shuffled { Then::PlayShuffled } else { Then::Play }));
    music.on_queue_album(|id| album_songs(&id, Then::Queue));
    music.on_play_track(play_track);
    music.on_queue_track(queue_track);
    music.on_star_album(|id, on| star_album(&id, on));
    music.on_star_track(star_track);
    music.on_star_song(star_song);
    music.on_toggle_pause(toggle_pause);
    music.on_next(next);
    music.on_previous(previous);
    music.on_seek(seek);
    music.on_set_volume(set_volume);
    music.on_toggle_shuffle(toggle_shuffle);
    music.on_cycle_repeat(cycle_repeat);
    music.on_play_queued(play_queued);
    music.on_remove_queued(remove_queued);
    music.on_clear_queue(clear_queue);
    music.on_toggle_queue(toggle_queue);
    music.on_stop(stop);
}

// ---- reaching the state and the page ----------------------------------------------------------

/// Change the state (never from within another `with_state`).
fn with_state<R>(change: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|state| state.borrow_mut().as_mut().map(change))
}

/// Change the page, when the window still exists.
fn with_music(change: impl FnOnce(&Music)) {
    let ui = STATE.with(|state| state.borrow().as_ref().and_then(|state| state.ui.upgrade()));
    if let Some(ui) = ui {
        change(&ui.global::<Music>());
    }
}

/// The worker, started when first needed.
fn send(task: Task) {
    let worker = with_state(|state| state.worker.get_or_insert_with(start_worker).clone());
    if let Some(worker) = worker
        && worker.send(task).is_err()
    {
        eprintln!("music: the worker thread is gone");
        with_state(|state| state.worker = None);
    }
}

fn start_worker() -> Sender<Task> {
    let (sender, tasks) = mpsc::channel::<Task>();
    let started = std::thread::Builder::new().name("music".into()).spawn(move || {
        for task in tasks {
            match task {
                Task::Check { index, client } => {
                    let result = client.ping();
                    let _ = slint::invoke_from_event_loop(move || checked(index, result));
                }
                Task::Test { request, client } => {
                    let result = client.ping().map(|info| TestResult { extensions: client.extensions().map(|extensions| extensions.len()).unwrap_or(0), info });
                    let _ = slint::invoke_from_event_loop(move || tested(request, result));
                }
                Task::Albums { request, client, list, offset } => {
                    let result = client.albums(&list, ALBUM_PAGE, offset);
                    let _ = slint::invoke_from_event_loop(move || albums_came(request, offset, result));
                }
                Task::Album { request, client, id } => {
                    let result = client.album(&id);
                    let _ = slint::invoke_from_event_loop(move || album_came(request, result));
                }
                Task::Artists { request, client } => {
                    let result = client.artists();
                    let _ = slint::invoke_from_event_loop(move || artists_came(request, result));
                }
                Task::Artist { request, client, id } => {
                    let result = client.artist(&id);
                    let _ = slint::invoke_from_event_loop(move || artist_came(request, result));
                }
                Task::Genres { request, client } => {
                    let result = client.genres();
                    let _ = slint::invoke_from_event_loop(move || genres_came(request, result));
                }
                Task::Playlists { request, client } => {
                    let result = client.playlists();
                    let _ = slint::invoke_from_event_loop(move || playlists_came(request, result));
                }
                Task::Playlist { request, client, id } => {
                    let result = client.playlist(&id);
                    let _ = slint::invoke_from_event_loop(move || playlist_came(request, result));
                }
                Task::Starred { request, client } => {
                    let result = client.starred();
                    let _ = slint::invoke_from_event_loop(move || starred_came(request, result));
                }
                Task::Songs { request, client, offset } => {
                    let result = client.search("", Counts { songs: SONG_PAGE, song_offset: offset, ..Counts::default() }).map(|found| found.song);
                    let _ = slint::invoke_from_event_loop(move || songs_came(request, offset, result));
                }
                Task::Songs0f { request, client, id, then } => {
                    let result = client.album(&id);
                    let _ = slint::invoke_from_event_loop(move || album_songs_came(request, result, then));
                }
                Task::Star { client, what, on } => {
                    if let Err(err) = client.star(&what, on) {
                        eprintln!("music: cannot star: {err}");
                    }
                }
                Task::Scrobble { client, id, submission } => {
                    if let Err(err) = client.scrobble(&id, submission) {
                        eprintln!("music: cannot tell the server a song plays: {err}");
                    }
                }
                Task::Cache { folder, limit } => {
                    let bytes = trim_cache(&folder, limit);
                    let _ = slint::invoke_from_event_loop(move || show_cache(bytes));
                }
            }
        }
    });
    if let Err(err) = started {
        eprintln!("music: cannot start the worker thread: {err}");
    }
    sender
}

fn send_cover(task: CoverTask) {
    let worker = with_state(|state| state.cover_worker.get_or_insert_with(start_cover_worker).clone());
    if let Some(worker) = worker
        && worker.send(task).is_err()
    {
        eprintln!("music: the covers' thread is gone");
        with_state(|state| state.cover_worker = None);
    }
}

fn start_cover_worker() -> Sender<CoverTask> {
    let (sender, tasks) = mpsc::channel::<CoverTask>();
    let started = std::thread::Builder::new().name("music-covers".into()).spawn(move || {
        for CoverTask { client, id, path } in tasks {
            let fetched = client.cover_art(&id, COVER_SIZE).and_then(|(bytes, _)| {
                if let Some(folder) = path.parent() {
                    std::fs::create_dir_all(folder).map_err(|err| subsonic::Error::Unreadable(err.to_string()))?;
                }
                std::fs::write(&path, bytes).map_err(|err| subsonic::Error::Unreadable(err.to_string()))
            });
            let _ = slint::invoke_from_event_loop(move || cover_fetched(id, path, fetched.is_ok()));
        }
    });
    if let Err(err) = started {
        eprintln!("music: cannot start the covers' thread: {err}");
    }
    sender
}

/// The client of the source looked at, if any.
fn current_client() -> Option<(usize, Client)> {
    with_state(|state| {
        let index = state.current?;
        let source = state.sources.get(index)?;
        Some((index, source.client(&state.agent)))
    })
    .flatten()
}

/// A new question for the middle column: its number, and the client to ask.
fn ask(loading: bool) -> Option<(u64, usize, Client)> {
    let asked = with_state(|state| {
        let index = state.current?;
        let source = state.sources.get(index)?;
        state.request += 1;
        Some((state.request, index, source.client(&state.agent)))
    })
    .flatten();
    if asked.is_some() && loading {
        with_music(|music| {
            music.set_state(MusicState::Loading);
            music.set_problem(SharedString::new());
        });
    }
    asked
}

/// Whether `request` is still the question asked.
fn current_request(request: u64) -> bool {
    with_state(|state| state.request == request).unwrap_or(false)
}

// ---- the sources ------------------------------------------------------------------------------

/// The page is shown: every source is asked whether it answers, the source looked at last (or
/// the first) is looked at when none is yet, and the cache is measured.
fn opened() {
    let (checks, current, wanted) = with_state(|state| {
        let mut checks = Vec::new();
        for (index, source) in state.sources.iter_mut().enumerate() {
            source.status = SourceStatus::Checking;
            checks.push(Task::Check { index, client: source.client(&state.agent) });
        }
        (checks, state.current, state.settings.borrow().music.source.clone())
    })
    .unwrap_or_default();
    for check in checks {
        send(check);
    }
    show_sources();
    measure_cache();
    if current.is_none() {
        let index = with_state(|state| state.sources.iter().position(|source| source.name == wanted).or(if state.sources.is_empty() { None } else { Some(0) })).flatten();
        match index {
            Some(index) => select_source(index as i32),
            None => {
                with_state(|state| state.shown = Shown::Nothing);
                show_view();
            }
        }
    }
}

/// Look at a source: its library's section as chosen.
fn select_source(index: i32) {
    let Ok(index) = usize::try_from(index) else { return };
    let known = with_state(|state| {
        if index >= state.sources.len() {
            return false;
        }
        state.current = Some(index);
        state.editing = None;
        state.origin = None;
        state.albums.clear();
        state.album = None;
        state.tracks.clear();
        state.names.clear();
        state.filter.clear();
        state.before_form = None;
        {
            let mut settings = state.settings.borrow_mut();
            if settings.music.source != state.sources[index].name {
                settings.music.source = state.sources[index].name.clone();
                settings.save();
            }
        }
        true
    })
    .unwrap_or(false);
    if !known {
        return;
    }
    with_music(|music| {
        music.set_current(index as i32);
        music.set_filter(SharedString::new());
    });
    show_sources();
    load_section();
}

fn checked(index: usize, result: Result<subsonic::ServerInfo, subsonic::Error>) {
    with_state(|state| {
        if let Some(source) = state.sources.get_mut(index) {
            source.status = status_of(&result.err());
        }
    });
    show_sources();
}

/// What a source's answer says of it.
fn status_of(error: &Option<subsonic::Error>) -> SourceStatus {
    match error {
        None => SourceStatus::Connected,
        Some(err) if err.is_password() => SourceStatus::PasswordRejected,
        Some(subsonic::Error::Unreachable(_)) => SourceStatus::NotReachable,
        Some(subsonic::Error::Unreadable(_)) | Some(subsonic::Error::Http(_)) => SourceStatus::Failed,
        // The server is there and has an opinion: the account works.
        Some(subsonic::Error::Server { .. }) => SourceStatus::Connected,
    }
}

fn show_sources() {
    let (rows, current) = with_state(|state| (state.sources.iter().map(Source::row).collect::<Vec<_>>(), state.current)).unwrap_or_default();
    with_music(|music| {
        music.set_sources(ModelRc::new(VecModel::from(rows)));
        music.set_current(current.map_or(-1, |index| index as i32));
    });
}

/// Something the page shows changed: the view, with what is in it.
fn show_view() {
    let (shown, section, origin, title_source, more, loading_more, sort, sortable, back, playing_id, current_at) = with_state(|state| {
        let current_at = state.playback.at.and_then(|at| state.playback.order.get(at).copied());
        let playing_id = current_at.and_then(|index| state.playback.queue.get(index)).map(|track| track.id.clone());
        let sortable = state.shown == Shown::Albums && state.origin.is_none();
        let back = state.origin.is_some() && state.shown != Shown::Album;
        (state.shown.clone(), state.section, state.origin.clone(), state.current.and_then(|index| state.sources.get(index)).map(|source| source.name.clone()), state.more, state.loading_more, state.sort.clone(), sortable, back, playing_id, current_at)
    })
    .unwrap_or((Shown::Nothing, MusicSection::Albums, None, None, false, false, AlbumList::Newest, false, false, None, None));
    let _ = (title_source, current_at);
    let (title, subtitle) = with_state(|state| titles(state, &origin)).unwrap_or_default();
    with_music(|music| {
        music.set_view(match shown {
            Shown::Nothing => MusicView::None,
            Shown::Form => MusicView::Form,
            Shown::Albums => MusicView::Albums,
            Shown::Album => MusicView::Album,
            Shown::Names => MusicView::Names,
            Shown::Tracks => MusicView::Tracks,
        });
        music.set_section(section);
        music.set_title(title.into());
        music.set_subtitle(subtitle.into());
        music.set_back(back);
        music.set_more(more);
        music.set_loading_more(loading_more);
        music.set_sortable(sortable);
        music.set_sort(sort_name(&sort).into());
    });
    show_albums();
    show_album();
    show_tracks(playing_id.as_deref());
    show_names();
}

/// The heading and the line under it for what is shown.
fn titles(state: &State, origin: &Option<Origin>) -> (String, String) {
    let songs_line = |tracks: &[Track]| {
        let seconds: u64 = tracks.iter().map(|track| track.duration).sum();
        if seconds > 0 { format!("{} · {}", count_songs(state, tracks.len()), clock(seconds)) } else { count_songs(state, tracks.len()) }
    };
    let albums_line = || if state.more { format!("{}+", count_albums(state, state.albums.len())) } else { count_albums(state, state.albums.len()) };
    match (&state.shown, origin) {
        (Shown::Albums, Some(Origin::Artist { name })) => (name.clone(), count_albums(state, state.albums.len())),
        (Shown::Albums, Some(Origin::Genre { name })) => (name.clone(), albums_line()),
        (Shown::Albums, _) => (section_name(state, MusicSection::Albums), albums_line()),
        (Shown::Tracks, Some(Origin::Playlist { name })) => (name.clone(), songs_line(&state.tracks)),
        (Shown::Tracks, _) => (section_name(state, state.section), if state.more { format!("{}+", count_songs(state, state.tracks.len())) } else { songs_line(&state.tracks) }),
        (Shown::Names, _) => {
            let n = state.names.len() as i32;
            let line = with_words(state, |words| match state.section {
                MusicSection::Artists => words.invoke_artists(n),
                MusicSection::Genres => words.invoke_genres(n),
                _ => words.invoke_playlists(n),
            })
            .map(|line| line.to_string())
            .unwrap_or_default();
            (section_name(state, state.section), line)
        }
        _ => (String::new(), String::new()),
    }
}

/// The page's words (ui/music.slint), through the window: `state` is the caller's, so that no
/// helper reaches for the state's cell while it is borrowed.
fn with_words<R>(state: &State, ask: impl FnOnce(&crate::MusicWords) -> R) -> Option<R> {
    let ui = state.ui.upgrade()?;
    Some(ask(&ui.global::<crate::MusicWords>()))
}

fn section_name(state: &State, section: MusicSection) -> String {
    with_words(state, |words| words.invoke_section(section).to_string()).unwrap_or_default()
}

fn count_songs(state: &State, n: usize) -> String {
    with_words(state, |words| words.invoke_songs(n as i32).to_string()).unwrap_or_else(|| format!("{n} songs"))
}

fn count_albums(state: &State, n: usize) -> String {
    with_words(state, |words| words.invoke_albums(n as i32).to_string()).unwrap_or_else(|| format!("{n} albums"))
}

fn sort_name(sort: &AlbumList) -> &'static str {
    match sort {
        AlbumList::ByName => "name",
        AlbumList::ByArtist => "artist",
        AlbumList::Frequent => "frequent",
        AlbumList::Recent => "recent",
        AlbumList::Random => "random",
        _ => "newest",
    }
}

fn sort_of(name: &str) -> AlbumList {
    match name {
        "name" => AlbumList::ByName,
        "artist" => AlbumList::ByArtist,
        "frequent" => AlbumList::Frequent,
        "recent" => AlbumList::Recent,
        "random" => AlbumList::Random,
        _ => AlbumList::Newest,
    }
}

// ---- the library ------------------------------------------------------------------------------

/// The section chosen in the column.
fn choose_section(section: MusicSection) {
    with_state(|state| {
        state.section = section;
        state.origin = None;
        state.album = None;
        state.editing = None;
        state.before_form = None;
    });
    load_section();
}

/// The section's own contents, from the server.
fn load_section() {
    let section = with_state(|state| state.section).unwrap_or(MusicSection::Albums);
    let Some((request, _, client)) = ask(true) else {
        with_state(|state| state.shown = Shown::Nothing);
        show_view();
        return;
    };
    let sort = with_state(|state| {
        state.albums.clear();
        state.tracks.clear();
        state.names.clear();
        state.more = false;
        state.loading_more = false;
        state.shown = match section {
            MusicSection::Albums => Shown::Albums,
            MusicSection::Artists | MusicSection::Genres | MusicSection::Playlists => Shown::Names,
            MusicSection::Songs | MusicSection::Favourites => Shown::Tracks,
        };
        state.sort.clone()
    })
    .unwrap_or(AlbumList::Newest);
    show_view();
    send(match section {
        MusicSection::Albums => Task::Albums { request, client, list: sort, offset: 0 },
        MusicSection::Artists => Task::Artists { request, client },
        MusicSection::Songs => Task::Songs { request, client, offset: 0 },
        MusicSection::Genres => Task::Genres { request, client },
        MusicSection::Playlists => Task::Playlists { request, client },
        MusicSection::Favourites => Task::Starred { request, client },
    });
}

/// What is shown, asked for again.
fn refresh() {
    let (shown, origin, album_id, section) = with_state(|state| (state.shown.clone(), state.origin.clone(), state.album.as_ref().map(|album| album.id.clone()), state.section)).unwrap_or((Shown::Nothing, None, None, MusicSection::Albums));
    match (shown, origin) {
        (Shown::Album, _) => {
            if let Some(id) = album_id {
                open_album(&id);
            }
        }
        (Shown::Albums, Some(Origin::Artist { .. })) | (Shown::Tracks, Some(Origin::Playlist { .. })) => {
            // An artist's albums or a playlist: back to the list, which comes fresh.
            with_state(|state| state.origin = None);
            load_section();
        }
        (Shown::Albums, Some(Origin::Genre { name })) => load_genre(&name),
        (Shown::Form, _) | (Shown::Nothing, _) => {}
        _ => {
            let _ = section;
            load_section();
        }
    }
}

fn set_sort(sort: &str) {
    with_state(|state| state.sort = sort_of(sort));
    let origin = with_state(|state| state.origin.clone()).flatten();
    if origin.is_none() {
        load_section();
    }
}

fn set_filter(text: &str) {
    with_state(|state| state.filter = text.trim().to_lowercase());
    show_albums();
}

/// The next page of albums or songs.
fn load_more() {
    let asked = with_state(|state| {
        if !state.more || state.loading_more {
            return None;
        }
        state.loading_more = true;
        let offset = if state.shown == Shown::Albums { state.albums.len() as u32 } else { state.tracks.len() as u32 };
        Some((state.shown.clone(), state.origin.clone(), state.sort.clone(), offset))
    })
    .flatten();
    let Some((shown, origin, sort, offset)) = asked else { return };
    let Some((request, _, client)) = ask(false) else { return };
    with_music(|music| music.set_loading_more(true));
    match (shown, origin) {
        (Shown::Albums, Some(Origin::Genre { name })) => send(Task::Albums { request, client, list: AlbumList::ByGenre(name), offset }),
        (Shown::Albums, _) => send(Task::Albums { request, client, list: sort, offset }),
        (Shown::Tracks, _) => send(Task::Songs { request, client, offset }),
        _ => {}
    }
}

fn albums_came(request: u64, offset: u32, result: Result<Vec<subsonic::Album>, subsonic::Error>) {
    if !current_request(request) {
        return;
    }
    match result {
        Ok(albums) => {
            with_state(|state| {
                state.more = albums.len() as u32 == ALBUM_PAGE;
                state.loading_more = false;
                if offset == 0 {
                    state.albums = albums;
                } else {
                    state.albums.extend(albums);
                }
            });
            with_music(|music| music.set_state(MusicState::Ready));
            show_view();
            ask_covers();
        }
        Err(err) => failed(&err),
    }
}

fn failed(err: &subsonic::Error) {
    let words = with_state(|state| {
        state.loading_more = false;
        problem_words(state, err)
    })
    .unwrap_or_else(|| err.to_string());
    with_music(|music| {
        music.set_loading_more(false);
        music.set_state(MusicState::Error);
        music.set_problem(words.into());
    });
}

/// Why something could not be fetched, as a sentence of the page's.
fn problem_words(state: &State, err: &subsonic::Error) -> String {
    let (kind, detail, code) = match err {
        err if err.is_password() => ("password", String::new(), String::new()),
        subsonic::Error::Server { message, code } => ("server", message.clone(), code.to_string()),
        subsonic::Error::Http(status) => ("http", status.to_string(), String::new()),
        subsonic::Error::Unreachable(why) => ("unreachable", why.clone(), String::new()),
        subsonic::Error::Unreadable(start) => ("unreadable", start.clone(), String::new()),
    };
    with_words(state, |words| words.invoke_problem(kind.into(), detail.clone().into(), code.clone().into()).to_string()).unwrap_or_else(|| err.to_string())
}

fn open_album(id: &str) {
    let Some((request, index, client)) = ask(true) else { return };
    with_state(|state| {
        // What is known of the album already: the grid's row, until the songs come.
        state.album = state.albums.iter().find(|album| album.id == id).cloned().or_else(|| state.album.clone().filter(|album| album.id == id));
        state.shown = Shown::Album;
        state.tracks.clear();
        state.more = false;
    });
    let _ = index;
    show_view();
    send(Task::Album { request, client, id: id.to_string() });
}

fn album_came(request: u64, result: Result<subsonic::Album, subsonic::Error>) {
    if !current_request(request) {
        return;
    }
    match result {
        Ok(album) => {
            with_state(|state| {
                let source = state.current.unwrap_or(0);
                state.tracks = album.song.iter().map(|song| Track::from_song(song, source, &album)).collect();
                state.album = Some(album);
            });
            with_music(|music| music.set_state(MusicState::Ready));
            show_view();
            ask_covers();
        }
        Err(err) => failed(&err),
    }
}

fn artists_came(request: u64, result: Result<Vec<subsonic::Artist>, subsonic::Error>) {
    if !current_request(request) {
        return;
    }
    match result {
        Ok(artists) => {
            with_state(|state| {
                state.names = artists.iter().map(|artist| NameRow { id: artist.id.clone().into(), name: artist.name.clone().into(), line: count_albums(state, artist.album_count as usize).into() }).collect();
            });
            with_music(|music| music.set_state(MusicState::Ready));
            show_view();
        }
        Err(err) => failed(&err),
    }
}

fn genres_came(request: u64, result: Result<Vec<subsonic::Genre>, subsonic::Error>) {
    if !current_request(request) {
        return;
    }
    match result {
        Ok(mut genres) => {
            genres.sort_by(|a, b| b.album_count.cmp(&a.album_count).then_with(|| a.value.cmp(&b.value)));
            with_state(|state| {
                state.names = genres.iter().map(|genre| NameRow { id: genre.value.clone().into(), name: genre.value.clone().into(), line: format!("{} · {}", count_albums(state, genre.album_count as usize), count_songs(state, genre.song_count as usize)).into() }).collect();
            });
            with_music(|music| music.set_state(MusicState::Ready));
            show_view();
        }
        Err(err) => failed(&err),
    }
}

fn playlists_came(request: u64, result: Result<Vec<subsonic::Playlist>, subsonic::Error>) {
    if !current_request(request) {
        return;
    }
    match result {
        Ok(playlists) => {
            with_state(|state| {
                state.names = playlists
                    .iter()
                    .map(|playlist| {
                        let line = if playlist.duration > 0 { format!("{} · {}", count_songs(state, playlist.song_count as usize), clock(playlist.duration)) } else { count_songs(state, playlist.song_count as usize) };
                        NameRow { id: playlist.id.clone().into(), name: playlist.name.clone().into(), line: line.into() }
                    })
                    .collect();
            });
            with_music(|music| music.set_state(MusicState::Ready));
            show_view();
        }
        Err(err) => failed(&err),
    }
}

/// An artist, a genre or a playlist opened from the names list.
fn open_name(id: &str) {
    let (section, name) = with_state(|state| (state.section, state.names.iter().find(|row| row.id == id).map(|row| row.name.to_string()).unwrap_or_default())).unwrap_or((MusicSection::Albums, String::new()));
    match section {
        MusicSection::Artists => {
            let Some((request, _, client)) = ask(true) else { return };
            with_state(|state| {
                state.origin = Some(Origin::Artist { name });
                state.shown = Shown::Albums;
                state.albums.clear();
                state.more = false;
            });
            show_view();
            send(Task::Artist { request, client, id: id.to_string() });
        }
        MusicSection::Genres => load_genre(id),
        MusicSection::Playlists => {
            let Some((request, _, client)) = ask(true) else { return };
            with_state(|state| {
                state.origin = Some(Origin::Playlist { name });
                state.shown = Shown::Tracks;
                state.tracks.clear();
                state.more = false;
            });
            show_view();
            send(Task::Playlist { request, client, id: id.to_string() });
        }
        _ => {}
    }
}

fn load_genre(name: &str) {
    let Some((request, _, client)) = ask(true) else { return };
    with_state(|state| {
        state.origin = Some(Origin::Genre { name: name.to_string() });
        state.shown = Shown::Albums;
        state.albums.clear();
        state.more = false;
    });
    show_view();
    send(Task::Albums { request, client, list: AlbumList::ByGenre(name.to_string()), offset: 0 });
}

fn artist_came(request: u64, result: Result<subsonic::Artist, subsonic::Error>) {
    if !current_request(request) {
        return;
    }
    match result {
        Ok(artist) => {
            with_state(|state| {
                let mut albums = artist.album;
                albums.sort_by(|a, b| a.year.unwrap_or(0).cmp(&b.year.unwrap_or(0)).then_with(|| a.name.cmp(&b.name)));
                state.albums = albums;
                state.more = false;
                state.origin = Some(Origin::Artist { name: artist.name });
            });
            with_music(|music| music.set_state(MusicState::Ready));
            show_view();
            ask_covers();
        }
        Err(err) => failed(&err),
    }
}

fn playlist_came(request: u64, result: Result<subsonic::Playlist, subsonic::Error>) {
    if !current_request(request) {
        return;
    }
    match result {
        Ok(playlist) => {
            with_state(|state| {
                let source = state.current.unwrap_or(0);
                let none = subsonic::Album::default();
                state.tracks = playlist.entry.iter().map(|song| Track::from_song(song, source, &none)).collect();
                state.origin = Some(Origin::Playlist { name: playlist.name });
            });
            with_music(|music| music.set_state(MusicState::Ready));
            show_view();
            ask_covers();
        }
        Err(err) => failed(&err),
    }
}

fn starred_came(request: u64, result: Result<subsonic::Starred, subsonic::Error>) {
    if !current_request(request) {
        return;
    }
    match result {
        Ok(starred) => {
            with_state(|state| {
                let source = state.current.unwrap_or(0);
                let none = subsonic::Album::default();
                state.tracks = starred.song.iter().map(|song| Track::from_song(song, source, &none)).collect();
            });
            with_music(|music| music.set_state(MusicState::Ready));
            show_view();
            ask_covers();
        }
        Err(err) => failed(&err),
    }
}

fn songs_came(request: u64, offset: u32, result: Result<Vec<subsonic::Song>, subsonic::Error>) {
    if !current_request(request) {
        return;
    }
    match result {
        Ok(songs) => {
            with_state(|state| {
                let source = state.current.unwrap_or(0);
                let none = subsonic::Album::default();
                let tracks: Vec<Track> = songs.iter().map(|song| Track::from_song(song, source, &none)).collect();
                state.more = songs.len() as u32 == SONG_PAGE;
                state.loading_more = false;
                if offset == 0 {
                    state.tracks = tracks;
                } else {
                    state.tracks.extend(tracks);
                }
            });
            with_music(|music| music.set_state(MusicState::Ready));
            show_view();
            ask_covers();
        }
        Err(err) => failed(&err),
    }
}

/// Back from an album to its grid, or from an artist's albums, a genre's or a playlist to the
/// names list.
fn go_back() {
    let shown = with_state(|state| state.shown.clone()).unwrap_or(Shown::Nothing);
    match shown {
        Shown::Album => {
            with_state(|state| {
                state.album = None;
                state.tracks.clear();
                state.shown = Shown::Albums;
            });
            with_music(|music| music.set_state(MusicState::Ready));
            show_view();
        }
        Shown::Albums | Shown::Tracks => {
            let had_names = with_state(|state| {
                state.origin = None;
                !state.names.is_empty()
            })
            .unwrap_or(false);
            if had_names {
                with_state(|state| {
                    state.shown = Shown::Names;
                    state.albums.clear();
                    state.tracks.clear();
                    state.more = false;
                });
                with_music(|music| music.set_state(MusicState::Ready));
                show_view();
            } else {
                load_section();
            }
        }
        _ => {}
    }
}

// ---- what the page shows ------------------------------------------------------------------------

fn show_albums() {
    let rows = with_state(|state| {
        let source_key = state.current.and_then(|index| state.sources.get(index)).map(Source::key).unwrap_or_default();
        let filter = state.filter.clone();
        state
            .albums
            .iter()
            .filter(|album| filter.is_empty() || album.name.to_lowercase().contains(&filter) || album.artist.to_lowercase().contains(&filter))
            .map(|album| album_row(album, state.covers.get(&(source_key.clone(), album.cover_art.clone())).cloned().flatten()))
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    with_state(|state| state.album_rows.set_vec(rows));
}

fn album_row(album: &subsonic::Album, cover: Option<Image>) -> AlbumRow {
    AlbumRow {
        id: album.id.clone().into(),
        name: album.name.clone().into(),
        artist: if album.display_artist.is_empty() { album.artist.clone() } else { album.display_artist.clone() }.into(),
        year: album.year.filter(|year| *year > 0).map(|year| year.to_string()).unwrap_or_default().into(),
        initials: initials(&album.name).into(),
        tint: tint(&album.id),
        has_cover: cover.is_some(),
        cover: cover.unwrap_or_default(),
        starred: album.starred.is_some(),
    }
}

fn show_album() {
    let (row, line) = with_state(|state| {
        let Some(album) = &state.album else { return (AlbumRow::default(), String::new()) };
        let source_key = state.current.and_then(|index| state.sources.get(index)).map(Source::key).unwrap_or_default();
        let cover = state.covers.get(&(source_key, album.cover_art.clone())).cloned().flatten();
        let mut parts = Vec::new();
        if let Some(year) = album.year.filter(|year| *year > 0) {
            parts.push(year.to_string());
        }
        if !album.genre.is_empty() {
            parts.push(album.genre.clone());
        }
        let songs = if state.tracks.is_empty() { album.song_count as usize } else { state.tracks.len() };
        let seconds = if state.tracks.is_empty() { album.duration } else { state.tracks.iter().map(|track| track.duration).sum() };
        parts.push(if seconds > 0 { format!("{}, {}", count_songs(state, songs), clock(seconds)) } else { count_songs(state, songs) });
        if let Some(first) = state.tracks.first()
            && !first.suffix.is_empty()
        {
            parts.push(first.suffix.to_uppercase());
        }
        (album_row(album, cover), parts.join(" · "))
    })
    .unwrap_or_default();
    with_music(|music| {
        music.set_album(row);
        music.set_album_line(line.into());
    });
}

fn show_tracks(playing_id: Option<&str>) {
    let in_album = with_state(|state| state.shown == Shown::Album).unwrap_or(false);
    let rows = with_state(|state| {
        let source_key = state.current.and_then(|index| state.sources.get(index)).map(Source::key).unwrap_or_default();
        state
            .tracks
            .iter()
            .enumerate()
            .map(|(index, track)| {
                let cover = if in_album { None } else { state.covers.get(&(source_key.clone(), track.cover_art.clone())).cloned().flatten() };
                TrackRow {
                    id: track.id.clone().into(),
                    number: if in_album { (index + 1).to_string() } else { String::new() }.into(),
                    title: track.title.clone().into(),
                    artist: track.artist.clone().into(),
                    album: track.album.clone().into(),
                    duration: clock(track.duration).into(),
                    starred: track.starred,
                    playing: playing_id == Some(track.id.as_str()),
                    initials: initials(&track.album).into(),
                    tint: tint(&track.album_id),
                    has_cover: cover.is_some(),
                    cover: cover.unwrap_or_default(),
                }
            })
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    with_state(|state| state.track_rows.set_vec(rows));
}

fn show_names() {
    let rows = with_state(|state| state.names.clone()).unwrap_or_default();
    with_music(|music| music.set_names(ModelRc::new(VecModel::from(rows))));
}

/// "KB" for "Kind of Blue": the first letters of the first two words.
fn initials(name: &str) -> String {
    let mut letters: String = name
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_' || c == '.' || c == ',' || c == ':' || c == '(' || c == '[')
        .filter(|word| !word.is_empty())
        .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()))
        .take(2)
        .flat_map(|c| c.to_uppercase())
        .collect();
    if letters.is_empty() {
        letters.push('♪');
    }
    letters
}

/// Which of the ten hues a name's tile takes.
fn tint(id: &str) -> i32 {
    let hash = id.bytes().fold(0u32, |hash, byte| hash.wrapping_mul(31).wrapping_add(u32::from(byte)));
    (hash % 10) as i32
}

/// "3:41", "1:02:15".
fn clock(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

fn clock_f(seconds: f64) -> String {
    clock(seconds.max(0.0).round() as u64)
}

// ---- covers -------------------------------------------------------------------------------------

/// The covers of what is shown and of the queue are asked for, the ones not known yet, a few
/// at a time.
fn ask_covers() {
    let wanted: Vec<(usize, String)> = with_state(|state| {
        let Some(current) = state.current else { return Vec::new() };
        let mut wanted = Vec::new();
        let mut seen = HashSet::new();
        let mut want = |source: usize, id: &str| {
            if !id.is_empty() && seen.insert((source, id.to_string())) {
                wanted.push((source, id.to_string()));
            }
        };
        if let Some(album) = &state.album {
            want(current, &album.cover_art);
        }
        for album in &state.albums {
            want(current, &album.cover_art);
        }
        if state.shown == Shown::Tracks {
            for track in &state.tracks {
                want(current, &track.cover_art);
            }
        }
        for track in &state.playback.queue {
            want(track.source, &track.cover_art);
        }
        wanted
    })
    .unwrap_or_default();
    with_state(|state| {
        for (source, id) in wanted {
            let Some(key) = state.sources.get(source).map(Source::key) else { continue };
            if state.covers.contains_key(&(key.clone(), id.clone())) || !state.covers_asked.insert((key, id.clone())) {
                continue;
            }
            state.cover_queue.push_back((source, id));
        }
    });
    ask_next_covers();
}

fn ask_next_covers() {
    loop {
        let next = with_state(|state| {
            if state.covers_in_flight >= COVERS_AT_ONCE {
                return None;
            }
            let (source, id) = state.cover_queue.pop_front()?;
            let source_ref = state.sources.get(source)?;
            let path = cover_path(&source_ref.key(), &id);
            state.covers_in_flight += 1;
            Some((source_ref.key(), id, path, source_ref.client(&state.agent)))
        })
        .flatten();
        let Some((key, id, path, client)) = next else { return };
        if path.exists() {
            decode_cover(key, id, path);
        } else {
            send_cover(CoverTask { client, id, path });
        }
    }
}

/// The cover's bytes came (or not): decoded off the UI thread into a picture.
fn cover_fetched(id: String, path: PathBuf, fetched: bool) {
    let key = path.parent().and_then(Path::parent).and_then(Path::file_name).map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    if fetched {
        decode_cover(key, id, path);
    } else {
        cover_ready(key, id, None);
    }
}

fn decode_cover(key: String, id: String, path: PathBuf) {
    images::load(path, move |picture| cover_ready(key, id, picture));
}

/// A cover is known, with or without a picture: every row that shows it is drawn again.
fn cover_ready(key: String, id: String, picture: Option<Image>) {
    with_state(|state| {
        state.covers_in_flight = state.covers_in_flight.saturating_sub(1);
        state.covers.insert((key.clone(), id.clone()), picture.clone());
    });
    if let Some(picture) = picture {
        with_state(|state| {
            let current_key = state.current.and_then(|index| state.sources.get(index)).map(Source::key).unwrap_or_default();
            if current_key == key {
                for row in 0..state.album_rows.row_count() {
                    if let Some(mut data) = state.album_rows.row_data(row)
                        && state.albums.iter().any(|album| album.id == data.id.as_str() && album.cover_art == id)
                    {
                        data.cover = picture.clone();
                        data.has_cover = true;
                        state.album_rows.set_row_data(row, data);
                    }
                }
                for row in 0..state.track_rows.row_count() {
                    if let Some(mut data) = state.track_rows.row_data(row)
                        && state.tracks.get(row).is_some_and(|track| track.cover_art == id)
                        && state.shown != Shown::Album
                    {
                        data.cover = picture.clone();
                        data.has_cover = true;
                        state.track_rows.set_row_data(row, data);
                    }
                }
            }
            for row in 0..state.queue_rows.row_count() {
                if let Some(mut data) = state.queue_rows.row_data(row)
                    && state.playback.queue.get(row).is_some_and(|track| track.cover_art == id && state.sources.get(track.source).is_some_and(|source| source.key() == key))
                {
                    data.cover = picture.clone();
                    data.has_cover = true;
                    state.queue_rows.set_row_data(row, data);
                }
            }
        });
        show_album();
        show_song();
    }
    ask_next_covers();
}

// ---- the cache --------------------------------------------------------------------------------

/// Where songs and covers are kept: ~/Documents/FinchGram/music_cache, a folder per source.
fn cache_folder() -> PathBuf {
    dirs::document_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from(".")).join("FinchGram").join("music_cache")
}

/// A name a file can have: the id's letters and digits, the rest replaced.
fn file_name_of(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

fn cover_path(source_key: &str, id: &str) -> PathBuf {
    cache_folder().join(source_key).join("covers").join(file_name_of(id))
}

/// The song's file once whole, and while on its way.
fn song_paths(source_key: &str, track: &Track) -> (PathBuf, PathBuf) {
    let folder = cache_folder().join(source_key);
    let suffix = if track.suffix.is_empty() { "bin".to_string() } else { file_name_of(&track.suffix) };
    let whole = folder.join(format!("{}.{suffix}", file_name_of(&track.id)));
    let part = folder.join(format!("{}.{suffix}.part", file_name_of(&track.id)));
    (whole, part)
}

/// The cache measured, on the worker, and trimmed to the setting.
fn measure_cache() {
    let limit = with_state(|state| state.settings.borrow().music.cache_limit_mb.saturating_mul(1 << 20)).unwrap_or(0);
    send(Task::Cache { folder: cache_folder(), limit });
}

/// Every song file under `folder`, oldest first, with its size; covers and parts are not counted.
fn cached_songs(folder: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let mut songs = Vec::new();
    let Ok(sources) = std::fs::read_dir(folder) else { return songs };
    for source in sources.flatten() {
        let Ok(files) = std::fs::read_dir(source.path()) else { continue };
        for file in files.flatten() {
            let Ok(kind) = file.file_type() else { continue };
            if !kind.is_file() {
                continue;
            }
            let path = file.path();
            if path.extension().is_some_and(|extension| extension == "part") {
                continue;
            }
            if let Ok(meta) = file.metadata() {
                songs.push((path, meta.len(), meta.modified().unwrap_or(std::time::UNIX_EPOCH)));
            }
        }
    }
    songs.sort_by_key(|(_, _, modified)| *modified);
    songs
}

/// The oldest songs deleted until the cache is within `limit` bytes (0 for no limit): what is
/// left, in bytes.
fn trim_cache(folder: &Path, limit: u64) -> u64 {
    let songs = cached_songs(folder);
    let mut total: u64 = songs.iter().map(|(_, size, _)| *size).sum();
    if limit > 0 {
        for (path, size, _) in &songs {
            if total <= limit {
                break;
            }
            if std::fs::remove_file(path).is_ok() {
                total -= size;
            }
        }
    }
    total
}

fn show_cache(bytes: u64) {
    let line = if bytes == 0 { None } else { with_state(|state| with_words(state, |words| words.invoke_cached(megabytes(bytes).into()).to_string())).flatten() };
    with_music(|music| music.set_cache_line(line.unwrap_or_default().into()));
}

fn megabytes(bytes: u64) -> String {
    if bytes >= 1 << 30 { format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64) } else { format!("{} MB", bytes >> 20) }
}

// ---- the form ---------------------------------------------------------------------------------

fn add_source() {
    with_state(|state| {
        state.editing = None;
        if state.shown != Shown::Form {
            state.before_form = Some(state.shown.clone());
        }
        state.shown = Shown::Form;
    });
    with_music(|music| {
        music.set_form_editing(false);
        music.set_form_name(SharedString::new());
        music.set_form_address(SharedString::new());
        music.set_form_port(SharedString::new());
        music.set_form_username(SharedString::new());
        music.set_form_password(SharedString::new());
        music.set_form_error(MusicFormError::None);
        music.set_test_state(SourceTest::None);
        music.set_test_line(SharedString::new());
    });
    show_view();
}

fn edit_source(index: i32) {
    let Ok(index) = usize::try_from(index) else { return };
    let source = with_state(|state| {
        let source = state.sources.get(index)?.clone();
        state.editing = Some(index);
        if state.shown != Shown::Form {
            state.before_form = Some(state.shown.clone());
        }
        state.shown = Shown::Form;
        Some(source)
    })
    .flatten();
    let Some(source) = source else { return };
    with_music(|music| {
        music.set_form_editing(true);
        music.set_form_name(source.name.into());
        music.set_form_address(source.address.into());
        music.set_form_port(if source.port == 0 { String::new() } else { source.port.to_string() }.into());
        music.set_form_username(source.username.into());
        music.set_form_password(source.password.into());
        music.set_form_error(MusicFormError::None);
        music.set_test_state(SourceTest::None);
        music.set_test_line(SharedString::new());
    });
    show_sources();
    show_view();
}

/// The form's fields, as typed.
struct Form {
    name: String,
    address: String,
    port: String,
    username: String,
    password: String,
}

fn form_fields() -> Option<Form> {
    let ui = STATE.with(|state| state.borrow().as_ref().and_then(|state| state.ui.upgrade()))?;
    let music = ui.global::<Music>();
    Some(Form {
        name: music.get_form_name().trim().to_string(),
        address: music.get_form_address().trim().to_string(),
        port: music.get_form_port().trim().to_string(),
        username: music.get_form_username().trim().to_string(),
        password: music.get_form_password().to_string(),
    })
}

/// The form as a source, checked: the address an http(s) one, a port written into it taken
/// out, the port a number; or what is wrong.
fn form_source(form: Form) -> Result<Source, MusicFormError> {
    if !subsonic::is_address(&form.address) {
        return Err(MusicFormError::Address);
    }
    let mut address = form.address.trim_end_matches('/').to_string();
    // A port written into the address: taken out, into the port.
    let mut inline = String::new();
    if let Some(scheme_end) = address.find("://") {
        let host_start = scheme_end + 3;
        let host_end = address[host_start..].find('/').map_or(address.len(), |slash| host_start + slash);
        let host = address[host_start..host_end].to_string();
        if !host.starts_with('[')
            && let Some((name, port)) = host.rsplit_once(':')
            && !port.is_empty()
            && port.chars().all(|c| c.is_ascii_digit())
        {
            inline = port.to_string();
            address = format!("{}{}{}", &address[..host_start], name, &address[host_end..]);
        }
    }
    let typed = if form.port.is_empty() { inline } else { form.port };
    let port = if typed.is_empty() {
        0
    } else {
        match typed.parse::<u32>() {
            Ok(port @ 1..=65535) => port as u16,
            _ => return Err(MusicFormError::Port),
        }
    };
    Ok(Source { name: form.name, kind: "subsonic".into(), address, port, username: form.username, password: form.password, status: SourceStatus::Unknown })
}

fn test_connection() {
    let Some(form) = form_fields() else { return };
    let source = match form_source(form) {
        Ok(source) => source,
        Err(error) => {
            with_music(|music| music.set_form_error(error));
            return;
        }
    };
    let (request, client) = with_state(|state| {
        state.test_request += 1;
        (state.test_request, source.client(&state.agent))
    })
    .unwrap_or((0, source.client(&subsonic::agent())));
    with_music(|music| {
        music.set_form_error(MusicFormError::None);
        music.set_test_state(SourceTest::Testing);
    });
    send(Task::Test { request, client });
}

fn tested(request: u64, result: Result<TestResult, subsonic::Error>) {
    let current = with_state(|state| state.test_request).unwrap_or(0);
    if request != current {
        return;
    }
    let (line, ok) = with_state(|state| match &result {
        Ok(TestResult { info, extensions }) => {
            let mut parts = Vec::new();
            if !info.kind.is_empty() {
                let mut kind = info.kind.clone();
                if let Some(first) = kind.get_mut(0..1) {
                    first.make_ascii_uppercase();
                }
                parts.push(if info.server_version.is_empty() { kind } else { format!("{kind} {}", info.server_version) });
            } else {
                parts.push(format!("Subsonic API {}", info.version));
            }
            if info.open_subsonic {
                parts.push(if *extensions > 0 { with_words(state, |words| words.invoke_extensions(*extensions as i32).to_string()).unwrap_or_default() } else { "OpenSubsonic".to_string() });
            }
            (parts.join(" · "), true)
        }
        Err(err) => (problem_words(state, err), false),
    })
    .unwrap_or_default();
    with_music(|music| {
        music.set_test_line(line.into());
        music.set_test_state(if ok { SourceTest::Ok } else { SourceTest::Failed });
    });
}

/// Add, or Save: the source goes into the database, and is looked at.
fn submit_form() {
    let Some(form) = form_fields() else { return };
    if form.name.is_empty() {
        with_music(|music| music.set_form_error(MusicFormError::Name));
        return;
    }
    let source = match form_source(form) {
        Ok(source) => source,
        Err(error) => {
            with_music(|music| music.set_form_error(error));
            return;
        }
    };
    let index = with_state(|state| {
        let index = match state.editing.filter(|index| *index < state.sources.len()) {
            Some(index) => {
                state.sources[index] = source;
                index
            }
            None => {
                state.sources.push(source);
                state.sources.len() - 1
            }
        };
        state.editing = None;
        state.before_form = None;
        save_sources(state);
        index
    });
    with_music(|music| music.set_form_password(SharedString::new()));
    if let Some(index) = index {
        with_state(|state| state.current = None);
        select_source(index as i32);
        let check = with_state(|state| state.sources.get(index).map(|source| Task::Check { index, client: source.client(&state.agent) })).flatten();
        if let Some(check) = check {
            with_state(|state| {
                if let Some(source) = state.sources.get_mut(index) {
                    source.status = SourceStatus::Checking;
                }
            });
            send(check);
        }
    }
}

fn cancel_form() {
    let (before, any, current) = with_state(|state| {
        state.editing = None;
        (state.before_form.take(), !state.sources.is_empty(), state.current)
    })
    .unwrap_or((None, false, None));
    with_music(|music| music.set_form_password(SharedString::new()));
    match (before, current, any) {
        (Some(before), Some(_), _) if before != Shown::Form && before != Shown::Nothing => {
            with_state(|state| state.shown = before);
            show_sources();
            show_view();
        }
        (_, None, true) => select_source(0),
        (_, Some(_), _) => load_section(),
        _ => {
            with_state(|state| state.shown = Shown::Nothing);
            show_sources();
            show_view();
        }
    }
}

fn remove_from_form() {
    let editing = with_state(|state| state.editing).flatten();
    if let Some(index) = editing {
        remove_source(index as i32);
    }
}

/// The source goes, with its password; the one before it is looked at instead.
fn remove_source(index: i32) {
    let Ok(index) = usize::try_from(index) else { return };
    let next = with_state(|state| {
        if index >= state.sources.len() {
            return None;
        }
        state.sources.remove(index);
        state.editing = None;
        state.before_form = None;
        save_sources(state);
        state.current = match state.current {
            Some(current) if current == index => None,
            Some(current) if current > index => Some(current - 1),
            other => other,
        };
        Some(if state.sources.is_empty() { None } else { Some(state.current.unwrap_or(index.min(state.sources.len() - 1)) as i32) })
    })
    .flatten();
    with_music(|music| music.set_form_password(SharedString::new()));
    match next {
        Some(Some(next)) => {
            with_state(|state| state.current = None);
            select_source(next);
        }
        Some(None) => {
            with_state(|state| {
                state.shown = Shown::Nothing;
                state.albums.clear();
                state.tracks.clear();
                state.names.clear();
            });
            show_sources();
            show_view();
        }
        None => {}
    }
}

fn save_sources(state: &State) {
    let records: Vec<database::MusicSourceRecord> = state.sources.iter().map(Source::record).collect();
    if let Err(err) = database::save_music_sources(&records) {
        eprintln!("music: cannot keep the sources in the database: {err}");
    }
}

// ---- the queue and what plays -------------------------------------------------------------------

/// An album's songs, fetched to play or queue them.
fn album_songs(id: &str, then: Then) {
    // The album open, when it is this one: its songs are here already.
    let here = with_state(|state| state.album.as_ref().filter(|album| album.id == id && !state.tracks.is_empty()).map(|album| (album.name.clone(), state.tracks.clone()))).flatten();
    if let Some((name, tracks)) = here {
        take_songs(name, tracks, then);
        return;
    }
    let Some((_, client)) = current_client() else { return };
    let request = with_state(|state| {
        state.test_request += 1;
        state.test_request
    })
    .unwrap_or(0);
    send(Task::Songs0f { request, client, id: id.to_string(), then });
}

fn album_songs_came(_request: u64, result: Result<subsonic::Album, subsonic::Error>, then: Then) {
    match result {
        Ok(album) => {
            let source = with_state(|state| state.current.unwrap_or(0)).unwrap_or(0);
            let tracks: Vec<Track> = album.song.iter().map(|song| Track::from_song(song, source, &album)).collect();
            take_songs(album.name, tracks, then);
        }
        Err(err) => eprintln!("music: cannot fetch the album's songs: {err}"),
    }
}

fn take_songs(from: String, tracks: Vec<Track>, then: Then) {
    if tracks.is_empty() {
        return;
    }
    match then {
        Then::Play => play_tracks(tracks, 0, from, Some(false)),
        Then::PlayShuffled => play_tracks(tracks, 0, from, Some(true)),
        Then::Queue => append_tracks(tracks),
    }
}

/// A song of the tracks shown: it plays, and the ones shown follow it; -1 shuffles them all.
fn play_track(index: i32) {
    let (tracks, from) = with_state(|state| {
        let from = match (&state.shown, &state.origin, &state.album) {
            (Shown::Album, _, Some(album)) => album.name.clone(),
            (_, Some(Origin::Playlist { name }), _) => name.clone(),
            _ => section_name(state, state.section),
        };
        (state.tracks.clone(), from)
    })
    .unwrap_or_default();
    if tracks.is_empty() {
        return;
    }
    if index < 0 {
        play_tracks(tracks, 0, from, Some(true));
    } else {
        let start = (index as usize).min(tracks.len() - 1);
        play_tracks(tracks, start, from, None);
    }
}

fn queue_track(index: i32) {
    let track = with_state(|state| usize::try_from(index).ok().and_then(|index| state.tracks.get(index).cloned())).flatten();
    if let Some(track) = track {
        append_tracks(vec![track]);
    }
}

/// `tracks` become the queue, `start` playing first; `shuffle` sets the order's shuffle anew
/// when given (an album's Shuffle), else the current choice holds.
fn play_tracks(tracks: Vec<Track>, start: usize, from: String, shuffle: Option<bool>) {
    with_state(|state| {
        if let Some(shuffle) = shuffle {
            state.playback.shuffle = shuffle;
            state.settings.borrow_mut().music.shuffle = shuffle;
            state.settings.borrow().save();
        }
        state.playback.queue = tracks;
        state.playback.from = from;
        state.playback.order = order_of(state.playback.queue.len(), state.playback.shuffle, Some(start));
        state.playback.at = Some(0);
    });
    with_music(|music| music.set_shuffle(with_state(|state| state.playback.shuffle).unwrap_or(false)));
    start_current();
    ask_covers();
}

/// Songs after the queue; they play when nothing does.
fn append_tracks(mut tracks: Vec<Track>) {
    for track in tracks.iter_mut() {
        track.added = true;
    }
    let playing = with_state(|state| {
        let first_new = state.playback.queue.len();
        let count = tracks.len();
        state.playback.queue.extend(tracks);
        // The new songs come after everything queued, in their own order.
        let mut added: Vec<usize> = (first_new..first_new + count).collect();
        if state.playback.shuffle {
            shuffle(&mut added);
        }
        state.playback.order.extend(added);
        if state.playback.from.is_empty() {
            state.playback.from = section_name(state, state.section);
        }
        state.playback.current.is_some()
    })
    .unwrap_or(false);
    if playing {
        // The next song may have changed.
        with_state(|state| state.playback.next = None);
        player::clear_next();
        show_queue();
        prepare_next();
    } else {
        with_state(|state| state.playback.at = Some(0));
        start_current();
    }
    ask_covers();
}

/// The order `count` songs play in: as they are, or shuffled with `first` kept first.
fn order_of(count: usize, shuffled: bool, first: Option<usize>) -> Vec<usize> {
    let mut order: Vec<usize> = (0..count).collect();
    if shuffled {
        shuffle(&mut order);
        if let Some(first) = first
            && let Some(at) = order.iter().position(|index| *index == first)
        {
            order.swap(0, at);
        }
    } else if let Some(first) = first {
        order.rotate_left(first.min(count.saturating_sub(1)));
        // Not rotated: the songs before `first` come after the last, as a list played from the
        // middle does.
    }
    order
}

/// Fisher–Yates with the system's randomness.
fn shuffle(items: &mut [usize]) {
    use ring::rand::SecureRandom as _;
    let random = ring::rand::SystemRandom::new();
    for i in (1..items.len()).rev() {
        let mut bytes = [0u8; 4];
        let _ = random.fill(&mut bytes);
        let j = (u32::from_le_bytes(bytes) as usize) % (i + 1);
        items.swap(i, j);
    }
}

/// The song at `order[at]` plays: fetched or from the cache, through mpv.
fn start_current() {
    let prepared = with_state(|state| {
        state.playback.next = None;
        state.playback.current = None;
        let at = state.playback.at?;
        let index = *state.playback.order.get(at)?;
        let track = state.playback.queue.get(index)?.clone();
        let source = state.sources.get(track.source)?;
        let playing = prepare(&state.agent, source, index, &track);
        state.playback.current = Some(playing);
        state.playback.scrobbled = false;
        state.playback.position = 0.0;
        state.playback.duration = track.duration as f64;
        state.playback.paused = false;
        Some((state.playback.current.as_ref().map(|playing| playing.url.clone()).unwrap_or_default(), state.playback.volume, track, source.client(&state.agent)))
    })
    .flatten();
    let Some((url, volume, track, client)) = prepared else {
        stop();
        return;
    };
    player::play(&url, volume);
    send(Task::Scrobble { client, id: track.id, submission: false });
    show_song();
    show_queue();
    show_tracks_playing();
    prepare_next();
}

/// Where mpv reads the song from: the cache file when it is whole, else a stream its fetch feeds.
fn prepare(agent: &ureq::Agent, source: &Source, index: usize, track: &Track) -> Playing {
    let (whole, part) = song_paths(&source.key(), track);
    if whole.is_file() {
        return Playing { at: index, url: whole.to_string_lossy().into_owned(), fetch: None };
    }
    let fetch = fetch::Fetch::start(agent.clone(), source.client(agent).stream_url(&track.id), part, whole, track.size, |_, _| {});
    Playing { at: index, url: fetch.url.clone(), fetch: Some(fetch) }
}

/// The index in `order` of the song after `at`, by the repeat mode; none at the end.
fn after(playback: &Playback, at: usize, by_hand: bool) -> Option<usize> {
    if playback.order.is_empty() {
        return None;
    }
    if playback.repeat == Repeat::One && !by_hand {
        return Some(at);
    }
    if at + 1 < playback.order.len() {
        Some(at + 1)
    } else if playback.repeat != Repeat::Off || by_hand {
        Some(0)
    } else {
        None
    }
}

/// The next song opened ahead by mpv, once the one playing is far enough or whole on the disk.
fn prepare_next() {
    let prepared = with_state(|state| {
        if state.playback.next.is_some() {
            return None;
        }
        let at = state.playback.at?;
        let current = state.playback.current.as_ref()?;
        let ready = match &current.fetch {
            None => true,
            Some(fetch) => fetch.is_done() || (state.playback.duration > 0.0 && state.playback.position / state.playback.duration >= PREFETCH_PART),
        };
        if !ready {
            return None;
        }
        let next_at = after(&state.playback, at, false)?;
        let index = *state.playback.order.get(next_at)?;
        let track = state.playback.queue.get(index)?.clone();
        let source = state.sources.get(track.source)?;
        let playing = prepare(&state.agent, source, index, &track);
        let url = playing.url.clone();
        state.playback.next = Some(playing);
        Some(url)
    })
    .flatten();
    if let Some(url) = prepared {
        player::queue_next(&url);
    }
}

/// What the music player reports.
fn heard(what: player::Heard) {
    match what {
        player::Heard::Position(seconds) => {
            with_state(|state| state.playback.position = seconds);
            with_music(|music| {
                music.set_position(seconds as f32);
                music.set_position_label(clock_f(seconds).into());
            });
            scrobble_if_listened();
            prepare_next();
        }
        player::Heard::Duration(seconds) => {
            if seconds > 0.0 {
                with_state(|state| state.playback.duration = seconds);
                with_music(|music| {
                    music.set_duration(seconds as f32);
                    music.set_duration_label(clock_f(seconds).into());
                });
            }
        }
        player::Heard::Paused(paused) => {
            with_state(|state| state.playback.paused = paused);
            with_music(|music| music.set_paused(paused));
        }
        player::Heard::Buffering(buffering) => with_music(|music| music.set_buffering(buffering)),
        player::Heard::Started(url) => started(&url),
        player::Heard::Idle => {
            // The last song ended (or a stop): nothing plays.
            with_state(|state| {
                state.playback.current = None;
                state.playback.next = None;
                state.playback.at = None;
            });
            show_song();
            show_queue();
            show_tracks_playing();
            measure_cache();
        }
        player::Heard::Failed(err) => {
            let fetch_failed = with_state(|state| state.playback.current.as_ref().and_then(|current| current.fetch.as_ref()).is_some_and(fetch::Fetch::is_failed)).unwrap_or(false);
            eprintln!("music: cannot play the song ({}): {err}", if fetch_failed { "it could not be fetched" } else { "mpv cannot play it" });
            // mpv goes on to the next song queued, or falls idle; either says so itself.
        }
    }
}

/// mpv plays the file at `url`: the next song took over from the one before (or the one asked
/// for started).
fn started(url: &str) {
    let took_over = with_state(|state| {
        if state.playback.current.as_ref().is_some_and(|current| current.url == url) {
            return None;
        }
        let next = state.playback.next.take()?;
        if next.url != url {
            state.playback.next = Some(next);
            return None;
        }
        let at = state.playback.at.and_then(|at| after(&state.playback, at, false));
        state.playback.at = at;
        let index = next.at;
        state.playback.current = Some(next);
        state.playback.scrobbled = false;
        state.playback.position = 0.0;
        let track = state.playback.queue.get(index)?.clone();
        state.playback.duration = track.duration as f64;
        let client = state.sources.get(track.source)?.client(&state.agent);
        Some((track.id, client))
    })
    .flatten();
    if let Some((id, client)) = took_over {
        send(Task::Scrobble { client, id, submission: false });
        with_music(|music| {
            music.set_position(0.0);
            music.set_position_label("0:00".into());
        });
        show_song();
        show_queue();
        show_tracks_playing();
        measure_cache();
        prepare_next();
    }
}

/// Past half the song, or four minutes: the server is told it was listened to, once.
fn scrobble_if_listened() {
    let scrobble = with_state(|state| {
        if state.playback.scrobbled || state.playback.duration <= 0.0 {
            return None;
        }
        let listened = state.playback.position >= (state.playback.duration * LISTENED_PART).min(LISTENED_SECONDS);
        if !listened {
            return None;
        }
        state.playback.scrobbled = true;
        let index = *state.playback.order.get(state.playback.at?)?;
        let track = state.playback.queue.get(index)?;
        Some((track.id.clone(), state.sources.get(track.source)?.client(&state.agent)))
    })
    .flatten();
    if let Some((id, client)) = scrobble {
        send(Task::Scrobble { client, id, submission: true });
    }
}

fn show_song() {
    let song = with_state(|state| {
        let at = state.playback.at?;
        let index = *state.playback.order.get(at)?;
        let track = state.playback.queue.get(index)?;
        let key = state.sources.get(track.source)?.key();
        let cover = state.covers.get(&(key, track.cover_art.clone())).cloned().flatten();
        Some((track.clone(), cover, state.playback.current.is_some()))
    })
    .flatten();
    with_music(|music| match song {
        Some((track, cover, playing)) => {
            music.set_song_initials(initials(&track.title).into());
            music.set_song_title(track.title.into());
            music.set_song_artist(track.artist.into());
            music.set_song_album(track.album.into());
            music.set_song_starred(track.starred);
            music.set_song_tint(tint(&track.album_id));
            music.set_song_has_cover(cover.is_some());
            music.set_song_cover(cover.unwrap_or_default());
            music.set_duration(track.duration as f32);
            music.set_duration_label(clock(track.duration).into());
            music.set_playing(playing);
        }
        None => {
            music.set_playing(false);
            music.set_buffering(false);
            music.set_position(0.0);
            music.set_position_label("0:00".into());
        }
    });
}

fn show_queue() {
    let (rows, count, line) = with_state(|state| {
        let current = state.playback.at.and_then(|at| state.playback.order.get(at).copied());
        let mut rows = Vec::with_capacity(state.playback.queue.len());
        let mut first_added = true;
        let mut first_own = true;
        let (up_next, added_by_you) = with_words(state, |words| (words.invoke_up_next().to_string(), words.invoke_added_by_you().to_string())).unwrap_or_default();
        for (index, track) in state.playback.queue.iter().enumerate() {
            let key = state.sources.get(track.source).map(Source::key).unwrap_or_default();
            let cover = state.covers.get(&(key, track.cover_art.clone())).cloned().flatten();
            let label = if track.added && first_added {
                first_added = false;
                added_by_you.as_str()
            } else if !track.added && first_own {
                first_own = false;
                up_next.as_str()
            } else {
                ""
            };
            rows.push(QueueRow {
                title: track.title.clone().into(),
                artist: track.artist.clone().into(),
                album: track.album.clone().into(),
                duration: clock(track.duration).into(),
                current: current == Some(index),
                added: track.added,
                label: label.into(),
                initials: initials(&track.album).into(),
                tint: tint(&track.album_id),
                has_cover: cover.is_some(),
                cover: cover.unwrap_or_default(),
            });
        }
        let seconds: u64 = state.playback.queue.iter().map(|track| track.duration).sum();
        let line = if state.playback.queue.is_empty() {
            String::new()
        } else if state.playback.from.is_empty() {
            format!("{} · {}", count_songs(state, state.playback.queue.len()), clock(seconds))
        } else {
            format!("{} · {} · {}", state.playback.from, count_songs(state, state.playback.queue.len()), clock(seconds))
        };
        (rows, state.playback.queue.len(), line)
    })
    .unwrap_or_default();
    with_state(|state| state.queue_rows.set_vec(rows));
    with_music(|music| {
        music.set_queue_count(count as i32);
        music.set_queue_line(line.into());
    });
}

/// The tracks shown, with the one playing marked.
fn show_tracks_playing() {
    let playing_id = with_state(|state| {
        let index = *state.playback.order.get(state.playback.at?)?;
        state.playback.queue.get(index).filter(|_| state.playback.current.is_some()).map(|track| track.id.clone())
    })
    .flatten();
    show_tracks(playing_id.as_deref());
}

fn toggle_pause() {
    let paused = with_state(|state| state.playback.current.is_some().then_some(state.playback.paused)).flatten();
    if let Some(paused) = paused {
        player::set_paused(!paused);
    }
}

fn next() {
    let moved = with_state(|state| {
        let at = state.playback.at?;
        let next_at = after(&state.playback, at, true)?;
        state.playback.at = Some(next_at);
        Some(())
    })
    .flatten();
    if moved.is_some() {
        start_current();
    } else {
        stop();
    }
}

/// Back to the song's start, or to the song before when it has just begun.
fn previous() {
    let (restart, moved) = with_state(|state| {
        if state.playback.position > 3.0 {
            return (true, false);
        }
        let Some(at) = state.playback.at else { return (false, false) };
        let before = if at > 0 { at - 1 } else if state.playback.repeat != Repeat::Off { state.playback.order.len().saturating_sub(1) } else { 0 };
        state.playback.at = Some(before);
        (false, true)
    })
    .unwrap_or((false, false));
    if restart {
        player::seek(0.0);
    } else if moved {
        start_current();
    }
}

fn seek(fraction: f32) {
    let seconds = with_state(|state| state.playback.duration * f64::from(fraction.clamp(0.0, 1.0))).unwrap_or(0.0);
    if seconds >= 0.0 {
        player::seek(seconds);
        with_music(|music| {
            music.set_position(seconds as f32);
            music.set_position_label(clock_f(seconds).into());
        });
    }
}

fn set_volume(volume: i32) {
    let volume = volume.clamp(0, 100);
    with_state(|state| state.playback.volume = volume);
    player::set_volume(volume);
    with_music(|music| music.set_volume(volume));
    // Written once the slider rests.
    with_state(|state| {
        state.volume_timer.start(slint::TimerMode::SingleShot, VOLUME_SETTLE, move || {
            with_state(|state| {
                let mut settings = state.settings.borrow_mut();
                settings.music.volume = volume as u32;
                settings.save();
            });
        });
    });
}

fn toggle_shuffle() {
    let shuffled = with_state(|state| {
        state.playback.shuffle = !state.playback.shuffle;
        // The song playing keeps playing; the rest take the new order.
        let current = state.playback.at.and_then(|at| state.playback.order.get(at).copied());
        state.playback.order = order_of(state.playback.queue.len(), state.playback.shuffle, current);
        state.playback.at = current.and_then(|index| state.playback.order.iter().position(|at| *at == index));
        state.playback.next = None;
        {
            let mut settings = state.settings.borrow_mut();
            settings.music.shuffle = state.playback.shuffle;
            settings.save();
        }
        state.playback.shuffle
    })
    .unwrap_or(false);
    with_music(|music| music.set_shuffle(shuffled));
    player::clear_next();
    show_queue();
    prepare_next();
}

fn cycle_repeat() {
    let repeat = with_state(|state| {
        state.playback.repeat = match state.playback.repeat {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        };
        state.playback.next = None;
        {
            let mut settings = state.settings.borrow_mut();
            settings.music.repeat = state.playback.repeat.setting().to_string();
            settings.save();
        }
        state.playback.repeat
    })
    .unwrap_or(Repeat::Off);
    with_music(|music| music.set_repeat(repeat.mode()));
    player::clear_next();
    prepare_next();
}

fn play_queued(index: i32) {
    let moved = with_state(|state| {
        let index = usize::try_from(index).ok()?;
        let at = state.playback.order.iter().position(|at| *at == index)?;
        state.playback.at = Some(at);
        Some(())
    })
    .flatten();
    if moved.is_some() {
        start_current();
    }
}

fn remove_queued(index: i32) {
    let Ok(index) = usize::try_from(index) else { return };
    let (was_current, left) = with_state(|state| {
        if index >= state.playback.queue.len() {
            return (false, true);
        }
        let current = state.playback.at.and_then(|at| state.playback.order.get(at).copied());
        state.playback.queue.remove(index);
        state.playback.order.retain(|at| *at != index);
        for at in state.playback.order.iter_mut() {
            if *at > index {
                *at -= 1;
            }
        }
        state.playback.next = None;
        let was_current = current == Some(index);
        if was_current {
            // The next song, which is now at the same place in the order.
            state.playback.at = state.playback.at.filter(|at| *at < state.playback.order.len()).or(if state.playback.order.is_empty() { None } else { Some(0) });
        } else if let Some(current) = current {
            state.playback.at = state.playback.order.iter().position(|at| *at == current);
        }
        (was_current, !state.playback.queue.is_empty())
    })
    .unwrap_or((false, true));
    if !left {
        clear_queue();
    } else if was_current {
        start_current();
    } else {
        player::clear_next();
        show_queue();
        prepare_next();
    }
}

fn clear_queue() {
    with_state(|state| {
        state.playback.queue.clear();
        state.playback.order.clear();
        state.playback.at = None;
        state.playback.from.clear();
    });
    stop();
}

/// Nothing plays: the player stops, the streams are over, the bar goes.
fn stop() {
    with_state(|state| {
        state.playback.current = None;
        state.playback.next = None;
        state.playback.at = None;
    });
    player::stop();
    crate::player::stream::withdraw_all_music();
    show_song();
    show_queue();
    show_tracks_playing();
}

fn toggle_queue() {
    let open = with_state(|state| {
        let mut settings = state.settings.borrow_mut();
        settings.music.queue_shown = !settings.music.queue_shown;
        settings.save();
        settings.music.queue_shown
    })
    .unwrap_or(true);
    with_music(|music| music.set_queue_open(open));
}

// ---- stars --------------------------------------------------------------------------------------

fn star_album(id: &str, on: bool) {
    let client = with_state(|state| {
        for album in state.albums.iter_mut().filter(|album| album.id == id) {
            album.starred = on.then(|| "now".to_string());
        }
        if let Some(album) = state.album.as_mut().filter(|album| album.id == id) {
            album.starred = on.then(|| "now".to_string());
        }
        state.current.and_then(|index| state.sources.get(index)).map(|source| source.client(&state.agent))
    })
    .flatten();
    show_albums();
    show_album();
    if let Some(client) = client {
        send(Task::Star { client, what: Starrable::Album(id.to_string()), on });
    }
}

fn star_track(index: i32, on: bool) {
    let starred = with_state(|state| {
        let track = state.tracks.get_mut(usize::try_from(index).ok()?)?;
        track.starred = on;
        let id = track.id.clone();
        for queued in state.playback.queue.iter_mut().filter(|queued| queued.id == id) {
            queued.starred = on;
        }
        Some((id, state.current.and_then(|index| state.sources.get(index)).map(|source| source.client(&state.agent))?))
    })
    .flatten();
    show_tracks_playing();
    show_song();
    if let Some((id, client)) = starred {
        send(Task::Star { client, what: Starrable::Song(id), on });
    }
}

/// The song playing, starred or not.
fn star_song(on: bool) {
    let starred = with_state(|state| {
        let index = *state.playback.order.get(state.playback.at?)?;
        let track = state.playback.queue.get_mut(index)?;
        track.starred = on;
        let id = track.id.clone();
        let source = track.source;
        for shown in state.tracks.iter_mut().filter(|shown| shown.id == id) {
            shown.starred = on;
        }
        Some((id, state.sources.get(source)?.client(&state.agent)))
    })
    .flatten();
    show_tracks_playing();
    show_song();
    if let Some((id, client)) = starred {
        send(Task::Star { client, what: Starrable::Song(id), on });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_tints_and_clocks_are_made_from_names() {
        assert_eq!(initials("Kind of Blue"), "KO");
        assert_eq!(initials("Rumours"), "R");
        assert_eq!(initials("The Köln Concert"), "TK");
        assert_eq!(initials("…"), "♪");
        assert_eq!(initials("netBloc Vol. 42: Live"), "NV");
        assert!((0..10).contains(&tint("6dGMsiesuYw9lti9yGIGWY")));
        assert_eq!(tint("x"), tint("x"));
        assert_eq!(clock(221), "3:41");
        assert_eq!(clock(3735), "1:02:15");
        assert_eq!(clock(0), "0:00");
        assert_eq!(megabytes(5 << 20), "5 MB");
        assert_eq!(megabytes(3 * (1 << 30) / 2), "1.5 GB");
    }

    #[test]
    fn the_order_keeps_the_first_song_first_and_repeat_decides_the_end() {
        assert_eq!(order_of(5, false, Some(2)), vec![2, 3, 4, 0, 1]);
        let shuffled = order_of(6, true, Some(4));
        assert_eq!(shuffled[0], 4);
        let mut sorted = shuffled.clone();
        sorted.sort();
        assert_eq!(sorted, vec![0, 1, 2, 3, 4, 5]);
        let mut playback = Playback { queue: Vec::new(), order: vec![0, 1, 2], at: Some(2), current: None, next: None, shuffle: false, repeat: Repeat::Off, volume: 100, paused: false, position: 0.0, duration: 0.0, scrobbled: false, from: String::new() };
        assert_eq!(after(&playback, 0, false), Some(1));
        assert_eq!(after(&playback, 2, false), None);
        assert_eq!(after(&playback, 2, true), Some(0));
        playback.repeat = Repeat::All;
        assert_eq!(after(&playback, 2, false), Some(0));
        playback.repeat = Repeat::One;
        assert_eq!(after(&playback, 1, false), Some(1));
        assert_eq!(after(&playback, 1, true), Some(2));
    }

    #[test]
    fn the_form_takes_a_port_out_of_the_address() {
        let form = |address: &str, port: &str| Form { name: "x".into(), address: address.into(), port: port.into(), username: "me".into(), password: String::new() };
        let source = form_source(form("https://nas.home:4533/music/", "")).expect("a source");
        assert_eq!((source.address.as_str(), source.port), ("https://nas.home/music", 4533));
        let source = form_source(form("http://10.0.0.5", "8080")).expect("a source");
        assert_eq!((source.address.as_str(), source.port), ("http://10.0.0.5", 8080));
        assert_eq!(form_source(form("nas.home", "")).err(), Some(MusicFormError::Address));
        assert_eq!(form_source(form("https://nas.home", "99999")).err(), Some(MusicFormError::Port));
        assert_eq!(file_name_of("al-6dGM/siesu:Yw9"), "al-6dGM_siesu_Yw9");
    }

    #[test]
    fn the_cache_is_trimmed_oldest_first() {
        let folder = std::env::temp_dir().join(format!("finchgram-music-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let source = folder.join("abcd");
        std::fs::create_dir_all(source.join("covers")).expect("folders");
        for (name, size, age) in [("old.mp3", 300u64, 30u64), ("new.flac", 500, 10), ("part.mp3.part", 900, 0)] {
            std::fs::write(source.join(name), vec![0u8; size as usize]).expect("a file");
            let when = std::time::SystemTime::now() - Duration::from_secs(age);
            let file = std::fs::File::options().write(true).open(source.join(name)).expect("the file");
            file.set_modified(when).expect("a date");
        }
        std::fs::write(source.join("covers").join("al-1"), vec![0u8; 100]).expect("a cover");
        assert_eq!(trim_cache(&folder, 0), 800);
        assert_eq!(trim_cache(&folder, 600), 500);
        assert!(!source.join("old.mp3").exists() && source.join("new.flac").exists() && source.join("part.mp3.part").exists());
        let _ = std::fs::remove_dir_all(&folder);
    }
}
