//! Watch (the canvas, rounds 12 and 13): a channel opened from Discover, Following, the column or
//! a pasted link. The app asks the channel's source for the stream when it plays, never the
//! directory: an Owncast server its /api/status and /hls/stream.m3u8, a PeerTube instance its live
//! video's playlist, an Icecast station its stream, a submitted stream its playlist; a Telegram
//! channel through TDLib, played as the user's own live streams are (src/telegram/live.rs). The
//! picture is the live player's (src/player/live.rs). A stream left while it plays goes on in the
//! mini player until stopped.
//!
//! Connections › Open a link lives here too: a pasted link is recognised by its form, looked up in
//! the directory by its host or handle alone, else its server is asked what it is.

use std::cell::{Cell, RefCell};
use std::io::Read as _;
use std::time::{Duration, Instant};

use serde_json::Value;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::{Link, channel, with_directory};
use crate::player::live::{self as player, Owner};
use crate::{LiveCategory, LiveChannel, LiveDirectory, LiveLinkState, LivePlay, LiveSource, LiveStatus, LiveToast, LiveWatch, LiveWhy, MainWindow};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const CALL_TIMEOUT: Duration = Duration::from_secs(15);
/// An answer of a source larger than this is not one of its API's.
const LARGEST_ANSWER: u64 = 4 * 1024 * 1024;
/// The quality menu and the line under the picture are read this long after the stream opens,
/// once mpv has seen its variants.
const FACTS_AFTER: Duration = Duration::from_millis(1500);

/// The stream open in Watch.
struct Current {
    id: String,
    link: Option<Link>,
    /// The address mpv plays; None for Telegram's (played by src/telegram/live.rs) or while asked.
    address: Option<String>,
    telegram: bool,
    /// The variants as mpv lists them: (video track id, height, bitrate).
    variants: Vec<(i64, i64, i64)>,
    paused_at: Option<Instant>,
    behind: i32,
}

thread_local! {
    static CURRENT: RefCell<Option<Current>> = const { RefCell::new(None) };
    static REQUEST: Cell<u64> = const { Cell::new(0) };
    static LINK_REQUEST: Cell<u64> = const { Cell::new(0) };
    static FACTS_TIMER: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
    static BEHIND_TIMER: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
    /// What Open a link found, to open it.
    static FOUND_LINK: RefCell<Option<(LiveChannel, Link, Option<String>)>> = const { RefCell::new(None) };
}

pub fn connect(ui: &MainWindow) {
    let directory = ui.global::<LiveDirectory>();
    directory.on_set_quality(set_quality);
    directory.on_toggle_pause(toggle_pause);
    directory.on_live_edge(live_edge);
    directory.on_confirm_mature(|| {
        set_state(LiveWatch::Resolving);
        let current = CURRENT.with(|current| current.borrow().as_ref().map(|current| (current.id.clone(), current.link.clone())));
        if let Some((id, link)) = current
            && let Some(card) = super::find_card(&id)
        {
            resolve(card, link);
        }
    });
    directory.on_open_elsewhere(|| {
        let current = CURRENT.with(|current| current.borrow().as_ref().map(|current| (current.id.clone(), current.link.clone())));
        if let Some((id, link)) = current {
            open_in_browser(super::find_card(&id), link);
        }
    });
    directory.on_stop(stop);
    directory.on_resume(|| {
        with_directory(|directory| {
            directory.set_watching(true);
            directory.set_mini(false);
            directory.set_column_open(false);
        });
    });
    directory.on_pop_out(pop_out);
    directory.on_pop_in(pop_in);
    player::on_loaded(Owner::Directory, loaded);
    player::on_ended(Owner::Directory, ended);
}

fn set_state(state: LiveWatch) {
    with_directory(|directory| directory.set_watch_state(state));
}

