//! Live streams (docs/live-streams.md): Telegram's own, in a view of their own beside the chats,
//! the channels and the bots. Who is live: the live stories of contacts and of channels
//! (updateChatActiveStories, after loadActiveStories), and the video chats and live streams of
//! chats (a chat's video_chat, then getGroupCall). FinchGram has no WebRTC, so it plays what
//! Telegram serves as a stream: an RTMP stream (OBS and the like), joined as a viewer with a
//! payload that carries no media, then fetched a second at a time with getGroupCallStreamSegment.
//! Each second is Telegram's header and a whole MP4 file; src/player/remux.rs makes one Matroska
//! stream of them, and the live player (src/player/live.rs) plays it as it comes. A stream that
//! Telegram sends over WebRTC only is listed for the phone, and not joined.
//!
//! The comments are the group call's messages; the user's own stream is opened as its host (the
//! number watching, the Stars, deleting comments, the end). Going live asks TDLib for the RTMP
//! address and key that OBS sends to, then starts the live story or the channel's live stream.
//!
//! While the view is open it covers the open chat: what arrives there is not marked as seen until
//! the view closes (conversation.rs), and closing the view leaves the stream.
//!
//! Everything runs on the UI thread.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{Value, json};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::api::{self, ChatType, MessageSender};
use super::store::{self, Names};
use super::{Error, Slot, actions, errors, send, time};
use crate::player::pipe::Pipe;
use crate::player::remux::{RemuxError, Remuxer};
use crate::{ActionNotice, Live, LiveComment, LiveDonor, LiveKind, LiveRow, LiveState, LiveTarget, LiveWords, MainWindow};

/// Where a stream joined starts: this far behind its live edge, as Telegram's own apps keep it.
const BEHIND_EDGE_MS: i64 = 2000;
/// A second asked for before it exists (TIME_TOO_BIG): the edge is asked for again after this (the
/// broadcaster may have paused), and the second once the edge says it is there.
const NOT_YET: Duration = Duration::from_millis(250);
/// While the broadcaster sends nothing, look again this often.
const LOOK_AGAIN: Duration = Duration::from_secs(2);
/// After a failed fetch, wait this long; after this many in a row, give up.
const RETRY: Duration = Duration::from_secs(1);
const FAILURES_ALLOWED: u32 = 8;
/// The comments the column keeps.
const COMMENTS_KEPT: usize = 100;
/// A stream is listed as for the phone below these audiences, when it is not an RTMP stream (the
/// server switches larger ones to stream mode: core.telegram.org/api/group-calls). FinchGram
/// plays RTMP streams only, so those are for the phone too, for now.
const NOT_RTMP_IS_PHONE: bool = true;

/// What a live stream is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A live story of the chat (a user's own chat, or a channel's).
    Story { story_id: i32 },
    /// The chat's video chat or live stream.
    Call,
}

/// Who is live, for one account.
#[derive(Default)]
struct Found {
    /// Chats with a live story: the story, and when it began.
    stories: HashMap<i64, (i32, i32)>,
    /// What getStory said of a live story: its group call and whether it is an RTMP stream.
    story_calls: HashMap<(i64, i32), (i32, bool)>,
    /// Chats with a video chat or live stream on: its group call.
    chat_calls: HashMap<i64, i32>,
    /// What TDLib said of the group calls.
    calls: HashMap<i32, api::GroupCall>,
    /// Asked for and not answered yet: asked once.
    asking_calls: HashSet<i32>,
    asking_stories: HashSet<(i64, i32)>,
    /// loadActiveStories was asked: the main list's stories are on their way.
    stories_loading: bool,
}

/// The stream open.
struct Watching {
    slot: Slot,
    generation: u64,
    kind: Kind,
    /// Its group call, once known.
    call: i32,
    joined: bool,
    /// The user's own: opened as its host.
    hosting: bool,
    rejoined: bool,
    feed: Option<Feed>,
    comments: Vec<Comment>,
    failures: u32,
}

/// The stream as it is fetched: which channel, how long a segment is, the next one to ask for.
struct Feed {
    channel: i32,
    scale: i32,
    chunk_ms: i64,
    /// The time of the next segment, and of the first (the stream's zero).
    next: i64,
    first: i64,
    /// Where the stream ended when last asked, and when that was.
    edge: i64,
    edge_seen: Instant,
    remuxer: Remuxer,
    pipe: Arc<Pipe>,
    /// A segment is being fetched.
    asking: bool,
    /// Something has gone into this pipe: its stream has begun.
    pushed: bool,
}

struct Comment {
    id: i32,
    sender: MessageSender,
    text: String,
    stars: i64,
    host: bool,
    deletable: bool,
}

thread_local! {
    static FOUND: RefCell<HashMap<Slot, Found>> = RefCell::new(HashMap::new());
    static WATCHING: RefCell<Option<Watching>> = const { RefCell::new(None) };
    /// Counted up whenever a stream opens or closes: answers for one closed are dropped.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
    /// The view is open: it covers the open chat.
    static SHOWN: Cell<bool> = const { Cell::new(false) };
    /// How many Stars make each level of paid comments, lowest first.
    static LEVELS: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
    /// Going live: the targets offered, by chat, and which are stories.
    static TARGETS: RefCell<Vec<(i64, bool)>> = const { RefCell::new(Vec::new()) };
}

pub fn connect(ui: &MainWindow) {
    let live = ui.global::<Live>();
    live.on_show(show);
    live.on_watch(|id| watch(&id));
    live.on_watch_link(|text| watch_link(&text));
    live.on_close(|| close(true));
    live.on_toggle_mute(|| {
        with_live(|live| {
            let muted = !live.get_muted();
            live.set_muted(muted);
            crate::player::live::set_muted(muted);
        });
    });
    live.on_set_volume(|volume| {
        with_live(|live| {
            live.set_volume(volume);
            live.set_muted(false);
        });
        crate::player::live::set_volume(volume);
        crate::player::live::set_muted(false);
    });
    live.on_send_comment(|text| send_comment(&text));
    live.on_copy_link(|| {
        let link = with_live_get(|live| live.get_link().to_string());
        copy(&link, ActionNotice::LinkCopied);
    });
    live.on_delete_comment(delete_comment);
    live.on_toggle_comments(toggle_comments);
    live.on_ask_end(|asking| with_live(|live| live.set_ending(asking)));
    live.on_end_live(end_live);
    live.on_open_go_live(open_go_live);
    live.on_close_go_live(|| with_live(|live| live.set_going_live(false)));
    live.on_choose_target(|index| {
        with_live(|live| live.set_target(index));
        fetch_rtmp_url(false);
    });
    live.on_start(start_live);
    live.on_copy_server(|| copy(&with_live_get(|live| live.get_server().to_string()), ActionNotice::Copied));
    live.on_copy_key(|| copy(&with_live_get(|live| live.get_key().to_string()), ActionNotice::Copied));
    live.on_new_key(|| fetch_rtmp_url(true));
    crate::player::live::on_ended(player_ended);
}

// ---- who is live -------------------------------------------------------------------------------------

/// The account is ready: its story list's live stories are asked for, so that the view's entry
/// shows its dot without being opened first.
pub fn account_ready() {
    load_active_stories();
}

/// loadActiveStories until TDLib says all were sent (404): they come as updateChatActiveStories.
fn load_active_stories() {
    let slot = super::context();
    let asked = with_found(slot, |found| std::mem::replace(&mut found.stories_loading, true));
    if asked {
        return;
    }
    load_more_stories(slot);
}

fn load_more_stories(slot: Slot) {
    super::in_context(slot, || {
        send(json!({ "@type": "loadActiveStories", "story_list": { "@type": "storyListMain" } }), move |answer| match answer {
            Ok(_) => load_more_stories(slot),
            Err(Error::Telegram { code: 404, .. }) => {}
            Err(err) => {
                eprintln!("live: cannot load the active stories: {err}");
                with_found(slot, |found| found.stories_loading = false);
            }
        });
    });
}

/// A chat TDLib tells about for the first time, with its video chat.
pub fn new_chat(chat_id: i64, video_chat: Option<&api::VideoChat>) {
    if let Some(video_chat) = video_chat
        && video_chat.group_call_id != 0
    {
        self::video_chat(chat_id, video_chat);
    }
}

/// A chat's video chat or live stream began (a group call) or ended (0).
pub fn video_chat(chat_id: i64, video_chat: &api::VideoChat) {
    let slot = super::context();
    let call = video_chat.group_call_id;
    let changed = with_found(slot, |found| {
        if call == 0 {
            found.chat_calls.remove(&chat_id).is_some()
        } else {
            found.chat_calls.insert(chat_id, call) != Some(call)
        }
    });
    if call != 0 {
        ask_call(slot, call);
    }
    if changed {
        refresh_if_front(slot);
    }
}

