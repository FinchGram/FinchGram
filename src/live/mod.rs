//! The live directory in the Live view (the canvas "FinchGram Desktop 直播", rounds 12 and 13;
//! docs/live-streams.md, docs/live-directory.md): Discover, Following, Watch and Connections, over
//! channels from every source. The directory (services.finchgram.com) lists Owncast, PeerTube and
//! Icecast, streams submitted by their broadcasters, and reviewed Telegram channels; Telegram's
//! own live streams, the user's, come from TDLib (src/telegram/live.rs) and join Following as
//! followed on Telegram. The view needs the Telegram login, as the rest of the window does.
//!
//! Threads (docs/architecture.md): one worker asks the directory, a request at a time in the
//! order asked, and answers through the event loop; an answer to a question no longer asked is
//! dropped. A second worker fetches thumbnails, which are many and slow, into a cache folder that
//! images.rs decodes from. The follows are the app's database's (src/database.rs); the languages
//! and the content setting are settings.toml's `[live]`.

mod channel;
mod chat;
mod directory;
mod follows;
pub mod popout;
pub mod roots;
mod thumbnails;
pub mod watch;

pub use channel::hue_of;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::settings::Settings;
use crate::{LiveCategory, LiveChannel, LiveDirectory, LiveMode, LiveOption, LiveSection, LiveSort, LiveSource, LiveSourceRow, LiveStatus, LiveSuggestion, LiveSync, LiveToast, MainWindow};
use directory::{Query, Result_};

/// Results a page asks for: the cards' grid fills three columns, the list is longer.
const GRID_PAGE: u32 = 18;
const LIST_PAGE: u32 = 30;
/// Home's group "Plays in FinchGram" shows this many, with "See all".
const HOME_CARDS: u32 = 6;
/// What the view shows is asked again when it is older than this and the view is open.
const FRESH_FOR: Duration = Duration::from_secs(120);
/// The suggestions wait this long after the last keystroke (the canvas's debounce).
const SUGGEST_AFTER: Duration = Duration::from_millis(200);
/// Recent searches kept for the empty box's suggestions, for this run only.
const RECENT_KEPT: usize = 6;
/// The sources indexed, in the canvas's order; Twitch, YouTube, Kick and Bilibili are not yet.
const INDEXED: [LiveSource; 5] = [LiveSource::Owncast, LiveSource::Peertube, LiveSource::Icecast, LiveSource::Hls, LiveSource::Telegram];
const CATEGORIES: [LiveCategory; 11] = [
    LiveCategory::Music,
    LiveCategory::Gaming,
    LiveCategory::Talk,
    LiveCategory::Technology,
    LiveCategory::Creative,
    LiveCategory::Irl,
    LiveCategory::Education,
    LiveCategory::Events,
    LiveCategory::News,
    LiveCategory::Sports,
    LiveCategory::Other,
];

/// What the worker is asked.
enum Job {
    Languages(Vec<String>),
    Search { request: u64, query: Query, more: bool },
    /// The other modes' totals, for the mode tabs' counts.
    Count { request: u64, mode: LiveMode, query: Query },
    Suggest { request: u64, words: String },
    Sources,
    /// The followed channels' state, by id.
    Followed { request: u64, ids: Vec<String> },
    Lookup { request: u64, platform: Option<String>, handle: Option<String>, host: Option<String> },
}

/// Where a channel is at its source: what Watch resolves a stream from.
#[derive(Debug, Clone)]
pub(crate) struct Link {
    /// The channel's own address (never a playback one).
    pub url: String,
    pub source: LiveSource,
    /// Its id at the source: a host, "channel@host", a username.
    pub source_ref: String,
    pub handle: Option<String>,
}

impl Link {
    fn of(result: &Result_) -> Option<Link> {
        Some(Link {
            url: result.channel.url.clone(),
            source: channel::source_of(&result.channel.source)?,
            source_ref: result.channel.source_ref.clone(),
            handle: result.channel.handle.clone(),
        })
    }
}

/// Discover's question, as the controls set it.
#[derive(Clone, PartialEq)]
struct Ask {
    words: String,
    mode: LiveMode,
    sort: Option<LiveSort>,
    list: bool,
    sources: Vec<LiveSource>,
    category: LiveCategory,
    language: String,
    playback: i32,
    status: Option<LiveStatus>,
}

impl Default for Ask {
    fn default() -> Self {
        Ask {
            words: String::new(),
            mode: LiveMode::Live,
            sort: None,
            list: false,
            sources: Vec::new(),
            category: LiveCategory::None,
            language: String::new(),
            playback: 0,
            status: None,
        }
    }
}

impl Ask {
    fn filtered(&self) -> bool {
        !self.sources.is_empty() || self.category != LiveCategory::None || !self.language.is_empty() || self.playback != 0 || self.status.is_some()
    }

    /// Nothing typed, filtered or chosen: the home groups.
    fn home(&self) -> bool {
        self.words.trim().is_empty() && !self.filtered() && self.mode == LiveMode::Live && self.sort.is_none()
    }