/// A channel's one action (Discover's cards, Following's rows, the column, a suggestion).
pub fn open(id: &str) {
    let card = super::find_card(id);
    // The user's own Telegram live streams: the 0.7.10 player and comments.
    if id.starts_with("story:") || id.starts_with("chat:") {
        let card = card.unwrap_or_else(|| placeholder(id, LiveSource::Telegram));
        begin(card, None, true);
        set_state(LiveWatch::Playing);
        crate::telegram::watch_live_row(id);
        return;
    }
    let Some(card) = card else { return };
    let link = super::link_of(id);
    if card.play == LivePlay::Elsewhere {
        open_in_browser(Some(card), link);
        return;
    }
    let show_mature = with_directory_get(|directory| directory.get_show_mature());
    let state = if card.why == LiveWhy::Webrtc {
        LiveWatch::Phone
    } else if card.why == LiveWhy::Offline {
        LiveWatch::Offline
    } else if card.play == LivePlay::SignIn {
        LiveWatch::SignIn
    } else if card.mature && !show_mature {
        LiveWatch::Mature
    } else {
        LiveWatch::Resolving
    };
    begin(card.clone(), link.clone(), card.source == LiveSource::Telegram);
    set_state(state);
    if state == LiveWatch::Resolving {
        resolve(card, link);
    }
}