/// A chat's active stories changed: a live story among them, or no longer.
pub fn active_stories(active: &api::ChatActiveStories) {
    let slot = super::context();
    let live = active.stories.iter().find(|story| story.is_live).map(|story| (story.story_id, story.date));
    let changed = with_found(slot, |found| match live {
        Some(story) => found.stories.insert(active.chat_id, story) != Some(story),
        None => found.stories.remove(&active.chat_id).is_some(),
    });
    if let Some((story_id, _)) = live {
        ask_story(slot, active.chat_id, story_id);
    }
    if changed {
        refresh_if_front(slot);
    }
}

pub fn story_deleted(chat_id: i64, story_id: i32) {
    let slot = super::context();
    let changed = with_found(slot, |found| {
        if found.stories.get(&chat_id).is_some_and(|(id, _)| *id == story_id) {
            found.stories.remove(&chat_id);
            true
        } else {
            false
        }
    });
    if changed {
        refresh_if_front(slot);
    }
}

/// A group call changed: its audience, its comments allowed or not, its end.
pub fn group_call(call: &api::GroupCall) {
    let slot = super::context();
    let known = with_found(slot, |found| {
        let known = found.calls.contains_key(&call.id) || found.chat_calls.values().any(|id| *id == call.id);
        if known || found.story_calls.values().any(|(id, _)| *id == call.id) {
            found.calls.insert(call.id, call.clone());
            if !call.is_active {
                found.chat_calls.retain(|_, id| *id != call.id);
            }
            true
        } else {
            false
        }
    });
    if known {
        refresh_if_front(slot);
    }
    let open = WATCHING.with(|watching| watching.borrow().as_ref().filter(|w| w.slot == slot && w.call == call.id).map(|w| w.generation));
    if let Some(generation) = open {
        show_call(call);
        if !call.is_active {
            stream_ended(generation);
        } else if call.need_rejoin {
            rejoin(generation);
        }
    }
}

/// getGroupCall, once for each call: what it is and how many watch.
fn ask_call(slot: Slot, call: i32) {
    let first = with_found(slot, |found| found.asking_calls.insert(call));
    if !first {
        return;
    }
    super::in_context(slot, || {
        send(json!({ "@type": "getGroupCall", "group_call_id": call }), move |answer| {
            with_found(slot, |found| found.asking_calls.remove(&call));
            match answer.map(serde_json::from_value::<api::GroupCall>) {
                Ok(Ok(group_call)) => group_call_answered(slot, group_call),
                Ok(Err(err)) => eprintln!("live: cannot read a group call: {err}"),
                Err(Error::Telegram { message, .. }) => {
                    // A call that is gone: its chat is no longer live.
                    eprintln!("live: group call {call}: {message}");
                    with_found(slot, |found| found.chat_calls.retain(|_, id| *id != call));
                    refresh_if_front(slot);
                }
                Err(Error::Stopped) => {}
            }
        });
    });
}

fn group_call_answered(slot: Slot, call: api::GroupCall) {
    with_found(slot, |found| {
        if !call.is_active {
            found.chat_calls.retain(|_, id| *id != call.id);
        }
        found.calls.insert(call.id, call);
    });
    refresh_if_front(slot);
}

/// getStory, once for each live story: its group call.
fn ask_story(slot: Slot, chat_id: i64, story_id: i32) {
    let wanted = with_found(slot, |found| !found.story_calls.contains_key(&(chat_id, story_id)) && found.asking_stories.insert((chat_id, story_id)));
    if !wanted {
        return;
    }
    super::in_context(slot, || {
        let request = json!({ "@type": "getStory", "story_poster_chat_id": chat_id, "story_id": story_id, "only_local": false });
        send(request, move |answer| {
            with_found(slot, |found| found.asking_stories.remove(&(chat_id, story_id)));
            match answer.map(serde_json::from_value::<api::Story>) {
                Ok(Ok(story)) => {
                    if let api::StoryContent::Live { group_call_id, is_rtmp_stream } = story.content {
                        with_found(slot, |found| found.story_calls.insert((chat_id, story_id), (group_call_id, is_rtmp_stream)));
                        ask_call(slot, group_call_id);
                        refresh_if_front(slot);
                    }
                }
                Ok(Err(err)) => eprintln!("live: cannot read a story: {err}"),
                Err(Error::Telegram { message, .. }) => eprintln!("live: story {story_id} of {chat_id}: {message}"),
                Err(Error::Stopped) => {}
            }
        });
    });
}

// ---- the list -------------------------------------------------------------------------------------

/// The account in front came to the front, or changed: its list.
pub fn refresh() {
    let slot = super::shown();
    let Some(ui) = super::window() else { return };
    let names = Names::from(&ui);
    let (stories, chats) = rows(slot, &names);
    let live = ui.global::<Live>();
    live.set_count((stories.len() + chats.len()) as i32);
    // The stream open keeps its row up to date (its viewers, its title).
    let open_id = live.get_open_id().to_string();
    if let Some(row) = stories.iter().chain(chats.iter()).find(|row| row.id.as_str() == open_id) {
        live.set_open(row.clone());
    }
    // The open chat's own, for the bar under its header.
    let open_chat = store::with_slot(slot, |store| store.open).flatten().map(|chat_id| chat_id.to_string());
    let in_chat = open_chat.and_then(|chat_id| stories.iter().chain(chats.iter()).find(|row| row.chat_id.as_str() == chat_id).cloned());
    live.set_in_chat(in_chat.unwrap_or_default());
    live.set_stories(ModelRc::new(VecModel::from(stories)));
    live.set_chats(ModelRc::new(VecModel::from(chats)));
}

fn refresh_if_front(slot: Slot) {
    if slot == super::shown() {
        refresh();
        // The chat lists' LIVE marks follow (store.rs asks chat_is_live) when the batch ends.
        store::with(|store| store.dirty.chats = true);
    }
}

/// Whether a chat of the account meant right now is live: a live story, or a video chat or live
/// stream on (the chat lists' LIVE mark).
pub fn chat_is_live(chat_id: i64) -> bool {
    let slot = super::context();
    FOUND.with(|found| {
        let found = found.borrow();
        let Some(found) = found.get(&slot) else { return false };
        let story = found.stories.get(&chat_id).is_some_and(|(story_id, _)| {
            found.story_calls.get(&(chat_id, *story_id)).and_then(|(call, _)| found.calls.get(call)).is_none_or(|call| call.is_active)
        });
        let call = found.chat_calls.get(&chat_id).and_then(|call| found.calls.get(call)).is_some_and(|call| call.is_active);
        story || call
    })
}

/// Another chat opened, or none: the bar under its header follows.
pub fn chat_changed() {
    refresh();
}