    fn query(&self, mature: bool, mode: LiveMode, limit: u32) -> Query {
        Query {
            words: self.words.clone(),
            mode: match mode {
                LiveMode::Live => "live",
                LiveMode::Channels => "channels",
                LiveMode::Upcoming => "upcoming",
            },
            sources: self.sources.iter().map(|source| channel::source_id(*source)).collect(),
            category: channel::category_id(self.category),
            language: (!self.language.is_empty()).then(|| self.language.clone()),
            status: if mode == LiveMode::Channels {
                self.status.map(|status| match status {
                    LiveStatus::Live => "live",
                    LiveStatus::Stale => "stale",
                    LiveStatus::Offline => "offline",
                    LiveStatus::Unknown => "unknown",
                })
            } else {
                None
            },
            playback: match self.playback {
                1 => Some("native"),
                2 => Some("external"),
                _ => None,
            },
            mature,
            sort: self.sort.map(|sort| match sort {
                LiveSort::Recommended => "recommended",
                LiveSort::Relevance => "relevance",
                LiveSort::Viewers => "viewers",
                LiveSort::Recent => "recent",
                LiveSort::Added => "added",
            }),
            limit,
            cursor: None,
        }
    }
}

struct State {
    settings: Rc<RefCell<Settings>>,
    ask: Ask,
    request: u64,
    suggest_request: u64,
    count_request: u64,
    follow_request: u64,
    cursor: Option<String>,
    loaded_at: Option<Instant>,
    results: Rc<VecModel<LiveChannel>>,
    home_here: Rc<VecModel<LiveChannel>>,
    following_live: Rc<VecModel<LiveChannel>>,
    following_off: Rc<VecModel<LiveChannel>>,
    /// The thumbnails' addresses, by channel id, for the cards shown.
    thumbnail_urls: HashMap<String, String>,
    sources: Vec<directory::Source>,
    recent: VecDeque<String>,
    suggest_timer: slint::Timer,
    refresh_timer: slint::Timer,
    shown: bool,
    /// Followed here: the database's.
    follows: Vec<follows::Follow>,
    /// The followed channels as the directory last told them.
    followed: HashMap<String, LiveChannel>,
    /// Where each channel shown is, by id.
    links: HashMap<String, Link>,
    /// Live now by source, from the last search of what is on (the sources panel's counts).
    live_by_source: HashMap<String, i32>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static UI: RefCell<Option<slint::Weak<MainWindow>>> = const { RefCell::new(None) };
    static WORKER: RefCell<Option<Sender<Job>>> = const { RefCell::new(None) };
}

fn with_state<R>(change: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|state| state.borrow_mut().as_mut().map(change))
}