fn with_directory_get<R: Default>(read: impl FnOnce(&LiveDirectory<'_>) -> R) -> R {
    let mut value = R::default();
    with_directory(|directory| value = read(&directory));
    value
}

/// Watch opens on `card`: whatever played before stops.
fn begin(card: LiveChannel, link: Option<Link>, telegram: bool) {
    stop_playing();
    REQUEST.set(REQUEST.get() + 1);
    with_directory(|directory| {
        directory.set_channel(card.clone());
        directory.set_watching(true);
        directory.set_column_open(false);
        directory.set_mini(false);
        directory.set_paused(false);
        directory.set_behind(0);
        directory.set_qualities(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
        directory.set_quality(0);
        directory.set_tech(SharedString::new());
        directory.set_problem(SharedString::new());
        directory.set_chat(matches!(card.source, LiveSource::Telegram | LiveSource::Owncast));
    });
    CURRENT.with(|current| {
        *current.borrow_mut() = Some(Current { id: card.id.to_string(), link, address: None, telegram, variants: Vec::new(), paused_at: None, behind: 0 });
    });
}

/// Ask the source for the stream, off the UI thread, and play it.
fn resolve(card: LiveChannel, link: Option<Link>) {
    let request = REQUEST.get() + 1;
    REQUEST.set(request);
    if card.source == LiveSource::Telegram {
        let Some(username) = link.as_ref().and_then(|link| link.handle.clone().or(Some(link.source_ref.clone()))).filter(|name| !name.is_empty()) else {
            set_state(LiveWatch::Offline);
            return;
        };
        CURRENT.with(|current| {
            if let Some(current) = current.borrow_mut().as_mut() {
                current.telegram = true;
            }
        });
        crate::telegram::watch_live_username(&username, move |found| {
            if REQUEST.get() != request {
                return;
            }
            set_state(if found { LiveWatch::Playing } else { LiveWatch::Offline });
        });
        return;
    }
    let Some(link) = link else {
        set_state(LiveWatch::Failed);
        return;
    };
    let _ = std::thread::Builder::new().name("live resolve".into()).spawn(move || {
        let resolved = address_of(&link);
        let _ = slint::invoke_from_event_loop(move || resolved_came(request, resolved));
    });
}

/// What a source said of the stream.
enum Resolved {
    Play { address: String, viewers: Option<i64>, title: Option<String> },
    Offline,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(format!("FinchGram/{}", crate::update::CURRENT_VERSION))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(Some(CALL_TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into()
}

/// A source's JSON answer; None for a status other than 2xx (an answer that says nothing here).
fn get_json(agent: &ureq::Agent, address: &str) -> Result<Option<Value>, String> {
    let mut response = agent.get(address).header("Accept", "application/json").call().map_err(|err| err.to_string())?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Ok(None);
    }
    let mut body = Vec::new();
    response.body_mut().as_reader().take(LARGEST_ANSWER).read_to_end(&mut body).map_err(|err| err.to_string())?;
    Ok(serde_json::from_slice(&body).ok())
}

fn origin(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

/// The stream's address, asked of its source (blocking; the resolver's thread).
fn address_of(link: &Link) -> Result<Resolved, String> {
    let agent = agent();
    match link.source {
        LiveSource::Owncast => {
            let origin = origin(&link.url).ok_or("not an address")?;
            let status = get_json(&agent, &format!("{origin}/api/status"))?.ok_or("the server did not say whether it is live")?;
            if status.get("online").and_then(Value::as_bool) != Some(true) {
                return Ok(Resolved::Offline);
            }
            Ok(Resolved::Play {
                address: format!("{origin}/hls/stream.m3u8"),
                viewers: status.get("viewerCount").and_then(Value::as_i64),
                title: status.get("streamTitle").and_then(Value::as_str).filter(|title| !title.is_empty()).map(str::to_string),
            })
        }
        LiveSource::Peertube => {
            let (name, host) = link.source_ref.rsplit_once('@').ok_or("not a PeerTube channel")?;
            let lives = get_json(&agent, &format!("https://{host}/api/v1/video-channels/{name}/videos?isLive=true&count=5&sort=-publishedAt"))?
                .ok_or("the instance did not list the channel's lives")?;
            let Some(uuid) = lives
                .get("data")
                .and_then(Value::as_array)
                .and_then(|videos| videos.iter().find(|video| video.get("isLive").and_then(Value::as_bool) == Some(true)))
                .and_then(|video| video.get("uuid").and_then(Value::as_str))
            else {
                return Ok(Resolved::Offline);
            };
            let video = get_json(&agent, &format!("https://{host}/api/v1/videos/{uuid}"))?.ok_or("the instance did not describe the live")?;
            // Published (1) is on air; waiting (4, 5) or anything else is not.
            if video.pointer("/state/id").and_then(Value::as_i64) != Some(1) {
                return Ok(Resolved::Offline);
            }
            let address = video
                .get("streamingPlaylists")
                .and_then(Value::as_array)
                .and_then(|lists| lists.first())
                .and_then(|list| list.get("playlistUrl"))
                .and_then(Value::as_str)
                .ok_or("the live has no playlist (a members-only live?)")?;
            Ok(Resolved::Play {
                address: address.to_string(),
                viewers: video.get("viewers").and_then(Value::as_i64),
                title: video.get("name").and_then(Value::as_str).map(str::to_string),
            })
        }
        LiveSource::Icecast | LiveSource::Hls => Ok(Resolved::Play { address: link.url.clone(), viewers: None, title: None }),
        _ => Err("this source plays on its own site".to_string()),
    }
}

fn resolved_came(request: u64, resolved: Result<Resolved, String>) {
    if REQUEST.get() != request {
        return;
    }
    match resolved {
        Ok(Resolved::Play { address, viewers, title }) => {
            CURRENT.with(|current| {
                if let Some(current) = current.borrow_mut().as_mut() {
                    current.address = Some(address.clone());
                }
            });
            with_directory(|directory| {
                let mut card = directory.get_channel();
                if let Some(viewers) = viewers {
                    card.viewers = i32::try_from(viewers).unwrap_or(-1);
                }
                if let Some(title) = title {
                    card.title = title.into();
                }
                directory.set_channel(card);
            });
            let (volume, muted) = super::sound();
            player::play_url(&address, volume, muted);
            // Owncast's chat beside it, read and send.
            let owncast = CURRENT.with(|current| current.borrow().as_ref().and_then(|current| current.link.clone()).filter(|link| link.source == LiveSource::Owncast));
            if let Some(origin) = owncast.and_then(|link| origin(&link.url)) {
                super::chat::open(&origin);
            }
        }
        Ok(Resolved::Offline) => set_state(LiveWatch::Offline),
        Err(err) => {
            eprintln!("live: cannot open the stream: {err}");
            with_directory(|directory| directory.set_problem(err.into()));
            set_state(LiveWatch::Failed);
        }
    }
}

/// mpv opened the stream: it plays; the quality menu and the facts line follow shortly.
fn loaded() {
    set_state(LiveWatch::Playing);
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::SingleShot, FACTS_AFTER, read_facts);
    FACTS_TIMER.with(|kept| *kept.borrow_mut() = Some(timer));
}

fn read_facts() {
    let mut variants = player::variants();
    variants.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
    variants.dedup_by_key(|variant| variant.1);
    let (height, bitrate, format) = player::playing_facts();
    let audio = with_directory_get(|directory| directory.get_channel().audio);
    let kind = if audio { "Icecast".to_string() } else if format.contains("hls") || format.is_empty() { "HLS".to_string() } else { format.to_uppercase() };
    let mut parts = vec![kind];
    if height > 0 && !audio {
        parts.push(format!("{height}p"));
    }
    if bitrate > 0 {
        parts.push(if bitrate >= 1_000_000 { format!("{:.1} Mbps", bitrate as f64 / 1_000_000.0) } else { format!("{} kbps", bitrate / 1000) });
    }
    let mut qualities: Vec<SharedString> = Vec::new();
    if variants.len() > 1 {
        qualities.push("Auto".into());
        qualities.extend(variants.iter().map(|(_, height, _)| SharedString::from(format!("{height}p"))));
    }
    CURRENT.with(|current| {
        if let Some(current) = current.borrow_mut().as_mut() {
            current.variants = variants;
        }
    });
    with_directory(|directory| {
        directory.set_tech(parts.join(" · ").into());
        directory.set_qualities(ModelRc::new(VecModel::from(qualities)));
    });
}

/// mpv came to the end of the stream (it ended), or could not play it.
fn ended(failed: bool) {
    let open = CURRENT.with(|current| current.borrow().as_ref().is_some_and(|current| !current.telegram && current.address.is_some()));
    if !open {
        return;
    }
    set_state(if failed { LiveWatch::Failed } else { LiveWatch::Offline });
    with_directory(|directory| directory.set_mini(false));
}

fn set_quality(index: i32) {
    let chosen = CURRENT.with(|current| {
        let current = current.borrow();
        let current = current.as_ref()?;
        if index <= 0 {
            return Some(None);
        }
        current.variants.get(usize::try_from(index - 1).ok()?).map(|variant| Some(variant.0))
    });
    if let Some(chosen) = chosen {
        player::choose_variant(chosen);
        with_directory(|directory| directory.set_quality(index.max(0)));
        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::SingleShot, FACTS_AFTER, read_facts);
        FACTS_TIMER.with(|kept| *kept.borrow_mut() = Some(timer));
    }
}

/// Held, or going on from where it was held: what came meanwhile is kept (mpv's cache), the
/// stream then behind the live edge by the time held.
fn toggle_pause() {
    let paused = !with_directory_get(|directory| directory.get_paused());
    player::set_paused(paused);
    with_directory(|directory| directory.set_paused(paused));
    CURRENT.with(|current| {
        if let Some(current) = current.borrow_mut().as_mut() {
            if paused {
                current.paused_at = Some(Instant::now());
            } else if let Some(at) = current.paused_at.take() {
                current.behind += i32::try_from(at.elapsed().as_secs()).unwrap_or(0);
            }
        }
    });
    if paused {
        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::Repeated, Duration::from_secs(1), || {
            let behind = CURRENT.with(|current| {
                current.borrow().as_ref().map_or(0, |current| current.behind + current.paused_at.map_or(0, |at| i32::try_from(at.elapsed().as_secs()).unwrap_or(0)))
            });
            with_directory(|directory| directory.set_behind(behind));
        });
        BEHIND_TIMER.with(|kept| *kept.borrow_mut() = Some(timer));
    } else {
        BEHIND_TIMER.with(|kept| *kept.borrow_mut() = None);
    }
}

/// Back to the live edge: the stream opened again.
fn live_edge() {
    let address = CURRENT.with(|current| {
        let mut current = current.borrow_mut();
        let current = current.as_mut()?;
        current.behind = 0;
        current.paused_at = None;
        current.address.clone()
    });
    BEHIND_TIMER.with(|kept| *kept.borrow_mut() = None);
    with_directory(|directory| {
        directory.set_behind(0);
        directory.set_paused(false);
    });
    if let Some(address) = address {
        let (volume, muted) = super::sound();
        player::play_url(&address, volume, muted);
    }
}

/// The stream playing in Watch goes into the pop-out window, on top of every app; Watch closes
/// and the view goes back to where it was. A directory's stream only: Telegram's play here.
fn pop_out() {
    let address = CURRENT.with(|current| current.borrow().as_ref().filter(|current| !current.telegram).and_then(|current| current.address.clone()));
    let Some(address) = address else { return };
    player::stop();
    let mut opened = false;
    super::UI.with(|ui| {
        if let Some(ui) = ui.borrow().as_ref().and_then(slint::Weak::upgrade) {
            let directory = ui.global::<LiveDirectory>();
            let words = ui.global::<crate::LiveDirectoryWords>();
            super::popout::open(&address, &directory.get_channel(), ui.global::<crate::Live>().get_muted(), &words);
            opened = true;
        }
    });
    if opened {
        with_directory(|directory| {
            directory.set_popped_out(true);
            directory.set_watching(false);
            directory.set_mini(false);
            directory.set_column_open(true);
        });
    }
}

/// The pop-out's stream back into the main window's Watch.
pub fn pop_in() {
    let Some(address) = super::popout::close() else { return };
    with_directory(|directory| {
        directory.set_popped_out(false);
        directory.set_watching(true);
        directory.set_mini(false);
        directory.set_column_open(false);
    });
    let (volume, muted) = super::sound();
    player::play_url(&address, volume, muted);
}

fn stop_playing() {
    super::chat::close();
    let _ = super::popout::close();
    let telegram = CURRENT.with(|current| current.borrow_mut().take().map(|current| current.telegram));
    match telegram {
        Some(true) => crate::telegram::close_live(),
        Some(false) => player::stop(),
        None => {}
    }
    BEHIND_TIMER.with(|kept| *kept.borrow_mut() = None);
}

/// Stopped: nothing plays, Watch and the mini player close.
pub fn stop() {
    stop_playing();
    with_directory(|directory| {
        directory.set_watching(false);
        directory.set_mini(false);
        directory.set_popped_out(false);
        directory.set_column_open(true);
    });
}

/// The user left Watch for a section or the chats: a stream that plays goes on in the mini player,
/// anything else (offline, a question) closes.
pub fn minimise() {
    // In the pop-out window it plays on, and the main window shows nothing of it.
    if super::popout::is_open() {
        return;
    }
    let state = with_directory_get(|directory| directory.get_watch_state());
    let playing = matches!(state, LiveWatch::Playing | LiveWatch::Network | LiveWatch::Resolving);
    // Telegram's streams stop with the view, as before: no mini player for them.
    let directory_stream = CURRENT.with(|current| current.borrow().as_ref().is_some_and(|current| !current.telegram));
    if playing && directory_stream {
        with_directory(|directory| directory.set_mini(true));
    } else {
        stop_playing();
        with_directory(|directory| directory.set_mini(false));
    }
}

/// Back from Watch (its back link).
pub fn leave() {
    minimise();
}

/// A channel's own page, in the browser (Terminal's [o page]).
pub fn open_page(id: &str) {
    let card = super::find_card(id);
    open_in_browser(card, super::link_of(id));
}

/// A channel that plays on its own site, opened there.
fn open_in_browser(card: Option<LiveChannel>, link: Option<Link>) {
    let Some(url) = link.map(|link| link.url) else { return };
    crate::platform::open_link(&url);
    let host = card.map(|card| card.host.to_string()).filter(|host| !host.is_empty()).or_else(|| url::Url::parse(&url).ok().and_then(|url| url.host_str().map(str::to_string)));
    super::toast(LiveToast::OpenedInBrowser, &host.unwrap_or_default());
}

/// A card for something not in the directory: a pasted link's stream, a Telegram row not shown.
fn placeholder(id: &str, source: LiveSource) -> LiveChannel {
    LiveChannel {
        id: id.into(),
        source,
        name: id.into(),
        initial: channel::initial_of(id).into(),
        hue: channel::hue_of(id),
        status: LiveStatus::Live,
        observed_minutes: 0,
        started_minutes: -1,
        last_live_minutes: -1,
        viewers: -1,
        category: LiveCategory::None,
        play: LivePlay::Watch,
        why: LiveWhy::None,
        upcoming_minutes: -1,
        ..Default::default()
    }
}

// ---- Connections › Open a link ----------------------------------------------------------------

/// What a pasted link is, by its form alone.
#[derive(Debug, Clone, PartialEq)]
enum Form {
    Nothing,
    /// A stream key or a token: for sending, not for watching here.
    Private,
    Telegram(String),
    Elsewhere(LiveSource),
    /// A stream by its address: an HLS or DASH playlist, an RTSP stream.
    Stream(String),
    /// A site: looked up in the directory, else asked what it is.
    Site { url: String, host: String },
}

fn form_of(text: &str) -> Form {
    let text = text.trim();
    if text.is_empty() {
        return Form::Nothing;
    }
    let with_scheme = if text.contains("://") { text.to_string() } else { format!("https://{text}") };
    let Ok(url) = url::Url::parse(&with_scheme) else { return Form::Nothing };
    let scheme = url.scheme().to_ascii_lowercase();
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if matches!(scheme.as_str(), "rtmp" | "rtmps" | "srt") {
        return Form::Private;
    }
    let secret = url.query_pairs().any(|(key, _)| matches!(key.to_ascii_lowercase().as_str(), "token" | "key" | "auth" | "signature" | "sig" | "expires" | "exp" | "hdnts" | "hmac" | "session" | "streamkey"));
    if secret || !url.username().is_empty() || url.password().is_some() {
        return Form::Private;
    }
    let bare = host.trim_start_matches("www.").trim_start_matches("m.");
    if bare == "t.me" || bare == "telegram.me" {
        let user = url.path_segments().and_then(|mut parts| parts.next()).unwrap_or("").to_string();
        return if user.is_empty() { Form::Nothing } else { Form::Telegram(user) };
    }
    for (domain, source) in [
        ("twitch.tv", LiveSource::Twitch),
        ("youtube.com", LiveSource::Youtube),
        ("youtu.be", LiveSource::Youtube),
        ("kick.com", LiveSource::Kick),
        ("bilibili.com", LiveSource::Bilibili),
    ] {
        if bare == domain || bare.ends_with(&format!(".{domain}")) {
            return Form::Elsewhere(source);
        }
    }
    let path = url.path().to_ascii_lowercase();
    if scheme == "rtsp" || path.ends_with(".m3u8") || path.ends_with(".mpd") {
        return Form::Stream(url.to_string());
    }
    if (scheme == "https" || scheme == "http") && !host.is_empty() {
        return Form::Site { url: url.to_string(), host };
    }
    Form::Nothing
}

fn set_link(state: LiveLinkState, source: LiveSource) {
    with_directory(|directory| {
        directory.set_link_state(state);
        directory.set_link_source(source);
    });
}

/// The link field changed (Connections › Open a link, or a link typed in Discover's box).
pub fn type_link(text: &str) {
    let request = LINK_REQUEST.get() + 1;
    LINK_REQUEST.set(request);
    FOUND_LINK.with(|found| *found.borrow_mut() = None);
    with_directory(|directory| {
        directory.set_link(text.into());
        directory.set_link_problem(SharedString::new());
    });
    match form_of(text) {
        Form::Nothing => set_link(if text.trim().is_empty() { LiveLinkState::Empty } else { LiveLinkState::NotStream }, LiveSource::Hls),
        Form::Private => set_link(LiveLinkState::Private, LiveSource::Hls),
        Form::Telegram(user) => {
            let card = LiveChannel { name: format!("@{user}").into(), ..placeholder(&format!("tg:{user}"), LiveSource::Telegram) };
            let link = Link { url: format!("https://t.me/{user}"), source: LiveSource::Telegram, source_ref: user.to_lowercase(), handle: Some(user) };
            found(card, link, None, LiveLinkState::Recognized);
        }
        Form::Elsewhere(source) => {
            set_link(LiveLinkState::Elsewhere, source);
            let url = if text.contains("://") { text.trim().to_string() } else { format!("https://{}", text.trim()) };
            let card = placeholder(&url, source);
            FOUND_LINK.with(|found| *found.borrow_mut() = Some((card, Link { url, source, source_ref: String::new(), handle: None }, None)));
        }
        Form::Stream(url) => {
            let card = LiveChannel { name: url::Url::parse(&url).ok().and_then(|url| url.host_str().map(str::to_string)).unwrap_or_default().into(), ..placeholder(&format!("link:{url}"), LiveSource::Hls) };
            let link = Link { url: url.clone(), source: LiveSource::Hls, source_ref: url.clone(), handle: None };
            found(card, link, Some(url), LiveLinkState::Playable);
        }
        Form::Site { url, host } => {
            set_link(LiveLinkState::Probing, LiveSource::Hls);
            // The directory first, by the host alone: the server visits nothing, nothing more is sent.
            super::lookup(request, None, None, Some(host));
            FOUND_LINK.with(|found| *found.borrow_mut() = Some((placeholder(&url, LiveSource::Hls), Link { url, source: LiveSource::Hls, source_ref: String::new(), handle: None }, None)));
        }
    }
}

fn found(card: LiveChannel, link: Link, address: Option<String>, state: LiveLinkState) {
    with_directory(|directory| directory.set_link_channel(card.clone()));
    set_link(state, card.source);
    FOUND_LINK.with(|found| *found.borrow_mut() = Some((card, link, address)));
}

/// The directory's answer to a link's host.
pub fn looked_up(request: u64, result: Result<Option<super::directory::Result_>, String>) {
    if LINK_REQUEST.get() != request {
        return;
    }
    if let Ok(Some(result)) = &result
        && let Some(card) = channel::from_result(result, chrono::Utc::now().timestamp())
        && let Some(link) = Link::of(result)
    {
        super::remember(&card, link.clone());
        found(card, link, None, LiveLinkState::Found);
        return;
    }
    // Not in the directory: the server itself is asked what it is.
    let Some(url) = FOUND_LINK.with(|found| found.borrow().as_ref().map(|(_, link, _)| link.url.clone())) else { return };
    let _ = std::thread::Builder::new().name("live probe".into()).spawn(move || {
        let probed = probe(&url);
        let _ = slint::invoke_from_event_loop(move || probed_came(request, probed));
    });
}

/// What a probe found, in plain data (a card is made on the UI thread).
struct Probed {
    id: String,
    name: String,
    host: String,
    source: LiveSource,
    link: Link,
    address: Option<String>,
}

impl Probed {
    fn card(&self) -> LiveChannel {
        let audio = self.source == LiveSource::Icecast;
        LiveChannel {
            name: self.name.clone().into(),
            host: self.host.clone().into(),
            audio,
            play: if audio { LivePlay::Listen } else { LivePlay::Watch },
            ..placeholder(&self.id, self.source)
        }
    }
}

/// What a site turned out to be: an Owncast server, a PeerTube live, an Icecast server.
fn probe(url: &str) -> Result<Option<Probed>, String> {
    let agent = agent();
    let origin = origin(url).ok_or("not an address")?;
    let host = url::Url::parse(url).ok().and_then(|url| url.host_str().map(str::to_string)).unwrap_or_default();
    // Owncast answers its config at /api/config.
    if let Ok(Some(config)) = get_json(&agent, &format!("{origin}/api/config"))
        && config.get("name").and_then(Value::as_str).is_some()
        && config.get("streamTitle").is_some()
    {
        let name = config.get("name").and_then(Value::as_str).unwrap_or(&host).to_string();
        let link = Link { url: origin.clone(), source: LiveSource::Owncast, source_ref: host.clone(), handle: None };
        return Ok(Some(Probed { id: format!("link:{origin}"), name, host, source: LiveSource::Owncast, link, address: None }));
    }
    // A PeerTube video page: /w/<id> or /videos/watch/<id>.
    if let Ok(parsed) = url::Url::parse(url) {
        let segments: Vec<&str> = parsed.path_segments().map(Iterator::collect).unwrap_or_default();
        let video = match segments.as_slice() {
            ["w", id] | ["videos", "watch", id] => Some((*id).to_string()),
            _ => None,
        };
        if let Some(video) = video
            && let Ok(Some(answer)) = get_json(&agent, &format!("{origin}/api/v1/videos/{video}"))
            && answer.get("isLive").and_then(Value::as_bool) == Some(true)
        {
            let name = answer.pointer("/channel/displayName").and_then(Value::as_str).unwrap_or(&host).to_string();
            let channel_name = answer.pointer("/channel/name").and_then(Value::as_str).unwrap_or("").to_string();
            let address = answer.pointer("/streamingPlaylists/0/playlistUrl").and_then(Value::as_str).map(str::to_string);
            let link = Link { url: url.to_string(), source: LiveSource::Peertube, source_ref: format!("{channel_name}@{host}"), handle: None };
            return Ok(Some(Probed { id: format!("link:{url}"), name, host, source: LiveSource::Peertube, link, address }));
        }
    }
    // Icecast's status page lists its streams.
    if let Ok(Some(status)) = get_json(&agent, &format!("{origin}/status-json.xsl")) {
        let sources = status.pointer("/icestats/source");
        let first = match sources {
            Some(Value::Array(list)) => list.first().cloned(),
            Some(source @ Value::Object(_)) => Some(source.clone()),
            _ => None,
        };
        if let Some(source) = first
            && let Some(listen) = source.get("listenurl").and_then(Value::as_str)
        {
            let name = source.get("server_name").and_then(Value::as_str).unwrap_or(&host).to_string();
            let link = Link { url: listen.to_string(), source: LiveSource::Icecast, source_ref: listen.to_string(), handle: None };
            return Ok(Some(Probed { id: format!("link:{listen}"), name, host, source: LiveSource::Icecast, link, address: Some(listen.to_string()) }));
        }
    }
    Ok(None)
}

fn probed_came(request: u64, probed: Result<Option<Probed>, String>) {
    if LINK_REQUEST.get() != request {
        return;
    }
    match probed {
        Ok(Some(probed)) => found(probed.card(), probed.link, probed.address, LiveLinkState::Playable),
        Ok(None) => set_link(LiveLinkState::NotStream, LiveSource::Hls),
        Err(err) => {
            with_directory(|directory| directory.set_link_problem(err.into()));
            set_link(LiveLinkState::Failed, LiveSource::Hls);
        }
    }
}

/// Open what the link led to.
pub fn open_link() {
    let Some((card, link, address)) = FOUND_LINK.with(|found| found.borrow().clone()) else { return };
    let state = with_directory_get(|directory| directory.get_link_state());
    match state {
        LiveLinkState::Found => open(&card.id),
        LiveLinkState::Elsewhere => open_in_browser(Some(card), Some(link)),
        LiveLinkState::Recognized if card.source == LiveSource::Telegram => {
            super::remember(&card, link.clone());
            begin(card.clone(), Some(link.clone()), true);
            set_state(LiveWatch::Resolving);
            resolve(card, Some(link));
        }
        LiveLinkState::Playable => {
            super::remember(&card, link.clone());
            begin(card, Some(link.clone()), false);
            set_state(LiveWatch::Resolving);
            match address {
                Some(address) => resolved_came(REQUEST.get(), Ok(Resolved::Play { address, viewers: None, title: None })),
                None => {
                    let request = REQUEST.get() + 1;
                    REQUEST.set(request);
                    let _ = std::thread::Builder::new().name("live resolve".into()).spawn(move || {
                        let resolved = address_of(&link);
                        let _ = slint::invoke_from_event_loop(move || resolved_came(request, resolved));
                    });
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pasted_link_is_told_by_its_form() {
        assert_eq!(form_of(""), Form::Nothing);
        assert_eq!(form_of("rtmp://live.example.org/app/abcdef"), Form::Private);
        assert_eq!(form_of("https://cdn.example.org/live/index.m3u8?token=abc"), Form::Private);
        assert_eq!(form_of("t.me/techmorning"), Form::Telegram("techmorning".into()));
        assert_eq!(form_of("https://t.me/mika_keys/s/live"), Form::Telegram("mika_keys".into()));
        assert_eq!(form_of("https://www.twitch.tv/somebody"), Form::Elsewhere(LiveSource::Twitch));
        assert_eq!(form_of("youtu.be/xyz"), Form::Elsewhere(LiveSource::Youtube));
        assert_eq!(form_of("https://cam.example.org/live/index.m3u8"), Form::Stream("https://cam.example.org/live/index.m3u8".into()));
        assert_eq!(form_of("rtsp://cam.example.org/stream"), Form::Stream("rtsp://cam.example.org/stream".into()));
        assert_eq!(form_of("live.keyboardlab.example"), Form::Site { url: "https://live.keyboardlab.example/".into(), host: "live.keyboardlab.example".into() });
    }
}