/// The rows of the account's live streams: live stories (the newest first), then the chats' (the
/// largest audience first).
fn rows(slot: Slot, names: &Names) -> (Vec<LiveRow>, Vec<LiveRow>) {
    let (stories, chat_calls, story_calls, calls) = FOUND.with(|found| {
        let found = found.borrow();
        match found.get(&slot) {
            Some(found) => (found.stories.clone(), found.chat_calls.clone(), found.story_calls.clone(), found.calls.clone()),
            None => Default::default(),
        }
    });
    store::with_slot(slot, |store| {
        let mut story_rows: Vec<(i32, LiveRow)> = stories
            .iter()
            .filter_map(|(&chat_id, &(story_id, date))| {
                let chat = store.chats.get(&chat_id)?;
                let call = story_calls.get(&(chat_id, story_id));
                let facts = call.and_then(|(call, _)| calls.get(call));
                if facts.is_some_and(|facts| !facts.is_active) {
                    return None;
                }
                let rtmp = call.map(|(_, rtmp)| *rtmp).or(facts.map(|facts| facts.is_rtmp_stream));
                let mine = chat_id == store.my_id || facts.is_some_and(|facts| facts.can_be_managed);
                let row = row_of(store, chat, names, format!("story:{chat_id}:{story_id}"), LiveKind::Story, facts, rtmp, mine);
                Some((date, row))
            })
            .collect();
        story_rows.sort_by_key(|(date, _)| std::cmp::Reverse(*date));
        let mut chat_rows: Vec<LiveRow> = chat_calls
            .iter()
            .filter_map(|(&chat_id, call)| {
                let chat = store.chats.get(&chat_id)?;
                let facts = calls.get(call);
                // Only what is on: a video chat planned for later is not live.
                if facts.is_none_or(|facts| !facts.is_active) {
                    return None;
                }
                let kind = if matches!(chat.kind, ChatType::Supergroup { is_channel: true, .. }) { LiveKind::Stream } else { LiveKind::VideoChat };
                let mine = facts.is_some_and(|facts| facts.can_be_managed);
                Some(row_of(store, chat, names, format!("chat:{chat_id}"), kind, facts, facts.map(|facts| facts.is_rtmp_stream), mine))
            })
            .collect();
        chat_rows.sort_by(|a, b| b.viewers.cmp(&a.viewers).then_with(|| a.name.cmp(&b.name)));
        (story_rows.into_iter().map(|(_, row)| row).collect(), chat_rows)
    })
    .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn row_of(
    store: &store::Store,
    chat: &api::Chat,
    names: &Names,
    id: String,
    kind: LiveKind,
    facts: Option<&api::GroupCall>,
    rtmp: Option<bool>,
    mine: bool,
) -> LiveRow {
    let face = store.chat_row(chat, api::ChatList::Main, names);
    // Until TDLib has said what the stream is, it is listed as one that plays.
    let playable = rtmp.is_none_or(|rtmp| rtmp || !NOT_RTMP_IS_PHONE) || mine;
    LiveRow {
        id: id.into(),
        chat_id: chat.id.to_string().into(),
        kind,
        name: if chat.id == store.my_id { store.user_name(store.my_id, names).into() } else { face.title.clone() },
        title: facts.map(|facts| facts.title.clone()).unwrap_or_default().into(),
        initial: face.initial,
        picture: face.picture,
        has_picture: face.has_picture,
        chat_kind: face.kind,
        viewers: facts.map_or(-1, |facts| facts.participant_count),
        playable,
        mine,
    }
}

// ---- the view ---------------------------------------------------------------------------------------

/// The view opens or closes. Open, it loads what is not known yet and asks the calls listed how
/// many watch; closed, it leaves the stream and lets the open chat be seen again.
fn show(shown: bool) {
    let was = SHOWN.replace(shown);
    with_live(|live| live.set_shown(shown));
    if shown {
        load_active_stories();
        let slot = super::shown();
        let calls: Vec<i32> = FOUND.with(|found| found.borrow().get(&slot).map(|found| found.calls.keys().copied().collect()).unwrap_or_default());
        for call in calls {
            ask_call(slot, call);
        }
        refresh();
    } else if was {
        close(true);
        with_live(|live| live.set_going_live(false));
        super::conversation::window_came_to_front();
    }
}

/// Whether the view covers the open chat: what arrives there is not seen meanwhile.
pub fn covers_chat() -> bool {
    SHOWN.get()
}

/// The account in front is about to be another (or is gone): its stream is left.
pub fn forget() {
    close(true);
    with_live(|live| live.set_going_live(false));
}

/// A log out: what was found of the account goes.
pub fn forget_account() {
    let slot = super::context();
    FOUND.with(|found| found.borrow_mut().remove(&slot));
    if WATCHING.with(|watching| watching.borrow().as_ref().is_some_and(|w| w.slot == slot)) {
        close(false);
    }
}

// ---- watching -------------------------------------------------------------------------------------

/// Open the stream of the row `id`: "story:<chat>:<story>" or "chat:<chat>".
fn watch(id: &str) {
    close(true);
    let slot = super::shown();
    let Some((chat_id, kind)) = parse_id(id) else { return };
    let generation = next_generation();
    let names = super::window().map(|ui| Names::from(&ui));
    let row = names.as_ref().and_then(|names| {
        let (stories, chats) = rows(slot, names);
        stories.into_iter().chain(chats).find(|row| row.id.as_str() == id)
    });
    let row = match (row, names) {
        (Some(row), _) => row,
        // A live story opened from a link, or a stream TDLib has not described yet: not in the list.
        (None, Some(names)) => {
            let made = store::with_slot(slot, |store| {
                store.chats.get(&chat_id).map(|chat| {
                    let live_kind = match kind {
                        Kind::Story { .. } => LiveKind::Story,
                        Kind::Call if matches!(chat.kind, ChatType::Supergroup { is_channel: true, .. }) => LiveKind::Stream,
                        Kind::Call => LiveKind::VideoChat,
                    };
                    row_of(store, chat, &names, id.to_string(), live_kind, None, None, chat_id == store.my_id)
                })
            });
            match made.flatten() {
                Some(row) => row,
                None => return,
            }
        }
        (None, None) => return,
    };
    let hosting = row.mine;
    let link = link_of(slot, chat_id, kind);
    let since = match kind {
        Kind::Story { .. } => FOUND.with(|found| found.borrow().get(&slot).and_then(|found| found.stories.get(&chat_id)).map(|(_, date)| *date)),
        Kind::Call => None,
    }
    .map(time::clock)
    .unwrap_or_default();
    with_live(|live| {
        live.set_open_id(id.into());
        live.set_open(row);
        live.set_state(LiveState::Joining);
        live.set_problem(SharedString::new());
        live.set_viewers(0);
        live.set_since(since.into());
        live.set_link(link.into());
        live.set_hosting(hosting);
        live.set_ending(false);
        live.set_stars_total(0);
        live.set_donors(ModelRc::default());
        live.set_comments(ModelRc::default());
        live.set_comments_on(false);
        live.set_can_comment(false);
        live.set_paid_from(0);
        live.set_going_live(false);
    });
    WATCHING.with(|watching| {
        *watching.borrow_mut() = Some(Watching {
            slot,
            generation,
            kind,
            call: 0,
            joined: false,
            hosting,
            rejoined: false,
            feed: None,
            comments: Vec::new(),
            failures: 0,
        })
    });
    // Its group call: known from the list, or asked for.
    let known = FOUND.with(|found| {
        let found = found.borrow();
        let found = found.get(&slot)?;
        match kind {
            Kind::Story { story_id } => found.story_calls.get(&(chat_id, story_id)).map(|(call, _)| *call),
            Kind::Call => found.chat_calls.get(&chat_id).copied(),
        }
    });
    match (known, kind) {
        (Some(call), _) => call_known(generation, call),
        (None, Kind::Story { story_id }) => {
            super::in_context(slot, || {
                let request = json!({ "@type": "getStory", "story_poster_chat_id": chat_id, "story_id": story_id, "only_local": false });
                send(request, move |answer| match answer.map(serde_json::from_value::<api::Story>) {
                    Ok(Ok(story)) => match story.content {
                        api::StoryContent::Live { group_call_id, is_rtmp_stream } => {
                            with_found(slot, |found| found.story_calls.insert((chat_id, story_id), (group_call_id, is_rtmp_stream)));
                            call_known(generation, group_call_id);
                        }
                        api::StoryContent::Other => set_state(generation, LiveState::Ended),
                    },
                    Ok(Err(err)) => failed(generation, &err.to_string()),
                    Err(Error::Telegram { message, .. }) => failed(generation, &message),
                    Err(Error::Stopped) => {}
                });
            });
        }
        (None, Kind::Call) => set_state(generation, LiveState::Ended),
    }
}

fn parse_id(id: &str) -> Option<(i64, Kind)> {
    let mut parts = id.split(':');
    match (parts.next()?, parts.next()?.parse().ok()?, parts.next()) {
        ("story", chat_id, Some(story)) => Some((chat_id, Kind::Story { story_id: story.parse().ok()? })),
        ("chat", chat_id, None) => Some((chat_id, Kind::Call)),
        _ => None,
    }
}

/// The t.me link of a live stream, for a chat with a username: its live story, or its live stream.
fn link_of(slot: Slot, chat_id: i64, kind: Kind) -> String {
    let username = store::with_slot(slot, |store| {
        let chat = store.chats.get(&chat_id)?;
        let names = match chat.kind {
            ChatType::Private { user_id } => store.users.get(&user_id)?.usernames.as_ref(),
            ChatType::Supergroup { supergroup_id, .. } => store.supergroups.get(&supergroup_id)?.usernames.as_ref(),
            _ => None,
        }?;
        names.active_usernames.first().cloned()
    })
    .flatten();
    match (username, kind) {
        (Some(username), Kind::Story { .. }) => format!("https://t.me/{username}/s/live"),
        (Some(username), Kind::Call) => format!("https://t.me/{username}?livestream"),
        (None, _) => String::new(),
    }
}

/// The stream's group call is known: what it is decides whether it can be joined.
fn call_known(generation: u64, call: i32) {
    let Some(slot) = with_watching(generation, |watching| {
        watching.call = call;
        watching.slot
    }) else {
        return;
    };
    super::in_context(slot, || {
        send(json!({ "@type": "getGroupCall", "group_call_id": call }), move |answer| match answer.map(serde_json::from_value::<api::GroupCall>) {
            Ok(Ok(group_call)) => {
                with_found(slot, |found| found.calls.insert(group_call.id, group_call.clone()));
                if !is_current(generation) {
                    return;
                }
                show_call(&group_call);
                if !group_call.is_active {
                    set_state(generation, LiveState::Ended);
                } else if !group_call.is_rtmp_stream && NOT_RTMP_IS_PHONE {
                    set_state(generation, LiveState::Phone);
                } else {
                    join(generation, false);
                }
            }
            Ok(Err(err)) => failed(generation, &err.to_string()),
            Err(Error::Telegram { message, .. }) => failed(generation, &message),
            Err(Error::Stopped) => {}
        });
    });
}

/// What the page shows of the open stream's call: how many watch, its comments.
fn show_call(call: &api::GroupCall) {
    with_live(|live| {
        live.set_viewers(call.participant_count);
        live.set_comments_on(call.are_messages_allowed);
        live.set_can_comment(call.can_send_messages);
        live.set_paid_from(i32::try_from(call.paid_message_star_count).unwrap_or(i32::MAX));
        let mut open = live.get_open();
        if open.viewers != call.participant_count || open.title.as_str() != call.title {
            open.viewers = call.participant_count;
            open.title = call.title.clone().into();
            live.set_open(open);
        }
    });
}

/// Join the stream's call as a viewer: muted, no video, a payload without media (as Telegram's web
/// app joins an RTMP stream). The answer says whether Telegram serves it as a stream.
fn join(generation: u64, again: bool) {
    let Some((slot, call, kind)) = with_watching(generation, |watching| (watching.slot, watching.call, watching.kind)) else { return };
    let ssrc = random_ssrc();
    let payload = json!({ "fingerprints": [], "pwd": "", "ssrc": ssrc, "ssrc-groups": [], "ufrag": "" }).to_string();
    let parameters = json!({
        "@type": "groupCallJoinParameters", "audio_source_id": ssrc, "payload": payload, "is_muted": true, "is_my_video_enabled": false,
    });
    let request = match kind {
        Kind::Story { .. } => json!({ "@type": "joinLiveStory", "group_call_id": call, "join_parameters": parameters }),
        Kind::Call => json!({
            "@type": "joinVideoChat", "group_call_id": call, "participant_id": null, "join_parameters": parameters, "invite_hash": "",
        }),
    };
    super::in_context(slot, || {
        send(request, move |answer| {
            let answer = match answer {
                Ok(answer) => answer,
                Err(Error::Telegram { message, .. }) => return failed(generation, &message),
                Err(Error::Stopped) => return,
            };
            if !is_current(generation) {
                // Closed meanwhile: the join is left at once.
                super::in_context(slot, || leave_call(call));
                return;
            }
            let response: Value = answer.get("text").and_then(Value::as_str).and_then(|text| serde_json::from_str(text).ok()).unwrap_or_default();
            let stream = response.get("stream").and_then(Value::as_bool).unwrap_or(false);
            with_watching(generation, |watching| watching.joined = true);
            if !stream {
                // Served over WebRTC: not for FinchGram. Leave, and say so.
                with_watching(generation, |watching| watching.joined = false);
                super::in_context(slot, || leave_call(call));
                set_state(generation, LiveState::Phone);
                return;
            }
            if !again && with_watching(generation, |watching| watching.hosting).unwrap_or(false) {
                load_donors(generation);
            }
            look_for_stream(generation);
        });
    });
}

/// Joined again after Telegram dropped us (need_rejoin, GROUPCALL_JOIN_MISSING): once.
fn rejoin(generation: u64) {
    let first = with_watching(generation, |watching| !std::mem::replace(&mut watching.rejoined, true)).unwrap_or(false);
    if first {
        join(generation, true);
    } else {
        failed(generation, "GROUPCALL_JOIN_MISSING");
    }
}

/// Where the stream is: getGroupCallStreams. Nothing yet (the broadcaster's app is not sending)
/// is looked at again in a moment; a stream starts the feed, or moves it to the edge.
fn look_for_stream(generation: u64) {
    let Some((slot, call)) = with_watching(generation, |watching| (watching.slot, watching.call)) else { return };
    super::in_context(slot, || {
        send(json!({ "@type": "getGroupCallStreams", "group_call_id": call }), move |answer| {
            match answer.map(serde_json::from_value::<api::GroupCallStreams>) {
                Ok(Ok(streams)) => {
                    // The unified channel of an RTMP stream (sound and picture in one) is 1.
                    let stream = streams.streams.iter().find(|stream| stream.channel_id == 1).or_else(|| streams.streams.first()).cloned();
                    match stream {
                        Some(stream) => stream_found(generation, &stream),
                        None => {
                            if with_watching(generation, |watching| watching.feed.is_none()).unwrap_or(false) {
                                set_state(generation, LiveState::Waiting);
                            }
                            slint::Timer::single_shot(LOOK_AGAIN, move || look_for_stream(generation));
                        }
                    }
                }
                Ok(Err(err)) => failed(generation, &err.to_string()),
                Err(Error::Telegram { message, .. }) if is_join_missing(&message) => rejoin(generation),
                Err(Error::Telegram { message, .. }) => fetch_failed(generation, &message, move || look_for_stream(generation)),
                Err(Error::Stopped) => {}
            }
        });
    });
}

/// The stream is there: the feed starts a little behind its edge, or a feed that fell behind (or
/// was told its time is wrong) moves on to it.
fn stream_found(generation: u64, stream: &api::GroupCallStream) {
    let chunk_ms = match chunk_ms(stream.scale) {
        Some(chunk_ms) => chunk_ms,
        None => return failed(generation, "a stream scale FinchGram does not know"),
    };
    let start = (stream.time_offset.div_euclid(chunk_ms) * chunk_ms - BEHIND_EDGE_MS).max(0);
    let (volume, muted) = with_live_get(|live| (live.get_volume(), live.get_muted()));
    let started = with_watching(generation, |watching| match &mut watching.feed {
        Some(feed) => {
            feed.edge = stream.time_offset;
            feed.edge_seen = Instant::now();
            if feed.next < start || feed.channel != stream.channel_id || feed.scale != stream.scale {
                feed.next = start.max(feed.next);
                feed.channel = stream.channel_id;
                feed.scale = stream.scale;
                feed.chunk_ms = chunk_ms;
                // A stream not begun yet begins where the feed now starts.
                if !feed.pushed {
                    feed.first = feed.next;
                }
            }
            None
        }
        None => {
            let pipe = Pipe::new();
            watching.feed = Some(Feed {
                channel: stream.channel_id,
                scale: stream.scale,
                chunk_ms,
                next: start,
                first: start,
                edge: stream.time_offset,
                edge_seen: Instant::now(),
                remuxer: Remuxer::new(),
                pipe: pipe.clone(),
                asking: false,
                pushed: false,
            });
            Some(pipe)
        }
    })
    .flatten();
    if let Some(pipe) = started {
        crate::player::live::set_volume(volume);
        crate::player::live::play(pipe, volume, muted);
    }
    fetch_next(generation);
}

/// How long a segment is at `scale`: 1000 ms at 0, halved for each step up (Telegram uses a few
/// values around 0).
fn chunk_ms(scale: i32) -> Option<i64> {
    match scale {
        0..=3 => Some(1000 >> scale),
        -3..=-1 => Some(1000 << -scale),
        _ => None,
    }
}

/// Ask for the next segment, once it exists (by the edge last seen, moving on with the clock).
fn fetch_next(generation: u64) {
    let wait = with_watching(generation, |watching| {
        let feed = watching.feed.as_mut()?;
        if feed.asking {
            return None;
        }
        // The edge moves on with the clock; a pause of the broadcaster's is found by TIME_TOO_BIG.
        let edge_now = feed.edge + feed.edge_seen.elapsed().as_millis() as i64;
        let ready_in = feed.next + feed.chunk_ms - edge_now;
        Some(if ready_in > 0 { Duration::from_millis(ready_in as u64 + 50) } else { Duration::ZERO })
    })
    .flatten();
    match wait {
        Some(wait) if wait > Duration::ZERO => slint::Timer::single_shot(wait, move || request_segment(generation)),
        Some(_) => request_segment(generation),
        None => {}
    }
}

fn request_segment(generation: u64) {
    let Some((slot, call, time, scale, channel)) = with_watching(generation, |watching| {
        let call = watching.call;
        let slot = watching.slot;
        let feed = watching.feed.as_mut()?;
        if feed.asking {
            return None;
        }
        feed.asking = true;
        Some((slot, call, feed.next, feed.scale, feed.channel))
    })
    .flatten() else {
        return;
    };
    let request = json!({
        "@type": "getGroupCallStreamSegment", "group_call_id": call, "time_offset": time, "scale": scale, "channel_id": channel,
        "video_quality": { "@type": "groupCallVideoQualityFull" },
    });
    super::in_context(slot, || {
        send(request, move |answer| {
            with_watching(generation, |watching| {
                if let Some(feed) = &mut watching.feed {
                    feed.asking = false;
                }
            });
            if !is_current(generation) {
                return;
            }
            match answer {
                Ok(answer) => {
                    let bytes = answer
                        .get("data")
                        .and_then(Value::as_str)
                        .and_then(|data| base64::engine::general_purpose::STANDARD.decode(data).ok())
                        .unwrap_or_default();
                    segment_arrived(generation, time, &bytes);
                }
                Err(Error::Telegram { message, .. }) => segment_refused(generation, &message),
                Err(Error::Stopped) => {}
            }
        });
    });
}

/// What became of a segment.
enum Taken {
    /// Into the stream.
    Pushed,
    /// Nothing to play in it (an empty second, one already had, one that cannot be read).
    Skipped,
    /// The pipe took no more: mpv is far behind.
    Refused,
    /// Another resolution or codec than the stream began with.
    Changed,
}

/// A segment came: into the stream, and on to the next.
fn segment_arrived(generation: u64, time: i64, bytes: &[u8]) {
    let taken = with_watching(generation, |watching| {
        let feed = watching.feed.as_mut()?;
        if time != feed.next {
            return Some(Taken::Skipped);
        }
        feed.next += feed.chunk_ms;
        if bytes.is_empty() {
            // A second without anything in it: the broadcaster paused.
            return Some(Taken::Skipped);
        }
        let file = match segment_file(bytes) {
            Ok(file) => file,
            Err(err) => {
                eprintln!("live: a segment FinchGram cannot read: {err}");
                return Some(Taken::Skipped);
            }
        };
        Some(match feed.remuxer.push(file, time - feed.first) {
            Ok(stream_bytes) => {
                if feed.pipe.push(stream_bytes) {
                    feed.pushed = true;
                    Taken::Pushed
                } else {
                    Taken::Refused
                }
            }
            Err(RemuxError::Changed) => Taken::Changed,
            Err(RemuxError::Unreadable(why)) => {
                eprintln!("live: a segment FinchGram cannot read: {why}");
                Taken::Skipped
            }
        })
    })
    .flatten();
    match taken {
        Some(Taken::Pushed) => {
            with_watching(generation, |watching| watching.failures = 0);
            if with_live_get(|live| live.get_state()) != LiveState::Playing {
                set_state(generation, LiveState::Playing);
            }
        }
        Some(Taken::Skipped) => {}
        // The broadcaster changed OBS's settings, or the player fell far behind: the stream starts
        // again with a new header, from the next second (the edge for one far behind).
        Some(Taken::Changed) => restart_feed(generation),
        Some(Taken::Refused) => {
            restart_feed(generation);
            look_for_stream(generation);
            return;
        }
        None => return,
    }
    fetch_next(generation);
}

/// The feed starts over: a new pipe and remuxer, played anew, from where it is.
fn restart_feed(generation: u64) {
    let (volume, muted) = with_live_get(|live| (live.get_volume(), live.get_muted()));
    let pipe = with_watching(generation, |watching| {
        let feed = watching.feed.as_mut()?;
        feed.pipe.cancel();
        let pipe = Pipe::new();
        feed.pipe = pipe.clone();
        feed.remuxer = Remuxer::new();
        feed.first = feed.next;
        feed.pushed = false;
        Some(pipe)
    })
    .flatten();
    if let Some(pipe) = pipe {
        crate::player::live::play(pipe, volume, muted);
    }
}

/// Telegram refused a segment: not there yet, our time is off, we are not in the call any more,
/// or something else (tried again a few times).
fn segment_refused(generation: u64, message: &str) {
    match message {
        "TIME_TOO_BIG" => slint::Timer::single_shot(NOT_YET, move || look_for_stream(generation)),
        "TIME_TOO_SMALL" | "TIME_INVALID" | "TIME_RANGE_INVALID" => look_for_stream(generation),
        message if is_join_missing(message) => rejoin(generation),
        message if message.starts_with("FLOOD_WAIT") => {
            let seconds = errors::wait_seconds(message).unwrap_or(1).clamp(1, 30) as u64;
            slint::Timer::single_shot(Duration::from_secs(seconds), move || fetch_next(generation));
        }
        "GROUPCALL_INVALID" | "GROUPCALL_NOT_MODIFIED" => stream_ended(generation),
        message => fetch_failed(generation, message, move || fetch_next(generation)),
    }
}

fn is_join_missing(message: &str) -> bool {
    matches!(message, "GROUPCALL_JOIN_MISSING" | "GROUPCALL_FORBIDDEN")
}

/// A failure that may pass: `again` after a moment, until too many in a row.
fn fetch_failed(generation: u64, message: &str, again: impl FnOnce() + 'static) {
    let failures = with_watching(generation, |watching| {
        watching.failures += 1;
        watching.failures
    })
    .unwrap_or(FAILURES_ALLOWED + 1);
    eprintln!("live: {message} ({failures})");
    if failures > FAILURES_ALLOWED {
        failed(generation, message);
    } else {
        slint::Timer::single_shot(RETRY, again);
    }
}

/// The MP4 inside a segment: after Telegram's header (TL, little-endian): its signature
/// 0xa12e810d, the container's name, the active mask, and the events (each an offset, an
/// endpoint, a rotation and one more number).
fn segment_file(bytes: &[u8]) -> Result<&[u8], String> {
    let mut at = 0usize;
    let int = |at: &mut usize| -> Result<i32, String> {
        let value = bytes.get(*at..*at + 4).ok_or("a segment cut short")?;
        *at += 4;
        Ok(i32::from_le_bytes(value.try_into().unwrap_or_default()))
    };
    let string = |at: &mut usize| -> Result<Vec<u8>, String> {
        let first = *bytes.get(*at).ok_or("a segment cut short")?;
        let (length, header) = if first < 254 {
            (usize::from(first), 1)
        } else {
            let three = bytes.get(*at + 1..*at + 4).ok_or("a segment cut short")?;
            (usize::from(three[0]) | usize::from(three[1]) << 8 | usize::from(three[2]) << 16, 4)
        };
        let value = bytes.get(*at + header..*at + header + length).ok_or("a segment cut short")?.to_vec();
        // Padded to four bytes, the length's own byte(s) included.
        *at += (header + length).div_ceil(4) * 4;
        Ok(value)
    };
    if int(&mut at)? as u32 != 0xa12e_810d {
        return Err("not a stream segment".into());
    }
    let container = string(&mut at)?;
    int(&mut at)?; // the active mask
    let events = int(&mut at)?;
    if !(0..=1000).contains(&events) {
        return Err("a segment with a count of events past belief".into());
    }
    for _ in 0..events {
        int(&mut at)?;
        string(&mut at)?;
        int(&mut at)?;
        int(&mut at)?;
    }
    if container != b"mp4" {
        return Err(format!("a segment in {}", String::from_utf8_lossy(&container)));
    }
    Ok(&bytes[at.min(bytes.len())..])
}

/// The live player came to an end: of the stream (the pipe ended), or because mpv could not play it.
fn player_ended(failed_to_play: bool) {
    let generation = GENERATION.get();
    if !is_current(generation) {
        return;
    }
    if failed_to_play {
        failed(generation, "");
    } else if with_live_get(|live| live.get_state()) == LiveState::Playing {
        set_state(generation, LiveState::Ended);
    }
}

/// The stream is over: what came is played to its end, and nothing more is asked for.
fn stream_ended(generation: u64) {
    let left = with_watching(generation, |watching| {
        if let Some(feed) = watching.feed.take() {
            feed.pipe.end();
        }
        let joined = std::mem::replace(&mut watching.joined, false);
        (watching.slot, watching.call, joined)
    });
    if let Some((slot, call, true)) = left {
        super::in_context(slot, || leave_call(call));
    }
    set_state(generation, LiveState::Ended);
}

fn failed(generation: u64, message: &str) {
    if !is_current(generation) {
        return;
    }
    let explained = if message.is_empty() { String::new() } else { errors::explain(message) };
    with_live(|live| live.set_problem(explained.into()));
    stop_feed(generation);
    set_state(generation, LiveState::Failed);
}

/// The feed stops, and the player with it; the call is still joined (for its comments).
fn stop_feed(generation: u64) {
    let stopped = with_watching(generation, |watching| watching.feed.take().is_some()).unwrap_or(false);
    if stopped {
        crate::player::live::stop();
    }
}

fn set_state(generation: u64, state: LiveState) {
    if is_current(generation) {
        with_live(|live| live.set_state(state));
    }
}

/// Close the stream open: the player stops, the call is left (the user's own stream goes on).
fn close(clear_page: bool) {
    next_generation();
    let left = WATCHING.with(|watching| watching.borrow_mut().take());
    crate::player::live::stop();
    if let Some(watching) = left {
        if let Some(feed) = &watching.feed {
            feed.pipe.cancel();
        }
        if watching.joined {
            super::in_context(watching.slot, || leave_call(watching.call));
        }
    }
    if clear_page {
        with_live(|live| {
            live.set_open_id(SharedString::new());
            live.set_state(LiveState::Idle);
            live.set_comments(ModelRc::default());
            live.set_hosting(false);
            live.set_ending(false);
            live.set_problem(SharedString::new());
        });
    }
}

fn leave_call(call: i32) {
    send(json!({ "@type": "leaveGroupCall", "group_call_id": call }), |answer| {
        if let Err(Error::Telegram { message, .. }) = answer {
            eprintln!("live: cannot leave the group call: {message}");
        }
    });
}

// ---- comments -------------------------------------------------------------------------------------

/// A comment in a group call: into the column, when it is the open stream's.
pub fn comment(call: i32, message: &api::GroupCallMessage) {
    let slot = super::context();
    let added = WATCHING.with(|watching| {
        let mut watching = watching.borrow_mut();
        let watching = watching.as_mut().filter(|w| w.slot == slot && w.call == call)?;
        if watching.comments.iter().any(|kept| kept.id == message.message_id) {
            return None;
        }
        watching.comments.push(Comment {
            id: message.message_id,
            sender: message.sender_id,
            text: message.text.text.clone(),
            stars: message.paid_message_star_count,
            host: message.is_from_owner,
            deletable: message.can_be_deleted,
        });
        let excess = watching.comments.len().saturating_sub(COMMENTS_KEPT);
        watching.comments.drain(..excess);
        Some(())
    });
    if added.is_some() {
        show_comments();
    }
}

pub fn comments_deleted(call: i32, message_ids: &[i32]) {
    let slot = super::context();
    let removed = WATCHING.with(|watching| {
        let mut watching = watching.borrow_mut();
        let watching = watching.as_mut().filter(|w| w.slot == slot && w.call == call)?;
        let before = watching.comments.len();
        watching.comments.retain(|comment| !message_ids.contains(&comment.id));
        (watching.comments.len() != before).then_some(())
    });
    if removed.is_some() {
        show_comments();
    }
}

/// A comment of ours did not go: Telegram's reason, for a moment.
pub fn comment_failed(call: i32, error: &api::TdError) {
    let open = WATCHING.with(|watching| watching.borrow().as_ref().is_some_and(|w| w.call == call));
    if open {
        actions::show_failure(&errors::explain(&error.message));
    }
}

/// How many Stars make each level of paid comments.
pub fn levels(levels: &[api::GroupCallMessageLevel]) {
    let mut minimums: Vec<i64> = levels.iter().map(|level| level.min_star_count).collect();
    minimums.sort_unstable();
    LEVELS.with(|kept| *kept.borrow_mut() = minimums);
    show_comments();
}

/// The colour level of a paid comment: 1 … 3, the highest 3; 0 for a free one.
fn tier(stars: i64) -> i32 {
    if stars <= 0 {
        return 0;
    }
    LEVELS.with(|levels| {
        let levels = levels.borrow();
        if levels.is_empty() {
            return 1;
        }
        let reached = levels.iter().filter(|minimum| stars >= **minimum).count().max(1);
        (reached * 3).div_ceil(levels.len()).clamp(1, 3) as i32
    })
}

fn show_comments() {
    let Some(ui) = super::window() else { return };
    let names = Names::from(&ui);
    let rows: Vec<LiveComment> = WATCHING.with(|watching| {
        let watching = watching.borrow();
        let Some(watching) = watching.as_ref() else { return Vec::new() };
        store::with_slot(watching.slot, |store| {
            watching
                .comments
                .iter()
                .map(|comment| {
                    let name = store.sender_name(&comment.sender, &names);
                    let mine = matches!(comment.sender, MessageSender::User { user_id } if user_id == store.my_id);
                    LiveComment {
                        id: comment.id,
                        initial: store::initial(&name),
                        name: name.into(),
                        text: comment.text.clone().into(),
                        stars: i32::try_from(comment.stars).unwrap_or(i32::MAX),
                        tier: tier(comment.stars),
                        host: comment.host,
                        mine,
                        deletable: comment.deletable,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
    });
    ui.global::<Live>().set_comments(ModelRc::new(VecModel::from(rows)));
}

/// A free comment (the host's are always free; paid ones go from Telegram's own apps for now).
fn send_comment(text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let Some((slot, call)) = WATCHING.with(|watching| watching.borrow().as_ref().filter(|w| w.joined).map(|w| (w.slot, w.call))) else { return };
    let request = json!({
        "@type": "sendGroupCallMessage", "group_call_id": call,
        "text": { "@type": "formattedText", "text": text, "entities": [] }, "paid_message_star_count": 0,
    });
    super::in_context(slot, || {
        send(request, |answer| {
            if let Err(Error::Telegram { message, .. }) = answer {
                actions::show_failure(&errors::explain(&message));
            }
        });
    });
}

fn delete_comment(id: i32) {
    let Some((slot, call)) = WATCHING.with(|watching| watching.borrow().as_ref().filter(|w| w.hosting).map(|w| (w.slot, w.call))) else { return };
    let request = json!({ "@type": "deleteGroupCallMessages", "group_call_id": call, "message_ids": [id], "report_spam": false });
    super::in_context(slot, || {
        send(request, |answer| {
            if let Err(Error::Telegram { message, .. }) = answer {
                actions::show_failure(&errors::explain(&message));
            }
        });
    });
}

fn toggle_comments() {
    let Some((slot, call)) = WATCHING.with(|watching| watching.borrow().as_ref().filter(|w| w.hosting).map(|w| (w.slot, w.call))) else { return };
    let allowed = !with_live_get(|live| live.get_comments_on());
    with_live(|live| live.set_comments_on(allowed));
    let request = json!({ "@type": "toggleGroupCallAreMessagesAllowed", "group_call_id": call, "are_messages_allowed": allowed });
    super::in_context(slot, || {
        send(request, move |answer| {
            if let Err(Error::Telegram { message, .. }) = answer {
                with_live(|live| live.set_comments_on(!allowed));
                actions::show_failure(&errors::explain(&message));
            }
        });
    });
}

// ---- the host ---------------------------------------------------------------------------------------

fn load_donors(generation: u64) {
    let Some((slot, call, Kind::Story { .. })) = with_watching(generation, |watching| (watching.slot, watching.call, watching.kind)) else { return };
    super::in_context(slot, || {
        send(json!({ "@type": "getLiveStoryTopDonors", "group_call_id": call }), move |answer| {
            if let Ok(Ok(donors)) = answer.map(serde_json::from_value::<api::LiveStoryDonors>) {
                super::in_context(slot, || self::donors(call, &donors));
            }
        });
    });
}

/// The top supporters of a live story and its Stars, for its host.
pub fn donors(call: i32, donors: &api::LiveStoryDonors) {
    let hosting = WATCHING.with(|watching| watching.borrow().as_ref().is_some_and(|w| w.call == call && w.hosting));
    if !hosting {
        return;
    }
    let Some(ui) = super::window() else { return };
    let names = Names::from(&ui);
    let rows: Vec<LiveDonor> = store::with(|store| {
        donors
            .top_donors
            .iter()
            .take(3)
            .map(|donor| LiveDonor {
                name: match (&donor.sender_id, donor.is_anonymous) {
                    (Some(sender), false) => store.sender_name(sender, &names).into(),
                    _ => "…".into(),
                },
                stars: i32::try_from(donor.star_count).unwrap_or(i32::MAX),
                tier: tier(donor.star_count),
            })
            .collect()
    })
    .unwrap_or_default();
    let live = ui.global::<Live>();
    live.set_stars_total(i32::try_from(donors.total_star_count).unwrap_or(i32::MAX));
    live.set_donors(ModelRc::new(VecModel::from(rows)));
}

/// End the user's own stream, for everyone.
fn end_live() {
    with_live(|live| live.set_ending(false));
    let Some((slot, call, generation)) =
        WATCHING.with(|watching| watching.borrow().as_ref().filter(|w| w.hosting && w.call != 0).map(|w| (w.slot, w.call, w.generation)))
    else {
        return;
    };
    super::in_context(slot, || {
        send(json!({ "@type": "endGroupCall", "group_call_id": call }), move |answer| match answer {
            Ok(_) => stream_ended(generation),
            Err(Error::Telegram { message, .. }) => actions::show_failure(&errors::explain(&message)),
            Err(Error::Stopped) => {}
        });
    });
}

// ---- going live -------------------------------------------------------------------------------------

/// The form: where to go live (the user's own live story, or a channel they may stream in), and
/// the RTMP address and key of the first.
fn open_go_live() {
    close(true);
    let Some(ui) = super::window() else { return };
    let names = Names::from(&ui);
    let targets: Vec<(LiveTarget, (i64, bool))> = store::with(|store| {
        let mut targets = Vec::new();
        // The user's own live story goes out from the chat with oneself (Saved Messages), whose id
        // is the user's.
        if store.my_id != 0 {
            let name = store.user_name(store.my_id, &names);
            let (picture, has_picture) = store.users.get(&store.my_id).map(|me| store.user_avatar(me)).unwrap_or_else(|| (store.blank_image(), false));
            targets.push((
                LiveTarget { chat_id: store.my_id.to_string().into(), initial: store::initial(&name), name: name.into(), picture, has_picture, story: true },
                (store.my_id, true),
            ));
        }
        let mut channels: Vec<&api::Chat> = store
            .chats
            .values()
            .filter(|chat| matches!(chat.kind, ChatType::Supergroup { is_channel: true, .. }) && may_stream_in(store, chat))
            .collect();
        channels.sort_by_key(|chat| chat.title.to_lowercase());
        for chat in channels {
            let face = store.chat_row(chat, api::ChatList::Main, &names);
            targets.push((
                LiveTarget { chat_id: chat.id.to_string().into(), name: face.title.clone(), initial: face.initial, picture: face.picture, has_picture: face.has_picture, story: false },
                (chat.id, false),
            ));
        }
        targets
    })
    .unwrap_or_default();
    TARGETS.with(|kept| *kept.borrow_mut() = targets.iter().map(|(_, target)| *target).collect());
    let live = ui.global::<Live>();
    live.set_targets(ModelRc::new(VecModel::from(targets.into_iter().map(|(row, _)| row).collect::<Vec<_>>())));
    live.set_target(0);
    live.set_privacy(0);
    live.set_allow_comments(true);
    live.set_paid_minimum(SharedString::new());
    live.set_stream_title(SharedString::new());
    live.set_server(SharedString::new());
    live.set_key(SharedString::new());
    live.set_key_shown(false);
    live.set_starting(false);
    live.set_start_problem(SharedString::new());
    live.set_going_live(true);
    fetch_rtmp_url(false);
}

/// Whether the user may start a live stream in the channel: its owner, or an administrator who
/// manages its video chats.
fn may_stream_in(store: &store::Store, chat: &api::Chat) -> bool {
    let ChatType::Supergroup { supergroup_id, .. } = chat.kind else { return false };
    match store.supergroups.get(&supergroup_id).map(|group| &group.status) {
        Some(api::ChatMemberStatus::Creator) => true,
        Some(api::ChatMemberStatus::Administrator { rights, .. }) => rights.can_manage_video_chats,
        _ => false,
    }
}

/// The chosen target's RTMP address and key; `replace`: a new key, the old one no longer works.
fn fetch_rtmp_url(replace: bool) {
    let index = with_live_get(|live| live.get_target());
    let Some((chat_id, story)) = TARGETS.with(|kept| usize::try_from(index).ok().and_then(|index| kept.borrow().get(index).copied())) else { return };
    let function = match (story, replace) {
        (true, false) => "getLiveStoryRtmpUrl",
        (true, true) => "replaceLiveStoryRtmpUrl",
        (false, false) => "getVideoChatRtmpUrl",
        (false, true) => "replaceVideoChatRtmpUrl",
    };
    send(json!({ "@type": function, "chat_id": chat_id }), move |answer| {
        // The form may have moved on to another target meanwhile.
        if with_live_get(|live| live.get_target()) != index {
            return;
        }
        match answer.map(serde_json::from_value::<api::RtmpUrl>) {
            Ok(Ok(url)) => with_live(|live| {
                live.set_server(url.url.into());
                live.set_key(url.stream_key.into());
                live.set_start_problem(SharedString::new());
            }),
            Ok(Err(err)) => eprintln!("live: cannot read an RTMP address: {err}"),
            Err(Error::Telegram { message, .. }) => with_live(|live| {
                live.set_server(SharedString::new());
                live.set_key(SharedString::new());
                live.set_start_problem(errors::explain(&message).into());
            }),
            Err(Error::Stopped) => {}
        }
    });
}

/// Start the live story, or the channel's live stream, and open it as its host.
fn start_live() {
    let index = with_live_get(|live| live.get_target());
    let Some((chat_id, story)) = TARGETS.with(|kept| usize::try_from(index).ok().and_then(|index| kept.borrow().get(index).copied())) else { return };
    let (privacy, comments, paid, title) = with_live_get(|live| {
        (live.get_privacy(), live.get_allow_comments(), live.get_paid_minimum().trim().parse::<i64>().unwrap_or(0).max(0), live.get_stream_title().trim().to_string())
    });
    with_live(|live| {
        live.set_starting(true);
        live.set_start_problem(SharedString::new());
    });
    let slot = super::shown();
    if story {
        let privacy_settings = match privacy {
            1 => json!({ "@type": "storyPrivacySettingsContacts", "except_user_ids": [] }),
            2 => json!({ "@type": "storyPrivacySettingsCloseFriends" }),
            _ => json!({ "@type": "storyPrivacySettingsEveryone", "except_user_ids": [] }),
        };
        let request = json!({
            "@type": "startLiveStory", "chat_id": chat_id, "privacy_settings": privacy_settings, "protect_content": false,
            "is_rtmp_stream": true, "enable_messages": comments, "paid_message_star_count": if comments { paid } else { 0 },
        });
        send(request, move |answer| {
            with_live(|live| live.set_starting(false));
            match answer.map(serde_json::from_value::<api::StartLiveStoryResult>) {
                Ok(Ok(api::StartLiveStoryResult::Ok { story })) => {
                    if let api::StoryContent::Live { group_call_id, is_rtmp_stream } = story.content {
                        with_found(slot, |found| {
                            found.stories.insert(chat_id, (story.id, story.date));
                            found.story_calls.insert((chat_id, story.id), (group_call_id, is_rtmp_stream));
                        });
                    }
                    with_live(|live| live.set_going_live(false));
                    watch(&format!("story:{chat_id}:{}", story.id));
                }
                // One is on already: it is opened instead.
                Ok(Ok(api::StartLiveStoryResult::Fail { error_type: api::CanPostStoryResult::LiveStoryIsActive { story_id } })) => {
                    with_live(|live| live.set_going_live(false));
                    watch(&format!("story:{chat_id}:{story_id}"));
                }
                Ok(Ok(api::StartLiveStoryResult::Fail { error_type })) => {
                    with_live(|live| {
                        let words = live_words(|words| match error_type {
                            api::CanPostStoryResult::PremiumNeeded => words.invoke_premium_needed(),
                            api::CanPostStoryResult::BoostNeeded => words.invoke_boost_needed(),
                            _ => words.invoke_story_limit(),
                        });
                        live.set_start_problem(words);
                    });
                }
                Ok(Err(err)) => eprintln!("live: cannot read how starting a live story went: {err}"),
                Err(Error::Telegram { message, .. }) => with_live(|live| live.set_start_problem(errors::explain(&message).into())),
                Err(Error::Stopped) => {}
            }
        });
    } else {
        let request = json!({ "@type": "createVideoChat", "chat_id": chat_id, "title": title, "start_date": 0, "is_rtmp_stream": true });
        send(request, move |answer| {
            with_live(|live| live.set_starting(false));
            match answer.map(serde_json::from_value::<api::GroupCallId>) {
                Ok(Ok(call)) => {
                    with_found(slot, |found| found.chat_calls.insert(chat_id, call.id));
                    with_live(|live| live.set_going_live(false));
                    // Known as soon as TDLib has said what the call is.
                    let request = json!({ "@type": "getGroupCall", "group_call_id": call.id });
                    send(request, move |answer| {
                        if let Ok(Ok(group_call)) = answer.map(serde_json::from_value::<api::GroupCall>) {
                            group_call_answered(slot, group_call);
                        }
                        watch(&format!("chat:{chat_id}"));
                    });
                }
                Ok(Err(err)) => eprintln!("live: cannot read a new video chat: {err}"),
                Err(Error::Telegram { message, .. }) => with_live(|live| live.set_start_problem(errors::explain(&message).into())),
                Err(Error::Stopped) => {}
            }
        });
    }
}

// ---- a link -----------------------------------------------------------------------------------------

/// A link pasted: t.me/<username>/s/live opens that live story. False at once for words that
/// are no link at all; a link that leads to no live story says so in the pane.
fn watch_link(text: &str) -> bool {
    let text = text.trim();
    let looks_like_link = text.contains("t.me/") || text.contains("telegram.me/") || text.starts_with("tg:");
    if !looks_like_link {
        return false;
    }
    let link = if text.contains("://") || text.starts_with("tg:") { text.to_string() } else { format!("https://{text}") };
    let slot = super::shown();
    send(json!({ "@type": "getInternalLinkType", "link": link }), move |answer| {
        let username = match answer.map(serde_json::from_value::<api::InternalLinkType>) {
            Ok(Ok(api::InternalLinkType::LiveStory { story_poster_username })) => story_poster_username,
            _ => return no_live_there(),
        };
        super::in_context(slot, || {
            send(json!({ "@type": "searchPublicChat", "username": username }), move |answer| {
                let Some(chat_id) = answer.ok().and_then(|chat| chat.get("id").and_then(Value::as_i64)) else { return no_live_there() };
                super::in_context(slot, || {
                    send(json!({ "@type": "getChatActiveStories", "chat_id": chat_id }), move |answer| {
                        match answer.map(serde_json::from_value::<api::ChatActiveStories>) {
                            Ok(Ok(active)) => match active.stories.iter().find(|story| story.is_live) {
                                Some(story) => {
                                    with_found(slot, |found| found.stories.insert(chat_id, (story.story_id, story.date)));
                                    watch(&format!("story:{chat_id}:{}", story.story_id));
                                }
                                None => no_live_there(),
                            },
                            _ => no_live_there(),
                        }
                    });
                });
            });
        });
    });
    true
}

/// The link leads to no live story now: said in the pane.
fn no_live_there() {
    close(true);
    with_live(|live| {
        let words = live_words(|words| words.invoke_not_live_now());
        live.set_open_id("link".into());
        live.set_open(LiveRow { name: SharedString::new(), kind: LiveKind::Story, viewers: -1, ..Default::default() });
        live.set_problem(words);
        live.set_state(LiveState::Failed);
    });
}

// ---- helpers ----------------------------------------------------------------------------------------

fn next_generation() -> u64 {
    let generation = GENERATION.get() + 1;
    GENERATION.set(generation);
    generation
}

fn is_current(generation: u64) -> bool {
    GENERATION.get() == generation && WATCHING.with(|watching| watching.borrow().as_ref().is_some_and(|w| w.generation == generation))
}

/// Change the stream open, when it is still the one of `generation`.
fn with_watching<R>(generation: u64, change: impl FnOnce(&mut Watching) -> R) -> Option<R> {
    WATCHING.with(|watching| watching.borrow_mut().as_mut().filter(|w| w.generation == generation).map(change))
}

fn with_found<R>(slot: Slot, change: impl FnOnce(&mut Found) -> R) -> R {
    FOUND.with(|found| change(found.borrow_mut().entry(slot).or_default()))
}

fn with_live(change: impl FnOnce(&Live)) {
    super::with_ui(|ui| change(&ui.global::<Live>()));
}

fn with_live_get<R: Default>(read: impl FnOnce(&Live) -> R) -> R {
    super::window().map(|ui| read(&ui.global::<Live>())).unwrap_or_default()
}

fn live_words(words: impl FnOnce(&LiveWords) -> SharedString) -> SharedString {
    super::window().map(|ui| words(&ui.global::<LiveWords>())).unwrap_or_default()
}

fn copy(text: &str, notice: ActionNotice) {
    if text.is_empty() {
        return;
    }
    match crate::platform::copy_text(text) {
        Ok(()) => actions::show_notice(notice),
        Err(err) => eprintln!("live: cannot copy: {err}"),
    }
}

/// A random synchronization source for the join: any 32 bits but 0, as Telegram's apps make one.
fn random_ssrc() -> i32 {
    let mut bytes = [0u8; 4];
    if let Ok(mut random) = std::fs::File::open("/dev/urandom") {
        let _ = std::io::Read::read_exact(&mut random, &mut bytes);
    }
    let ssrc = i32::from_le_bytes(bytes);
    if ssrc == 0 { 1 } else { ssrc }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A segment as Telegram makes one: the header, then `file`.
    fn segment(container: &str, events: &[(&str, i32)], file: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(0xa12e_810d_u32.to_le_bytes());
        let tl_string = |bytes: &mut Vec<u8>, value: &str| {
            let start = bytes.len();
            bytes.push(value.len() as u8);
            bytes.extend(value.as_bytes());
            while !(bytes.len() - start).is_multiple_of(4) {
                bytes.push(0);
            }
        };
        tl_string(&mut bytes, container);
        bytes.extend(1i32.to_le_bytes());
        bytes.extend((events.len() as i32).to_le_bytes());
        for (endpoint, offset) in events {
            bytes.extend(offset.to_le_bytes());
            tl_string(&mut bytes, endpoint);
            bytes.extend(0i32.to_le_bytes());
            bytes.extend(0i32.to_le_bytes());
        }
        bytes.extend(file);
        bytes
    }

    #[test]
    fn the_file_is_found_after_telegrams_header() {
        let file = b"\x00\x00\x00\x20ftypisom-and-the-rest";
        assert_eq!(segment_file(&segment("mp4", &[], file)).unwrap(), file);
        // Events in between, with endpoints of every length, are skipped.
        let events = [("a", 0), ("endpoint-of-some-length", 12), ("abc", 40)];
        assert_eq!(segment_file(&segment("mp4", &events, file)).unwrap(), file);
        assert!(segment_file(&segment("ogg", &[], file)).is_err(), "audio-only segments are not this stream's");
        assert!(segment_file(b"\x01\x02\x03\x04rest").is_err());
        assert!(segment_file(&[0x0d, 0x81, 0x2e]).is_err(), "cut short");
    }

    #[test]
    fn a_whole_second_from_telegram_plays_as_matroska() {
        let mut remuxer = Remuxer::new();
        let bytes = segment("mp4", &[("endpoint", 0)], crate::player::remux::tests::SECONDS[1]);
        let file = segment_file(&bytes).expect("a file");
        let stream = remuxer.push(file, 0).expect("remuxed");
        assert_eq!(&stream[..4], &[0x1A, 0x45, 0xDF, 0xA3]);
    }

    #[test]
    fn segment_lengths_follow_the_scale() {
        assert_eq!(chunk_ms(0), Some(1000));
        assert_eq!(chunk_ms(1), Some(500));
        assert_eq!(chunk_ms(-1), Some(2000));
        assert_eq!(chunk_ms(9), None);
    }

    #[test]
    fn rows_are_known_by_their_ids() {
        assert_eq!(parse_id("story:-100123:7"), Some((-100123, Kind::Story { story_id: 7 })));
        assert_eq!(parse_id("chat:42"), Some((42, Kind::Call)));
        assert_eq!(parse_id("chat:42:1"), None);
        assert_eq!(parse_id("story:42"), None);
        assert_eq!(parse_id("nonsense"), None);
    }

    #[test]
    fn paid_comments_take_the_colour_of_their_level() {
        LEVELS.with(|levels| *levels.borrow_mut() = Vec::new());
        assert_eq!(tier(0), 0);
        assert_eq!(tier(5), 1, "paid, with no levels known");
        LEVELS.with(|levels| *levels.borrow_mut() = vec![1, 50, 100, 500, 2000, 10000]);
        assert_eq!(tier(1), 1);
        assert_eq!(tier(100), 2);
        assert_eq!(tier(500), 2);
        assert_eq!(tier(2000), 3);
        assert_eq!(tier(10000), 3);
    }
}