fn with_directory_get<R: Default>(read: impl FnOnce(&LiveDirectory<'_>) -> R) -> R {
    let mut value = R::default();
    with_directory(|directory| value = read(&directory));
    value
}

fn with_directory(change: impl FnOnce(LiveDirectory<'_>)) {
    UI.with(|ui| {
        if let Some(ui) = ui.borrow().as_ref().and_then(slint::Weak::upgrade) {
            change(ui.global::<LiveDirectory>());
        }
    });
}

fn ask(job: Job) {
    WORKER.with(|worker| {
        if let Some(worker) = worker.borrow().as_ref() {
            let _ = worker.send(job);
        }
    });
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn connect(ui: &MainWindow, settings: Rc<RefCell<Settings>>) {
    UI.with(|kept| *kept.borrow_mut() = Some(ui.as_weak()));
    let (worker, jobs) = mpsc::channel::<Job>();
    std::thread::Builder::new()
        .name("live directory".into())
        .spawn(move || {
            let mut client = directory::Client::new();
            for job in jobs {
                answer(&mut client, job);
            }
        })
        .expect("a thread for the live directory");
    WORKER.with(|kept| *kept.borrow_mut() = Some(worker));
    thumbnails::start();

    let languages = settings.borrow().live.languages.clone();
    let show_mature = settings.borrow().live.show_mature;
    ask(Job::Languages(languages));
    let state = State {
        settings,
        ask: Ask::default(),
        request: 0,
        suggest_request: 0,
        count_request: 0,
        follow_request: 0,
        cursor: None,
        loaded_at: None,
        results: Rc::new(VecModel::default()),
        home_here: Rc::new(VecModel::default()),
        following_live: Rc::new(VecModel::default()),
        following_off: Rc::new(VecModel::default()),
        thumbnail_urls: HashMap::new(),
        sources: Vec::new(),
        recent: VecDeque::new(),
        suggest_timer: slint::Timer::default(),
        refresh_timer: slint::Timer::default(),
        shown: false,
        follows: follows::all(),
        followed: HashMap::new(),
        links: HashMap::new(),
        live_by_source: HashMap::new(),
    };
    let directory = ui.global::<LiveDirectory>();
    directory.set_results(ModelRc::from(state.results.clone()));
    directory.set_home_here(ModelRc::from(state.home_here.clone()));
    directory.set_following_live(ModelRc::from(state.following_live.clone()));
    directory.set_following_off(ModelRc::from(state.following_off.clone()));
    directory.set_show_mature(show_mature);
    directory.set_mode_counts(ModelRc::new(VecModel::from(vec![-1, -1, -1])));
    directory.set_playback_counts(ModelRc::new(VecModel::from(vec![-1, -1, -1])));
    STATE.with(|kept| *kept.borrow_mut() = Some(state));
    set_options();

    directory.on_go(go);
    directory.on_back(|| {
        watch::leave();
        with_directory(|directory| {
            directory.set_watching(false);
            directory.set_column_open(true);
        });
    });
    directory.on_search(|words| search(words.to_string()));
    directory.on_typed(|words| typed(words.to_string()));
    directory.on_choose_suggestion(choose_suggestion);
    directory.on_set_mode(|mode| {
        change(|ask| {
            ask.mode = mode;
            ask.status = None;
        })
    });
    directory.on_set_sort(|sort| change(|ask| ask.sort = Some(sort)));
    directory.on_set_list_layout(|list| {
        change(|ask| ask.list = list);
    });
    directory.on_toggle_source(|source| {
        change(|ask| {
            if let Some(at) = ask.sources.iter().position(|chosen| *chosen == source) {
                ask.sources.remove(at);
            } else {
                ask.sources.push(source);
            }
        })
    });
    directory.on_set_category(|category| change(|ask| ask.category = category));
    directory.on_set_language(|language| change(|ask| ask.language = language.to_string()));
    directory.on_set_playback(|playback| change(|ask| ask.playback = playback));
    directory.on_set_status(|any, status| change(|ask| ask.status = (!any).then_some(status)));
    directory.on_clear_filters(|| {
        change(|ask| {
            ask.sources.clear();
            ask.category = LiveCategory::None;
            ask.language.clear();
            ask.playback = 0;
            ask.status = None;
        })
    });
    directory.on_more(more);
    directory.on_retry(|| {
        // Only the next page failed: that page again; else everything.
        let page_failed = with_directory_get(|directory| directory.get_page_failed());
        with_directory(|directory| directory.set_page_failed(false));
        if page_failed {
            run(true);
        } else {
            run(false);
            ask(Job::Sources);
        }
    });
    directory.on_toggle_sources(|| with_directory(|directory| directory.set_sources_open(!directory.get_sources_open())));
    directory.on_open(|id| watch::open(&id));
    directory.on_open_page(|id| watch::open_page(&id));
    directory.on_follow(|id, on| follow(&id, on));
    directory.on_set_show_mature(set_show_mature);
    directory.on_toggle_preferred_language(|tag| toggle_language(&tag));
    show_languages();
    directory.on_type_link(|text| watch::type_link(&text));
    directory.on_send_chat(|text| chat::send(&text));
    directory.on_rename_chat(|name| chat::rename(&name));
    directory.on_open_link(watch::open_link);
    watch::connect(ui);
}

/// The Live view opened or closed (src/telegram/live.rs): what it shows is asked for again when
/// it is old, and every two minutes while it stays open.
pub fn shown(shown: bool) {
    let stale = with_state(|state| {
        state.shown = shown;
        state.loaded_at.is_none_or(|at| at.elapsed() > FRESH_FOR)
    })
    .unwrap_or(false);
    if !shown {
        with_state(|state| state.refresh_timer.stop());
        // A stream of the directory goes on in the mini player (Telegram's stop with the view).
        watch::minimise();
        return;
    }
    if stale {
        run(false);
        ask(Job::Sources);
    }
    refresh_followed();
    with_state(|state| {
        state.refresh_timer.start(slint::TimerMode::Repeated, FRESH_FOR, || {
            let open = with_state(|state| state.shown).unwrap_or(false);
            if open {
                run(false);
                ask(Job::Sources);
                refresh_followed();
            }
        });
    });
}

fn go(section: LiveSection) {
    with_directory(|directory| {
        directory.set_section(section);
        directory.set_column_open(true);
        if directory.get_watching() {
            // The stream goes on in the mini player.
            directory.set_watching(false);
            watch::minimise();
        }
    });
    if section == LiveSection::Following {
        refresh_followed();
    }
}

/// A control changed the question: ask again from the first page.
fn change(edit: impl FnOnce(&mut Ask)) {
    let changed = with_state(|state| {
        let before = state.ask.clone();
        edit(&mut state.ask);
        state.ask != before
    })
    .unwrap_or(false);
    if changed {
        set_options();
        run(false);
    }
}

fn search(words: String) {
    let words = words.trim().to_string();
    with_state(|state| {
        if !words.is_empty() && !is_link(&words) {
            state.recent.retain(|recent| recent != &words);
            state.recent.push_front(words.clone());
            state.recent.truncate(RECENT_KEPT);
        }
    });
    with_directory(|directory| {
        directory.set_suggestions_open(false);
        directory.set_query(words.clone().into());
        directory.set_query_is_link(is_link(&words));
    });
    if is_link(&words) {
        // A link is no search: Connections › Open a link takes it.
        with_directory(|directory| directory.set_section(LiveSection::Connections));
        watch::type_link(&words);
        return;
    }
    change(|ask| {
        // Relevance with words, Recommended without, unless the user chose an order.
        if ask.sort == Some(LiveSort::Relevance) || ask.sort == Some(LiveSort::Recommended) {
            ask.sort = None;
        }
        ask.words = words;
    });
}

/// Whether the box holds a link rather than words (the canvas's test).
fn is_link(text: &str) -> bool {
    let text = text.trim();
    if text.contains(char::is_whitespace) {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    ["http://", "https://", "rtmp://", "rtmps://", "srt://", "rtsp://"].iter().any(|scheme| lower.starts_with(scheme))
        || (lower.contains('.') && lower.split('/').next().is_some_and(|host| {
            let labels: Vec<&str> = host.split('.').collect();
            labels.len() >= 2
                && labels.iter().all(|label| !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
                && labels.last().is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()))
        }))
}

/// Ask the directory: the first page (`more` false) or the next.
fn run(more: bool) {
    let job = with_state(|state| {
        let mature = state.settings.borrow().live.show_mature;
        state.request += 1;
        let home = state.ask.home();
        let limit = if home {
            HOME_CARDS
        } else if state.ask.list || state.ask.mode != LiveMode::Live {
            LIST_PAGE
        } else {
            GRID_PAGE
        };
        let mut query = state.ask.query(mature, state.ask.mode, limit);
        if home {
            query.playback = Some("native");
        }
        if more {
            query.cursor = state.cursor.clone();
        } else {
            state.cursor = None;
        }
        (state.request, query, home)
    });
    let Some((request, query, home)) = job else { return };
    with_directory(|directory| {
        directory.set_home(home);
        if more {
            directory.set_more_loading(true);
        } else {
            directory.set_loading(true);
            directory.set_failed(false);
        }
    });
    ask(Job::Search { request, query, more });
    if !more {
        count_modes();
    }
}

fn more() {
    let has = with_state(|state| state.cursor.is_some()).unwrap_or(false);
    if has {
        run(true);
    }
}

/// The other modes' totals, for the tabs: the same question, one result each.
fn count_modes() {
    let jobs = with_state(|state| {
        state.count_request += 1;
        let mature = state.settings.borrow().live.show_mature;
        let request = state.count_request;
        [LiveMode::Live, LiveMode::Channels, LiveMode::Upcoming]
            .into_iter()
            .filter(|mode| *mode != state.ask.mode)
            .map(|mode| Job::Count { request, mode, query: state.ask.query(mature, mode, 1) })
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    for job in jobs {
        ask(job);
    }
}

fn typed(words: String) {
    with_directory(|directory| {
        directory.set_query(words.clone().into());
        directory.set_query_is_link(is_link(&words));
        directory.set_suggestions_open(true);
    });
    let words = words.trim().to_string();
    if words.is_empty() || is_link(&words) {
        show_suggestions(&words, None);
        return;
    }
    with_state(|state| {
        state.suggest_request += 1;
        let request = state.suggest_request;
        state.suggest_timer.start(slint::TimerMode::SingleShot, SUGGEST_AFTER, move || {
            ask(Job::Suggest { request, words: words.clone() });
        });
    });
}

/// The suggestions for what the box holds: recent searches for nothing, "open it" for a link, the
/// directory's answer for words.
fn show_suggestions(words: &str, answer: Option<directory::Suggestions>) {
    let mut rows = Vec::new();
    if words.is_empty() {
        let recent = with_state(|state| state.recent.iter().cloned().collect::<Vec<_>>()).unwrap_or_default();
        rows.extend(recent.into_iter().map(|text| suggestion(0, text, LiveSource::Owncast, LiveCategory::None, "")));
    } else if is_link(words) {
        rows.push(suggestion(5, words.to_string(), LiveSource::Owncast, LiveCategory::None, ""));
    } else if let Some(answer) = answer {
        rows.extend(answer.queries.into_iter().map(|text| suggestion(1, text, LiveSource::Owncast, LiveCategory::None, "")));
        rows.extend(
            answer
                .channels
                .into_iter()
                .filter_map(|found| Some(suggestion(2, found.name, channel::source_of(&found.source)?, LiveCategory::None, &found.id))),
        );
        for filter in answer.filters {
            match filter.kind.as_str() {
                "source" => {
                    if let Some(source) = channel::source_of(&filter.value) {
                        rows.push(suggestion(3, filter.value, source, LiveCategory::None, ""));
                    }
                }
                "category" => {
                    let category = channel::category_of(Some(&filter.value));
                    if category != LiveCategory::None {
                        rows.push(suggestion(4, filter.value, LiveSource::Owncast, category, ""));
                    }
                }
                _ => {}
            }
        }
    }
    with_directory(|directory| directory.set_suggestions(ModelRc::new(VecModel::from(rows))));
}

fn suggestion(kind: i32, text: String, source: LiveSource, category: LiveCategory, id: &str) -> LiveSuggestion {
    LiveSuggestion { kind, text: text.into(), source, category, id: id.into() }
}

fn choose_suggestion(index: i32) {
    let mut chosen = None;
    with_directory(|directory| {
        chosen = usize::try_from(index).ok().and_then(|index| directory.get_suggestions().row_data(index));
        directory.set_suggestions_open(false);
    });
    let Some(chosen) = chosen else { return };
    match chosen.kind {
        0 | 1 => search(chosen.text.to_string()),
        2 => watch::open(&chosen.id),
        3 => {
            with_directory(|directory| directory.set_query(SharedString::new()));
            change(|ask| {
                ask.words.clear();
                if !ask.sources.contains(&chosen.source) {
                    ask.sources.push(chosen.source);
                }
            });
        }
        4 => {
            with_directory(|directory| directory.set_query(SharedString::new()));
            change(|ask| {
                ask.words.clear();
                ask.category = chosen.category;
            });
        }
        _ => search(chosen.text.to_string()),
    }
}

/// The filters' menus with their counts, the chosen marked; the counts come with each answer.
fn set_options() {
    set_options_counted(&directory::Facets::default(), false);
}

fn set_options_counted(facets: &directory::Facets, counted: bool) {
    let Some(ask) = with_state(|state| state.ask.clone()) else { return };
    let count = |map: &HashMap<String, i64>, key: &str| if counted { map.get(key).copied().unwrap_or(0) as i32 } else { -1 };
    let indexed = with_state(|state| {
        let listed: HashSet<&str> = state.sources.iter().filter(|source| source.indexing == "enabled").map(|source| source.id.as_str()).collect();
        INDEXED.iter().copied().filter(|source| listed.is_empty() || listed.contains(channel::source_id(*source))).collect::<Vec<_>>()
    })
    .unwrap_or_default();
    let sources: Vec<LiveOption> = indexed
        .into_iter()
        .map(|source| LiveOption {
            key: channel::source_id(source).into(),
            label: SharedString::new(),
            source,
            category: LiveCategory::None,
            count: count(&facets.source, channel::source_id(source)),
            chosen: ask.sources.contains(&source),
        })
        .collect();
    let categories: Vec<LiveOption> = CATEGORIES
        .iter()
        .map(|category| {
            let id = channel::category_id(*category).unwrap_or("");
            LiveOption { key: id.into(), label: SharedString::new(), source: LiveSource::Owncast, category: *category, count: count(&facets.category, id), chosen: ask.category == *category }
        })
        .collect();
    let mut languages: Vec<(String, i64)> = facets.language.iter().map(|(code, count)| (code.clone(), *count)).collect();
    if !ask.language.is_empty() && !languages.iter().any(|(code, _)| *code == ask.language) {
        languages.push((ask.language.clone(), 0));
    }
    languages.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let languages: Vec<LiveOption> = languages
        .into_iter()
        .map(|(code, n)| LiveOption {
            label: channel::language_name(&code).into(),
            chosen: ask.language == code,
            key: code.into(),
            source: LiveSource::Owncast,
            category: LiveCategory::None,
            count: if counted { n as i32 } else { -1 },
        })
        .collect();
    with_directory(|directory| {
        directory.set_source_options(ModelRc::new(VecModel::from(sources)));
        directory.set_category_options(ModelRc::new(VecModel::from(categories)));
        directory.set_language_options(ModelRc::new(VecModel::from(languages)));
        directory.set_mode(ask.mode);
        directory.set_sort(ask.sort.unwrap_or(if ask.words.trim().is_empty() { LiveSort::Recommended } else { LiveSort::Relevance }));
        directory.set_list_layout(ask.list);
        directory.set_playback(ask.playback);
        directory.set_chosen_category(ask.category);
        directory.set_chosen_language(ask.language.clone().into());
        directory.set_language_chosen(if ask.language.is_empty() { SharedString::new() } else { channel::language_name(&ask.language).into() });
        directory.set_status_any(ask.status.is_none());
        directory.set_status_filter(ask.status.unwrap_or(LiveStatus::Live));
        directory.set_filtered(ask.filtered());
        directory.set_home(ask.home());
    });
}

/// The worker's side: one job, its answer sent back to the UI thread.
fn answer(client: &mut directory::Client, job: Job) {
    match job {
        Job::Languages(languages) => client.set_languages(&languages),
        Job::Search { request, query, more } => {
            let result = client.search(&query);
            let _ = slint::invoke_from_event_loop(move || searched(request, more, result));
        }
        Job::Count { request, mode, query } => {
            let total = client.search(&query).ok().map(|page| (page.total.value.unwrap_or(-1), page.total.relation));
            let _ = slint::invoke_from_event_loop(move || counted(request, mode, total));
        }
        Job::Suggest { request, words } => {
            let result = client.suggest(&words);
            let _ = slint::invoke_from_event_loop(move || suggested(request, &words, result));
        }
        Job::Sources => {
            if let Ok(sources) = client.sources() {
                let _ = slint::invoke_from_event_loop(move || sources_came(sources));
            }
        }
        Job::Followed { request, ids } => {
            let mut found = Vec::new();
            for id in ids {
                match client.channel(&id) {
                    Ok(Some(result)) => found.push((id, Some(result))),
                    Ok(None) => found.push((id, None)),
                    Err(_) => {}
                }
            }
            let _ = slint::invoke_from_event_loop(move || followed_came(request, found));
        }
        Job::Lookup { request, platform, handle, host } => {
            let result = client.lookup(platform.as_deref(), handle.as_deref(), host.as_deref());
            let _ = slint::invoke_from_event_loop(move || watch::looked_up(request, result));
        }
    }
}

fn searched(request: u64, more: bool, result: Result<directory::SearchPage, String>) {
    let current = with_state(|state| state.request == request).unwrap_or(false);
    if !current {
        return;
    }
    with_state(|state| state.loaded_at = Some(Instant::now()));
    let page = match result {
        Ok(page) => page,
        Err(err) => {
            eprintln!("live directory: {err}");
            with_directory(|directory| {
                directory.set_loading(false);
                directory.set_more_loading(false);
                if more {
                    directory.set_page_failed(true);
                } else {
                    directory.set_failed(true);
                }
            });
            return;
        }
    };
    let now = now();
    let (follow_ids, home) = with_state(|state| (state.follows.iter().map(|follow| follow.id.clone()).collect::<HashSet<_>>(), state.ask.home())).unwrap_or_default();
    let mut cards = Vec::new();
    let mut thumbs = Vec::new();
    for item in &page.items {
        let Some(mut card) = channel::from_result(item, now) else { continue };
        if let Some(link) = Link::of(item) {
            with_state(|state| state.links.insert(card.id.to_string(), link));
        }
        card.following = follow_ids.contains(card.id.as_str());
        if let Some(url) = channel::thumbnail_of(item) {
            thumbs.push((card.id.to_string(), url));
        }
        cards.push(card);
    }
    let total = page.total.value.unwrap_or(-1) as i32;
    let relation = match page.total.relation.as_str() {
        "eq" => 0,
        "gte" => 1,
        _ => 2,
    };
    with_state(|state| {
        state.cursor = page.next_cursor.clone();
        let model = if home { state.home_here.clone() } else { state.results.clone() };
        if !more {
            model.set_vec(Vec::new());
            state.thumbnail_urls.clear();
        }
        for card in &cards {
            model.push(card.clone());
        }
        for (id, url) in &thumbs {
            state.thumbnail_urls.insert(id.clone(), url.clone());
        }
    });
    let mode = with_state(|state| state.ask.mode).unwrap_or(LiveMode::Live);
    with_directory(|directory| {
        directory.set_loading(false);
        directory.set_more_loading(false);
        directory.set_failed(false);
        directory.set_has_more(page.next_cursor.is_some());
        directory.set_partial(page.partial);
        directory.set_total(total);
        directory.set_total_relation(relation);
        if home {
            directory.set_home_here_count(total);
        }
        let counts = directory.get_mode_counts();
        let index = mode as usize;
        if index < counts.row_count() {
            counts.set_row_data(index, total);
        }
        directory.set_unknown_live(0);
        directory.set_hidden_mature(0);
        // Every source the directory indexes now plays in FinchGram; those that open elsewhere are
        // not integrated yet, and none of theirs is in the results.
        if !more {
            directory.set_playback_counts(ModelRc::new(VecModel::from(vec![total, total, 0])));
        }
    });
    if mode == LiveMode::Live && !more && !home {
        let live: HashMap<String, i32> = page.facets.source.iter().map(|(source, count)| (source.clone(), *count as i32)).collect();
        with_state(|state| state.live_by_source = live);
        show_sources();
    }
    if !more {
        set_options_counted(&page.facets, true);
    }
    for (id, url) in thumbs {
        thumbnails::fetch(&id, &url);
    }
    check_telegram(&cards);
}

/// The directory's Telegram channels among `cards` come as status unknown: Telegram is asked
/// (src/telegram/live_check.rs), the first few on screen, and their cards follow its answers.
fn check_telegram(cards: &[LiveChannel]) {
    const ASKED_AT_ONCE: usize = 6;
    let wanted: Vec<(String, String)> = cards
        .iter()
        .filter(|card| card.source == LiveSource::Telegram && card.status == LiveStatus::Unknown)
        .filter_map(|card| {
            let link = link_of(&card.id)?;
            let name = link.handle.clone().filter(|name| !name.is_empty()).unwrap_or(link.source_ref.clone());
            (!name.is_empty()).then(|| (card.id.to_string(), name.to_lowercase()))
        })
        .take(ASKED_AT_ONCE)
        .collect();
    if wanted.is_empty() {
        return;
    }
    let names: Vec<String> = wanted.iter().map(|(_, name)| name.clone()).collect();
    crate::telegram::check_live_usernames(names, move |name, seen| {
        for (id, _) in wanted.iter().filter(|(_, wanted)| wanted == name) {
            mark_cards(id, |card| {
                let (status, play, why) = match seen {
                    crate::telegram::LiveSeen::Live { stream: true } => (LiveStatus::Live, crate::LivePlay::Watch, crate::LiveWhy::None),
                    crate::telegram::LiveSeen::Live { stream: false } => (LiveStatus::Live, crate::LivePlay::Unavailable, crate::LiveWhy::Webrtc),
                    crate::telegram::LiveSeen::NotLive => (LiveStatus::Offline, crate::LivePlay::Unavailable, crate::LiveWhy::Offline),
                    crate::telegram::LiveSeen::Missing => (LiveStatus::Unknown, card.play, card.why),
                };
                card.status = status;
                card.play = play;
                card.why = why;
                card.observed_minutes = 0;
            });
        }
    });
}

fn counted(request: u64, mode: LiveMode, total: Option<(i64, String)>) {
    let current = with_state(|state| state.count_request == request).unwrap_or(false);
    if !current {
        return;
    }
    with_directory(|directory| {
        let counts = directory.get_mode_counts();
        let index = mode as usize;
        if index < counts.row_count() {
            counts.set_row_data(index, total.map_or(-1, |(value, _)| value as i32));
        }
    });
}

fn suggested(request: u64, words: &str, result: Result<directory::Suggestions, String>) {
    let current = with_state(|state| state.suggest_request == request).unwrap_or(false);
    if current {
        show_suggestions(words, result.ok());
    }
}

fn sources_came(sources: Vec<directory::Source>) {
    with_state(|state| state.sources = sources);
    show_sources();
    set_options();
}

/// The sources panel, from the sources' state and the live counts of the last search.
fn show_sources() {
    let now = now();
    let Some((sources, live_counts)) = with_state(|state| (state.sources.clone(), state.live_by_source.clone())) else { return };
    let rows: Vec<LiveSourceRow> = sources
        .iter()
        .filter_map(|source| {
            let id = channel::source_of(&source.id)?;
            Some(LiveSourceRow {
                source: id,
                indexed: source.indexing == "enabled",
                sync: match source.status.as_str() {
                    "ok" => LiveSync::Ok,
                    "degraded" => LiveSync::Degraded,
                    "rate_limited" => LiveSync::RateLimited,
                    "failed" => LiveSync::Failed,
                    _ => LiveSync::None,
                },
                live: live_counts.get(&source.id).copied().unwrap_or(-1),
                synced_minutes: directory::minutes_since(source.last_successful_sync_at.as_deref(), now).map_or(-1, |minutes| minutes as i32),
                viewer_counts: match source.capabilities.viewer_counts.as_str() {
                    "exact" => 1,
                    "self_reported" => 2,
                    "listeners" => 3,
                    _ => 0,
                },
                thumbnails: source.capabilities.thumbnails.as_bool().unwrap_or(source.capabilities.thumbnails.is_string()),
                plays_here: source.capabilities.playback.iter().any(|method| matches!(method.as_str(), "hls" | "dash" | "icecast" | "telegram")),
            })
        })
        .collect();
    with_directory(|directory| directory.set_sources(ModelRc::new(VecModel::from(rows))));
}

/// A thumbnail came (thumbnails.rs): the card that shows it gets it.
fn thumbnail_came(id: &str, image: Option<slint::Image>) {
    let Some(image) = image else {
        mark_cards(id, |card| card.has_thumbnail = false);
        return;
    };
    mark_cards(id, |card| card.thumbnail = image.clone());
}

/// Every card of a channel, in every list, changed alike.
fn mark_cards(id: &str, mut edit: impl FnMut(&mut LiveChannel)) {
    with_state(|state| {
        for model in [&state.results, &state.home_here, &state.following_live, &state.following_off] {
            for row in 0..model.row_count() {
                if let Some(mut card) = model.row_data(row)
                    && card.id == id
                {
                    edit(&mut card);
                    model.set_row_data(row, card);
                }
            }
        }
    });
}

fn follow(id: &str, on: bool) {
    // A channel followed on Telegram is unfollowed there.
    let card = find_card(id);
    if card.as_ref().is_some_and(|card| card.on_telegram) {
        toast(LiveToast::FollowedOnTelegram, "");
        return;
    }
    let name = card.as_ref().map(|card| card.name.to_string()).unwrap_or_default();
    let changed = with_state(|state| {
        if on {
            if state.follows.iter().any(|follow| follow.id == id) {
                return false;
            }
            let Some(card) = card.as_ref() else { return false };
            state.follows.push(follows::Follow { id: id.to_string(), source: channel::source_id(card.source).to_string(), name: card.name.to_string(), added: now() });
        } else {
            let before = state.follows.len();
            state.follows.retain(|follow| follow.id != id);
            if state.follows.len() == before {
                return false;
            }
            state.followed.remove(id);
        }
        if let Err(err) = follows::save(&state.follows) {
            eprintln!("live: the follows could not be saved: {err}");
        }
        true
    })
    .unwrap_or(false);
    if !changed {
        return;
    }
    mark_cards(id, |card| card.following = on);
    with_directory(|directory| {
        if directory.get_channel().id == id {
            let mut open = directory.get_channel();
            open.following = on;
            directory.set_channel(open);
        }
    });
    toast(if on { LiveToast::Following } else { LiveToast::Unfollowed }, &name);
    if on && let Some(card) = card {
        with_state(|state| {
            let mut card = card;
            card.following = true;
            state.followed.insert(id.to_string(), card);
        });
    }
    show_followed();
}

/// A card shown anywhere, by id.
fn find_card(id: &str) -> Option<LiveChannel> {
    with_state(|state| {
        for model in [&state.results, &state.home_here, &state.following_live, &state.following_off] {
            for row in 0..model.row_count() {
                if let Some(card) = model.row_data(row)
                    && card.id == id
                {
                    return Some(card);
                }
            }
        }
        state.followed.get(id).cloned()
    })
    .flatten()
}

/// Where a channel shown is.
pub(crate) fn link_of(id: &str) -> Option<Link> {
    with_state(|state| state.links.get(id).cloned()).flatten()
}

/// A channel opened from somewhere the directory did not list it (a pasted link): known by id.
pub(crate) fn remember(card: &LiveChannel, link: Link) {
    with_state(|state| {
        state.links.insert(card.id.to_string(), link);
        state.followed.entry(card.id.to_string()).or_insert_with(|| card.clone());
    });
}

/// Look a pasted link up in the directory, by its host or a platform's handle.
pub(crate) fn lookup(request: u64, platform: Option<String>, handle: Option<String>, host: Option<String>) {
    ask(Job::Lookup { request, platform, handle, host });
}

/// The volume and mute the Live view plays at.
pub(crate) fn sound() -> (i32, bool) {
    let mut sound = (100, false);
    UI.with(|ui| {
        if let Some(ui) = ui.borrow().as_ref().and_then(slint::Weak::upgrade) {
            let live = ui.global::<crate::Live>();
            sound = (live.get_volume(), live.get_muted());
        }
    });
    sound
}

/// Ask the directory about the followed channels.
fn refresh_followed() {
    let job = with_state(|state| {
        state.follow_request += 1;
        let ids: Vec<String> = state.follows.iter().map(|follow| follow.id.clone()).collect();
        (!ids.is_empty()).then_some(Job::Followed { request: state.follow_request, ids })
    })
    .flatten();
    if let Some(job) = job {
        ask(job);
    }
    show_followed();
}

fn followed_came(request: u64, found: Vec<(String, Option<Result_>)>) {
    let current = with_state(|state| state.follow_request == request).unwrap_or(false);
    if !current {
        return;
    }
    let now = now();
    let mut thumbs = Vec::new();
    with_state(|state| {
        for (id, result) in found {
            if let Some(link) = result.as_ref().and_then(Link::of) {
                state.links.insert(id.clone(), link);
            }
            match result.as_ref().and_then(|result| channel::from_result(result, now)) {
                Some(mut card) => {
                    card.following = true;
                    if let Some(url) = result.as_ref().and_then(channel::thumbnail_of) {
                        thumbs.push((id.clone(), url));
                    }
                    state.followed.insert(id, card);
                }
                None => {
                    // Gone from the directory (its owner took it out, or it was not seen for long):
                    // it stays followed, with its name, its status unknown.
                    if let Some(follow) = state.follows.iter().find(|follow| follow.id == id) {
                        state.followed.insert(id.clone(), follows::placeholder(follow));
                    }
                }
            }
        }
    });
    show_followed();
    for (id, url) in thumbs {
        thumbnails::fetch(&id, &url);
    }
}

/// Following and the column's groups: the followed channels live first, then the rest; with the
/// live streams the user follows on Telegram (src/telegram/live.rs).
pub fn show_followed() {
    let telegram = crate::telegram::live_rows_followed();
    with_state(|state| {
        let mut live: Vec<LiveChannel> = telegram;
        let mut off = Vec::new();
        for follow in &state.follows {
            let card = state.followed.get(&follow.id).cloned().unwrap_or_else(|| follows::placeholder(follow));
            if card.status == LiveStatus::Live {
                live.push(card);
            } else {
                off.push(card);
            }
        }
        live.sort_by(|a, b| b.viewers.cmp(&a.viewers).then_with(|| a.name.cmp(&b.name)));
        off.sort_by(|a, b| {
            let rank = |card: &LiveChannel| match card.status {
                LiveStatus::Stale => 0,
                LiveStatus::Offline => 1,
                _ => 2,
            };
            rank(a).cmp(&rank(b)).then_with(|| a.last_live_minutes.cmp(&b.last_live_minutes)).then_with(|| a.name.cmp(&b.name))
        });
        state.following_live.set_vec(live);
        state.following_off.set_vec(off);
    });
}

fn set_show_mature(on: bool) {
    with_state(|state| {
        let mut settings = state.settings.borrow_mut();
        settings.live.show_mature = on;
        settings.save();
    });
    with_directory(|directory| {
        directory.set_show_mature(on);
        directory.set_content_asking(false);
    });
    run(false);
}

/// The languages Settings › Live offers: FinchGram's own eleven, and any other chosen before.
const OFFERED_LANGUAGES: [&str; 11] = ["en", "zh-Hans", "zh-Hant", "es", "pt", "de", "fr", "ru", "ja", "ko", "ar"];

fn language_label(tag: &str) -> String {
    match tag {
        "zh-Hans" => "中文（简体）".to_string(),
        "zh-Hant" => "中文（繁體）".to_string(),
        _ => channel::language_name(tag),
    }
}

fn show_languages() {
    let chosen = with_state(|state| state.settings.borrow().live.languages.clone()).unwrap_or_default();
    let mut tags: Vec<String> = OFFERED_LANGUAGES.iter().map(|tag| (*tag).to_string()).collect();
    for tag in &chosen {
        if !tags.contains(tag) {
            tags.push(tag.clone());
        }
    }
    let options: Vec<LiveOption> = tags
        .into_iter()
        .map(|tag| LiveOption {
            label: language_label(&tag).into(),
            chosen: chosen.contains(&tag),
            key: tag.into(),
            source: LiveSource::Owncast,
            category: LiveCategory::None,
            count: -1,
        })
        .collect();
    with_directory(|directory| directory.set_preferred_languages(ModelRc::new(VecModel::from(options))));
}

/// A language of Settings › Live chosen or let go: Recommended follows.
fn toggle_language(tag: &str) {
    let languages = with_state(|state| {
        let mut settings = state.settings.borrow_mut();
        if let Some(at) = settings.live.languages.iter().position(|chosen| chosen == tag) {
            settings.live.languages.remove(at);
        } else {
            settings.live.languages.push(tag.to_string());
        }
        settings.save();
        settings.live.languages.clone()
    })
    .unwrap_or_default();
    show_languages();
    ask(Job::Languages(languages));
    run(false);
}

/// A short note at the bottom of the view, for a few seconds.
pub(crate) fn toast(what: LiveToast, name: &str) {
    with_directory(|directory| {
        directory.set_toast(what);
        directory.set_toast_name(name.into());
    });
    let clear = slint::Timer::default();
    clear.start(slint::TimerMode::SingleShot, Duration::from_millis(2600), || with_directory(|directory| directory.set_toast(LiveToast::None)));
    TOAST_TIMER.with(|kept| *kept.borrow_mut() = Some(clear));
}

thread_local! {
    static TOAST_TIMER: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
}
