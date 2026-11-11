//! The chat info panel's pages (the design's round 14; ui/chat-info-pages.slint): the shared media
//! in three tabs, all of a group's members (searched on Telegram's servers), its administrators,
//! auto-delete, and how long the chat is muted with what notifies all the same. A page takes the
//! panel's place in its column as the management page does (manage.rs): Back or Esc gives the
//! panel back, and another chat coming to the front closes it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use chrono::{Datelike, Local, TimeZone};
use serde_json::json;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::api::{self, ChatMemberStatus, ChatMembers, ChatType, FoundChatMessages, MessageContent as M, MessageSender, TextEntityType};
use super::store::{self, Names, Store};
use super::{Error, conversation, files, send, time, with_ui};
use crate::{ChatKind, Day, InfoChoice, InfoFile, InfoLink, InfoMonth, InfoPage, InfoPageKind, InfoPageWords, InfoPerson, InfoTab, InfoTile, InfoTileRow, InfoWords, MainWindow, ManageToggle, ManageWords};

/// How many messages a tab asks for at a time.
const MEDIA_PAGE: i32 = 60;
/// How many of a group's members the page lists.
const MEMBERS: i32 = 200;
/// A search runs this long after the last keystroke.
const SEARCH_AFTER: Duration = Duration::from_millis(350);
/// What each tab holds.
const FILTERS: [&str; 3] = ["searchMessagesFilterPhotoAndVideo", "searchMessagesFilterDocument", "searchMessagesFilterUrl"];
/// Auto-delete's choices, in seconds: off, a day, a week, a month (31 days, as Telegram's apps).
const AUTO_DELETE: [i32; 4] = [0, 86_400, 604_800, 2_678_400];
/// The mute's choices, in seconds: not muted, an hour, eight hours, two days, for good.
const MUTE: [i32; 5] = [0, 3_600, 28_800, 172_800, i32::MAX];
/// A mute that long or longer is for good (TDLib: more than a year).
const FOREVER: i32 = 366 * 86_400;
/// A picture as big as this on its longer side, or the largest there is, fills a tile.
const TILE_SIDE: i32 = 240;

#[derive(Default, Clone, Copy, PartialEq)]
enum Page {
    #[default]
    None,
    Media,
    Members,
    Admins,
    AutoDelete,
    Notifications,
}

/// Someone on the members' or the administrators' page.
#[derive(Clone)]
struct Person {
    user_id: i64,
    owner: bool,
    admin: bool,
    title: String,
}

#[derive(Default)]
struct State {
    page: Page,
    /// The chat the page is about.
    chat_id: i64,
    /// Counted up at each page, tab and search: an answer for an earlier one is let go.
    lookups: u64,
    tab: usize,
    /// How many each tab holds, once known.
    counts: [Option<i32>; 3],
    /// The tab's messages, the newest first.
    found: Vec<api::Message>,
    /// Where the next page of the tab starts; 0 when there is none.
    next_from: i64,
    loading: bool,
    /// The files on their way down, by message.
    downloading: HashSet<i64>,
    query: String,
    people: Vec<Person>,
    member_count: i32,
    /// The group hides its members from us: only its administrators are listed.
    hidden: bool,
    /// Whether we administer the group, for the administrators' foot note.
    administrator: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// The mute's length last chosen here, by chat: TDLib only says how long is left.
    static MUTE_PICKS: RefCell<HashMap<i64, i32>> = RefCell::new(HashMap::new());
}

pub fn connect(ui: &MainWindow) {
    let page = ui.global::<InfoPage>();
    page.on_back(close);
    page.on_pick_tab(|tab| pick_tab(tab.clamp(0, 2) as usize));
    page.on_search(|query| search(query.to_string()));
    page.on_choose(|id| choose(&id));
    page.on_toggle(|id| toggle(&id));
    page.on_open_person(|id| {
        if let Ok(user_id) = id.parse() {
            conversation::open_user(user_id);
        }
    });
    page.on_open_tile(|id| {
        if let Ok(message_id) = id.parse() {
            open_tile(message_id);
        }
    });
    page.on_open_file(|id| {
        if let Ok(message_id) = id.parse() {
            open_file(message_id);
        }
    });
    page.on_open_link(|id| {
        if let Ok(message_id) = id.parse() {
            open_link(message_id);
        }
    });
    page.on_load_more(|| load_media(true));
}

// ---- opening and closing ---------------------------------------------------------------------

/// The shared media at a tab (0 photos and videos, 1 files, 2 links).
pub fn open_media(tab: usize) {
    begin(Page::Media);
    STATE.with(|state| state.borrow_mut().tab = tab.min(2));
    fetch_counts();
    load_media(false);
}

pub fn open_members() {
    begin(Page::Members);
    load_people();
}

pub fn open_admins() {
    begin(Page::Admins);
    load_people();
}

pub fn open_auto_delete() {
    begin(Page::AutoDelete);
    refresh();
}

pub fn open_notifications() {
    begin(Page::Notifications);
    refresh();
}

fn begin(page: Page) {
    super::manage::close();
    let chat_id = store::with(|store| store.open).flatten().unwrap_or(0);
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let lookups = state.lookups + 1;
        *state = State { page, chat_id, lookups, ..State::default() };
    });
    with_ui(|ui| {
        let page = ui.global::<InfoPage>();
        page.set_query(SharedString::new());
        page.set_open(true);
    });
}

/// Back: the panel again.
pub fn close() {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let lookups = state.lookups + 1;
        *state = State { lookups, ..State::default() };
    });
    with_ui(|ui| {
        let page = ui.global::<InfoPage>();
        page.set_open(false);
        page.set_kind(InfoPageKind::None);
        page.set_months(ModelRc::default());
        page.set_people(ModelRc::default());
        page.set_choices(ModelRc::default());
        page.set_toggles(ModelRc::default());
    });
}

/// Esc: a page open closes. False when none is.
pub(super) fn escape() -> bool {
    if STATE.with(|state| state.borrow().page) == Page::None {
        return false;
    }
    close();
    true
}

/// Another chat came to the front: the page was about the one before.
pub fn chat_changed() {
    let (page, chat_id) = STATE.with(|state| {
        let state = state.borrow();
        (state.page, state.chat_id)
    });
    if page != Page::None && store::with(|store| store.open).flatten() != Some(chat_id) {
        close();
    }
}

/// The store changed (store.rs): the pictures that came, the names, the chat's settings.
pub fn changed() {
    if STATE.with(|state| state.borrow().page) != Page::None {
        refresh();
    }
}

/// Logged out: the next account's chats are its own.
pub fn forget() {
    close();
    MUTE_PICKS.with(|picks| picks.borrow_mut().clear());
}

/// An answer came, for the page `lookups` (let go when another page, tab or search came since).
fn answered(lookups: u64, take: impl FnOnce(&mut State)) {
    let current = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.lookups != lookups {
            return false;
        }
        take(&mut state);
        true
    });
    if current {
        refresh();
    }
}

// ---- the shared media ------------------------------------------------------------------------

/// How many photos and videos, files and links the chat holds, for the tabs.
fn fetch_counts() {
    let (chat_id, lookups) = STATE.with(|state| {
        let state = state.borrow();
        (state.chat_id, state.lookups)
    });
    for (tab, filter) in FILTERS.iter().enumerate() {
        let request = json!({ "@type": "getChatMessageCount", "chat_id": chat_id, "filter": { "@type": filter }, "return_local": false });
        send(request, move |answer| {
            let Some(count) = answer.ok().and_then(|value| value["count"].as_i64()) else { return };
            answered(lookups, |state| state.counts[tab] = Some(count as i32));
        });
    }
}

fn pick_tab(tab: usize) {
    let changed = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.page != Page::Media || state.tab == tab {
            return false;
        }
        state.tab = tab;
        state.found.clear();
        state.next_from = 0;
        state.loading = false;
        state.lookups += 1;
        true
    });
    if changed {
        // The counts asked for under the old lookup are let go: ask again.
        fetch_counts();
        load_media(false);
    }
}

/// The tab's messages, newest first, a page at a time; `more`: the page after those here.
fn load_media(more: bool) {
    let Some((chat_id, tab, from, lookups)) = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.page != Page::Media || state.loading || (more && state.next_from == 0) {
            return None;
        }
        state.loading = true;
        Some((state.chat_id, state.tab, if more { state.next_from } else { 0 }, state.lookups))
    }) else {
        return;
    };
    refresh();
    let request = json!({
        "@type": "searchChatMessages", "chat_id": chat_id, "query": "", "sender_id": null,
        "from_message_id": from, "offset": 0, "limit": MEDIA_PAGE, "filter": { "@type": FILTERS[tab] },
    });
    send(request, move |answer| {
        let found = match answer.map(serde_json::from_value::<FoundChatMessages>) {
            Ok(Ok(found)) => Some(found),
            Ok(Err(err)) => {
                eprintln!("telegram: cannot read the shared media: {err}");
                None
            }
            Err(Error::Stopped) => None,
            Err(err) => {
                eprintln!("telegram: cannot list the shared media: {err}");
                None
            }
        };
        answered(lookups, |state| {
            state.loading = false;
            match found {
                Some(found) => {
                    if found.total_count >= 0 && state.counts[tab].is_none() {
                        state.counts[tab] = Some(found.total_count);
                    }
                    state.next_from = found.next_from_message_id;
                    state.found.extend(found.messages);
                }
                None => state.next_from = 0,
            }
        });
    });
}

/// A photo or video clicked: its chat goes to the message, and the viewer shows it once it is
/// there (the viewer shows what the chat has loaded).
pub fn open_tile(message_id: i64) {
    super::actions::jump(message_id);
    super::viewer::view_when_here(message_id, 30);
}

/// A file clicked: shown in the Finder when it is here, stopped when it is on its way, else
/// downloaded (as a file card in the chat does).
fn open_file(message_id: i64) {
    let Some((file, downloading)) = STATE.with(|state| {
        let state = state.borrow();
        let message = state.found.iter().find(|message| message.id == message_id)?;
        let M::Document { document, .. } = &message.content else { return None };
        Some((document.document.clone(), state.downloading.contains(&message_id)))
    }) else {
        return;
    };
    if let Some(path) = files::completed(&file) {
        crate::platform::reveal_file(&path);
        return;
    }
    if downloading || file.local.is_downloading_active {
        let request = json!({ "@type": "cancelDownloadFile", "file_id": file.id, "only_if_pending": false });
        send(request, |answer| log("stop a download", answer));
        STATE.with(|state| state.borrow_mut().downloading.remove(&message_id));
        refresh();
        return;
    }
    STATE.with(|state| state.borrow_mut().downloading.insert(message_id));
    refresh();
    files::download(&file, files::ON_SCREEN, move |_| {
        STATE.with(|state| state.borrow_mut().downloading.remove(&message_id));
        refresh();
    });
}

/// A link clicked: opened as a link in a message is.
fn open_link(message_id: i64) {
    let url = STATE.with(|state| {
        let state = state.borrow();
        let message = state.found.iter().find(|message| message.id == message_id)?;
        link_of(&message.content).map(|link| link.url)
    });
    if let Some(url) = url {
        conversation::open_link(&url);
    }
}

/// A message's link, as the links tab shows it.
struct Link {
    url: String,
    shown: String,
    title: String,
    site: String,
}

fn link_of(content: &M) -> Option<Link> {
    let (text, preview) = match content {
        M::Text { text, link_preview } => (text, link_preview.as_ref()),
        M::Photo { caption, .. } | M::Video { caption, .. } | M::Document { caption, .. } | M::Audio { caption, .. } | M::Animation { caption, .. } => {
            (caption, None)
        }
        _ => return None,
    };
    let url = preview.map(|preview| preview.url.clone()).filter(|url| !url.is_empty()).or_else(|| first_url(text))?;
    let bare = url.trim_start_matches("https://").trim_start_matches("http://").trim_start_matches("www.").to_string();
    let domain = bare.split('/').next().unwrap_or_default().to_string();
    let shown = preview.map(|preview| preview.display_url.clone()).filter(|shown| !shown.is_empty()).unwrap_or(bare);
    let site = preview.map(|preview| preview.site_name.clone()).filter(|site| !site.is_empty()).unwrap_or_else(|| domain.clone());
    let words = text.text.lines().map(str::trim).find(|line| !line.is_empty() && !line.contains(&shown)).map(|line| line.chars().take(120).collect::<String>());
    let title = preview.map(|preview| preview.title.clone()).filter(|title| !title.is_empty()).or(words).unwrap_or_else(|| domain.clone());
    Some(Link { url, shown, title, site })
}

/// The first link in a text: a plain address, or words that link somewhere.
fn first_url(text: &api::FormattedText) -> Option<String> {
    text.entities.iter().find_map(|entity| match &entity.kind {
        TextEntityType::Url => Some(utf16_slice(&text.text, entity.offset, entity.length)),
        TextEntityType::TextUrl { url } => Some(url.clone()),
        _ => None,
    })
}

/// The part of `text` TDLib's offsets point at: they count UTF-16 code units.
fn utf16_slice(text: &str, offset: i32, length: i32) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    let start = (offset.max(0) as usize).min(units.len());
    let end = (start + length.max(0) as usize).min(units.len());
    String::from_utf16_lossy(&units[start..end])
}

/// Who sent a message, as the media's lines say it; nobody for a channel's own posts.
fn sender_name(store: &Store, chat_id: i64, sender: &MessageSender, names: &Names) -> String {
    match sender {
        MessageSender::User { user_id } => store.user_name(*user_id, names),
        MessageSender::Chat { chat_id: sender } if *sender != chat_id => store.chats.get(sender).map(|chat| store.title(chat, names)).unwrap_or_default(),
        MessageSender::Chat { .. } => String::new(),
    }
}

fn tile_of(store: &Store, message: &api::Message, from: String) -> Option<InfoTile> {
    let (file, preview, video, duration) = match &message.content {
        M::Photo { photo, is_secret: false, .. } => {
            let size = photo.sizes.iter().find(|size| size.width.max(size.height) >= TILE_SIDE).or(photo.sizes.last());
            (size.map(|size| &size.photo), photo.minithumbnail.as_ref(), false, 0)
        }
        M::Video { video, is_secret: false, .. } => (video.thumbnail.as_ref().map(|thumbnail| &thumbnail.file), video.minithumbnail.as_ref(), true, video.duration),
        _ => return None,
    };
    let (picture, has_picture) = store.picture_of(file, preview);
    Some(InfoTile {
        id: message.id.to_string().into(),
        picture,
        has_picture,
        video,
        duration: format!("{}:{:02}", duration / 60, duration % 60).into(),
        from: from.into(),
        moment: time::moment(message.date),
    })
}

fn file_of(message: &api::Message, from: String, downloading: &HashSet<i64>) -> Option<InfoFile> {
    let M::Document { document, .. } = &message.content else { return None };
    let file = &document.document;
    let ext = Path::new(&document.file_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_uppercase)
        .filter(|ext| !ext.is_empty() && ext.chars().count() <= 4)
        .unwrap_or_else(|| "FILE".to_string());
    let downloaded = files::completed(file).is_some();
    let size = if file.size > 0 { file.size } else { file.expected_size };
    Some(InfoFile {
        id: message.id.to_string().into(),
        ext: ext.into(),
        name: if document.file_name.is_empty() { "—".into() } else { document.file_name.clone().into() },
        size: store::size_text(size).into(),
        from: from.into(),
        moment: time::moment(message.date),
        downloaded,
        downloading: !downloaded && (downloading.contains(&message.id) || file.local.is_downloading_active),
    })
}

fn link_line(message: &api::Message, from: String) -> Option<InfoLink> {
    let link = link_of(&message.content)?;
    let initial: String = link.site.chars().next().map(|first| first.to_uppercase().collect()).unwrap_or_default();
    Some(InfoLink {
        id: message.id.to_string().into(),
        initial: initial.into(),
        title: link.title.into(),
        url: link.shown.into(),
        from: from.into(),
        moment: time::moment(message.date),
    })
}

/// One month of a tab, as it is gathered.
#[derive(Default)]
struct Month {
    year: i32,
    month: i32,
    tiles: Vec<InfoTile>,
    files: Vec<InfoFile>,
    links: Vec<InfoLink>,
}

/// The tab's messages by month, the newest first: tiles in rows of three, files, links.
fn months_of(store: &Store, state: &State, names: &Names) -> Vec<InfoMonth> {
    let this_year = Local::now().year();
    let mut months: Vec<Month> = Vec::new();
    for message in &state.found {
        let Some(when) = Local.timestamp_opt(i64::from(message.date), 0).single() else { continue };
        let (year, month) = (when.year(), when.month() as i32);
        if months.last().is_none_or(|last| (last.year, last.month) != (year, month)) {
            months.push(Month { year, month, ..Month::default() });
        }
        let from = sender_name(store, state.chat_id, &message.sender_id, names);
        let Some(last) = months.last_mut() else { continue };
        match state.tab {
            0 => last.tiles.extend(tile_of(store, message, from)),
            1 => last.files.extend(file_of(message, from, &state.downloading)),
            _ => last.links.extend(link_line(message, from)),
        }
    }
    months
        .into_iter()
        .filter(|month| !month.tiles.is_empty() || !month.files.is_empty() || !month.links.is_empty())
        .map(|month| InfoMonth {
            month: month.month,
            year: month.year,
            this_year: month.year == this_year,
            rows: ModelRc::new(VecModel::from(tile_rows(month.tiles))),
            files: ModelRc::new(VecModel::from(month.files)),
            links: ModelRc::new(VecModel::from(month.links)),
        })
        .collect()
}

/// Tiles three to a row, the last row filled up with empty ones.
pub(super) fn tile_rows(tiles: Vec<InfoTile>) -> Vec<InfoTileRow> {
    tiles
        .chunks(3)
        .map(|chunk| {
            let mut row = chunk.to_vec();
            row.resize(3, InfoTile::default());
            InfoTileRow { tiles: ModelRc::new(VecModel::from(row)) }
        })
        .collect()
}

/// The newest photos and videos of a chat as tiles, for the panel's shared media section.
pub(super) fn tiles(store: &Store, chat_id: i64, messages: &[api::Message], names: &Names) -> Vec<InfoTile> {
    messages
        .iter()
        .filter_map(|message| tile_of(store, message, sender_name(store, chat_id, &message.sender_id, names)))
        .collect()
}

/// 1284 → "1,284", as Fmt.count writes it.
pub(super) fn grouped(n: i32) -> String {
    let digits = n.max(0).to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

// ---- the members, the administrators ---------------------------------------------------------

fn search(query: String) {
    let lookups = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.query = query.trim().to_string();
        state.lookups += 1;
        state.lookups
    });
    slint::Timer::single_shot(SEARCH_AFTER, move || {
        if STATE.with(|state| state.borrow().lookups) == lookups {
            load_people();
        }
    });
}

fn person(member: &api::ChatMember) -> Option<Person> {
    let MessageSender::User { user_id } = member.member_id else { return None };
    let (owner, admin, title) = match &member.status {
        ChatMemberStatus::Creator => (true, false, String::new()),
        ChatMemberStatus::Administrator { custom_title, .. } => (false, true, custom_title.clone()),
        _ => (false, false, String::new()),
    };
    Some(Person { user_id, owner, admin, title })
}

/// The owner first, then the administrators, then everyone else in the order given.
fn by_rank(people: &mut [Person]) {
    people.sort_by_key(|person| if person.owner { 0 } else if person.admin { 1 } else { 2 });
}

fn administers(status: &ChatMemberStatus) -> bool {
    matches!(status, ChatMemberStatus::Creator | ChatMemberStatus::Administrator { .. })
}

fn load_people() {
    let (chat_id, page, query, lookups) = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.loading = true;
        (state.chat_id, state.page, state.query.clone(), state.lookups)
    });
    let Some((kind, administrator)) = store::with(|store| {
        let chat = store.chats.get(&chat_id)?;
        let status = match chat.kind {
            ChatType::BasicGroup { basic_group_id } => store.basic_groups.get(&basic_group_id).map(|group| group.status.clone()),
            ChatType::Supergroup { supergroup_id, .. } => store.supergroups.get(&supergroup_id).map(|group| group.status.clone()),
            _ => None,
        };
        Some((chat.kind, status.as_ref().is_some_and(administers)))
    })
    .flatten() else {
        return;
    };
    STATE.with(|state| state.borrow_mut().administrator = administrator);
    refresh();
    match (page, kind) {
        (Page::Admins, _) => load_admins(chat_id, lookups, false),
        (Page::Members, ChatType::Supergroup { supergroup_id, .. }) => {
            send(json!({ "@type": "getSupergroupFullInfo", "supergroup_id": supergroup_id }), move |answer| {
                let Ok(Ok(info)) = answer.map(serde_json::from_value::<api::SupergroupFullInfo>) else { return };
                answered(lookups, |state| state.member_count = info.member_count);
                if !info.can_get_members && !administrator {
                    load_admins(chat_id, lookups, true);
                    return;
                }
                let filter = if query.is_empty() {
                    json!({ "@type": "supergroupMembersFilterRecent" })
                } else {
                    json!({ "@type": "supergroupMembersFilterSearch", "query": query })
                };
                let request = json!({ "@type": "getSupergroupMembers", "supergroup_id": supergroup_id, "filter": filter, "offset": 0, "limit": MEMBERS });
                send(request, move |answer| {
                    let found = answer.ok().and_then(|value| serde_json::from_value::<ChatMembers>(value).ok());
                    answered(lookups, |state| {
                        state.loading = false;
                        let mut people: Vec<Person> = found.map(|found| found.members.iter().filter_map(person).collect()).unwrap_or_default();
                        by_rank(&mut people);
                        state.people = people;
                    });
                });
            });
        }
        (Page::Members, ChatType::BasicGroup { basic_group_id }) => {
            send(json!({ "@type": "getBasicGroupFullInfo", "basic_group_id": basic_group_id }), move |answer| {
                let Ok(Ok(info)) = answer.map(serde_json::from_value::<api::BasicGroupFullInfo>) else { return };
                // A small group's members are all here: the search runs over their names.
                let wanted = query.to_lowercase();
                let names = store::with(|store| {
                    info.members
                        .iter()
                        .filter_map(person)
                        .filter(|person| {
                            if wanted.is_empty() {
                                return true;
                            }
                            store.users.get(&person.user_id).is_some_and(|user| {
                                let name = format!("{} {}", user.first_name, user.last_name).to_lowercase();
                                let username = user.usernames.as_ref().and_then(|names| names.active_usernames.first()).map(|name| name.to_lowercase());
                                name.contains(&wanted) || username.is_some_and(|name| name.contains(&wanted))
                            })
                        })
                        .collect::<Vec<Person>>()
                })
                .unwrap_or_default();
                answered(lookups, |state| {
                    state.loading = false;
                    state.member_count = info.members.len() as i32;
                    let mut people = names;
                    by_rank(&mut people);
                    state.people = people;
                });
            });
        }
        _ => STATE.with(|state| state.borrow_mut().loading = false),
    }
}

/// The owner and the administrators, everyone may see; `hidden`: in place of the members, whom
/// the group hides.
fn load_admins(chat_id: i64, lookups: u64, hidden: bool) {
    send(json!({ "@type": "getChatAdministrators", "chat_id": chat_id }), move |answer| {
        let found = answer.ok().and_then(|value| serde_json::from_value::<api::ChatAdministrators>(value).ok()).unwrap_or_default();
        answered(lookups, |state| {
            state.loading = false;
            state.hidden = hidden;
            let mut people: Vec<Person> = found
                .administrators
                .into_iter()
                .map(|admin| Person { user_id: admin.user_id, owner: admin.is_owner, admin: !admin.is_owner, title: admin.custom_title })
                .collect();
            by_rank(&mut people);
            state.people = people;
        });
    });
}

// ---- auto-delete, the notifications ----------------------------------------------------------

/// Auto-delete's setting in words: "Off", "1 week", "3 days".
pub(super) fn auto_delete_label(ui: &MainWindow, seconds: i32) -> SharedString {
    let words = ui.global::<InfoPageWords>();
    match seconds {
        0 => ui.global::<InfoWords>().invoke_off(),
        86_400 => words.invoke_one_day(),
        604_800 => words.invoke_one_week(),
        2_678_400 => words.invoke_one_month(),
        _ => words.invoke_days((seconds / 86_400).max(1)),
    }
}

/// Whether we may change what the chat's info holds (auto-delete asks for that right in groups
/// and channels; in a private chat both may).
fn may_change_info(store: &Store, chat: &api::Chat) -> bool {
    let status = match chat.kind {
        ChatType::Private { .. } | ChatType::Secret { .. } => return true,
        ChatType::BasicGroup { basic_group_id } => store.basic_groups.get(&basic_group_id).map(|group| &group.status),
        ChatType::Supergroup { supergroup_id, .. } => store.supergroups.get(&supergroup_id).map(|group| &group.status),
    };
    match status {
        Some(ChatMemberStatus::Creator) => true,
        Some(ChatMemberStatus::Administrator { rights, .. }) => rights.can_change_info,
        _ => false,
    }
}

fn choose(id: &str) {
    let (page, chat_id) = STATE.with(|state| {
        let state = state.borrow();
        (state.page, state.chat_id)
    });
    let Ok(seconds) = id.parse::<i32>() else { return };
    match page {
        Page::AutoDelete => {
            let request = json!({ "@type": "setChatMessageAutoDeleteTime", "chat_id": chat_id, "message_auto_delete_time": seconds });
            send(request, |answer| log("set auto-delete", answer));
        }
        Page::Notifications => {
            let Some(settings) = store::with(|store| store.chats.get(&chat_id).map(|chat| chat.notification_settings.with_mute_for(seconds))).flatten() else {
                return;
            };
            MUTE_PICKS.with(|picks| picks.borrow_mut().insert(chat_id, seconds));
            let request = json!({ "@type": "setChatNotificationSettings", "chat_id": chat_id, "notification_settings": settings });
            send(request, |answer| log("mute a chat", answer));
        }
        _ => {}
    }
}

/// The switches under the mute: mentions and replies, pinned messages.
fn toggle(id: &str) {
    let chat_id = STATE.with(|state| state.borrow().chat_id);
    let Some(settings) = store::with(|store| {
        let chat = store.chats.get(&chat_id)?;
        let (_, mentions, pinned) = store.notifications(chat);
        let mut settings = chat.notification_settings.clone();
        match id {
            "mentions" => {
                settings.use_default_disable_mention_notifications = false;
                settings.disable_mention_notifications = mentions;
            }
            "pinned" => {
                settings.use_default_disable_pinned_message_notifications = false;
                settings.disable_pinned_message_notifications = pinned;
            }
            _ => return None,
        }
        Some(settings.request())
    })
    .flatten() else {
        return;
    };
    let request = json!({ "@type": "setChatNotificationSettings", "chat_id": chat_id, "notification_settings": settings });
    send(request, |answer| log("change a chat's notifications", answer));
}

fn log(what: &str, answer: Result<serde_json::Value, Error>) {
    match answer {
        Ok(_) | Err(Error::Stopped) => {}
        Err(err) => eprintln!("telegram: cannot {what}: {err}"),
    }
}

// ---- drawing ---------------------------------------------------------------------------------

fn refresh() {
    with_ui(|ui| {
        let page = ui.global::<InfoPage>();
        let names = Names::from(ui);
        let words = ui.global::<InfoWords>();
        let page_words = ui.global::<InfoPageWords>();
        STATE.with(|state| {
            let state = state.borrow();
            match state.page {
                Page::None => {}
                Page::Media => draw_media(&page, &state, &words, &page_words, &names),
                Page::Members | Page::Admins => draw_people(ui, &page, &state, &words, &page_words, &names),
                Page::AutoDelete => draw_auto_delete(ui, &page, &state, &words, &page_words, &names),
                Page::Notifications => draw_notifications(&page, &state, &words, &page_words),
            }
        });
    });
    store::fetch_wanted_photos();
}

fn clear_lists(page: &InfoPage) {
    page.set_count(SharedString::new());
    page.set_note(SharedString::new());
    page.set_explain(SharedString::new());
    page.set_foot(SharedString::new());
    page.set_choices_title(SharedString::new());
    page.set_toggles_title(SharedString::new());
    page.set_locked(false);
    page.set_lock_note(SharedString::new());
    page.set_searchable(false);
}

fn draw_media(page: &InfoPage, state: &State, words: &InfoWords, page_words: &InfoPageWords, names: &Names) {
    clear_lists(page);
    page.set_kind(InfoPageKind::Media);
    page.set_title(words.invoke_shared());
    page.set_tab(state.tab as i32);
    let labels = [words.invoke_tab_media(), words.invoke_tab_files(), words.invoke_tab_links()];
    let tabs: Vec<InfoTab> = labels
        .into_iter()
        .zip(state.counts)
        .map(|(label, count)| InfoTab { label, count: count.map(grouped).unwrap_or_default().into() })
        .collect();
    page.set_tabs(ModelRc::new(VecModel::from(tabs)));
    let months = store::with(|store| months_of(store, state, names)).unwrap_or_default();
    page.set_months(ModelRc::new(VecModel::from(months)));
    page.set_loading(state.loading);
    page.set_more(state.next_from != 0);
    page.set_empty(match state.tab {
        0 => page_words.invoke_no_media(),
        1 => page_words.invoke_no_files(),
        _ => page_words.invoke_no_links(),
    });
}

fn draw_people(ui: &MainWindow, page: &InfoPage, state: &State, words: &InfoWords, page_words: &InfoPageWords, names: &Names) {
    clear_lists(page);
    let admins = state.page == Page::Admins;
    let manage_words = ui.global::<ManageWords>();
    page.set_kind(if admins { InfoPageKind::Admins } else { InfoPageKind::Members });
    page.set_title(if admins { words.invoke_administrators() } else { words.invoke_members() });
    page.set_searchable(!admins && !state.hidden);
    page.set_loading(state.loading);
    let shown = if admins || state.hidden { state.people.len() as i32 } else { state.member_count };
    page.set_count(if shown > 0 { grouped(shown).into() } else { SharedString::new() });
    if state.hidden {
        page.set_note(page_words.invoke_hidden_note(state.member_count));
    }
    page.set_list_title(if admins || state.hidden {
        words.invoke_administrators()
    } else if !state.query.is_empty() {
        page_words.invoke_results()
    } else {
        words.invoke_members()
    });
    // All members: how many are online now; a search: how many it found.
    let online = store::with(|store| store.online_members.get(&state.chat_id).copied()).flatten().unwrap_or(0);
    page.set_list_count(if admins || state.hidden {
        SharedString::new()
    } else if !state.query.is_empty() {
        grouped(state.people.len() as i32).into()
    } else if online > 0 {
        page_words.invoke_online(online)
    } else {
        SharedString::new()
    });
    if (admins || state.hidden) && !state.administrator {
        page.set_foot(page_words.invoke_admins_foot());
    }
    let searching = !state.query.is_empty();
    let people: Vec<InfoPerson> = store::with(|store| {
        state
            .people
            .iter()
            .filter_map(|person| {
                let user = store.users.get(&person.user_id)?;
                let name = store.user_name(person.user_id, names);
                let (picture, has_picture) = store.user_avatar(user);
                let (status, last_seen) = store::user_status(user);
                let rank = if person.owner {
                    words.invoke_owner()
                } else if person.admin {
                    words.invoke_admin()
                } else {
                    SharedString::new()
                };
                let (sub, role) = if admins || state.hidden {
                    let sub = if person.owner { manage_words.invoke_group_owner() } else { manage_words.invoke_administrator() };
                    (sub, SharedString::from(person.title.as_str()))
                } else {
                    let username = user.usernames.as_ref().and_then(|names| names.active_usernames.first()).filter(|_| searching);
                    let sub = username.map(|name| SharedString::from(format!("@{name}"))).unwrap_or_default();
                    (sub, if person.title.is_empty() { rank } else { person.title.as_str().into() })
                };
                Some(InfoPerson {
                    id: person.user_id.to_string().into(),
                    initial: store::initial(&name),
                    name: name.into(),
                    picture,
                    has_picture,
                    you: person.user_id == store.my_id,
                    status,
                    last_seen,
                    sub,
                    role,
                    owner: person.owner,
                    admin: person.admin,
                })
            })
            .collect()
    })
    .unwrap_or_default();
    page.set_people(ModelRc::new(VecModel::from(people)));
}

fn draw_auto_delete(ui: &MainWindow, page: &InfoPage, state: &State, words: &InfoWords, page_words: &InfoPageWords, names: &Names) {
    clear_lists(page);
    page.set_kind(InfoPageKind::AutoDelete);
    page.set_title(words.invoke_auto_delete());
    let Some((kind, seconds, may, name)) = store::with(|store| {
        let chat = store.chats.get(&state.chat_id)?;
        Some((store.kind(chat), chat.message_auto_delete_time, may_change_info(store, chat), store.title(chat, names)))
    })
    .flatten() else {
        return;
    };
    let mut values: Vec<i32> = AUTO_DELETE.to_vec();
    if !values.contains(&seconds) {
        values.push(seconds);
    }
    let choices: Vec<InfoChoice> = values
        .into_iter()
        .map(|value| InfoChoice { id: value.to_string().into(), label: auto_delete_label(ui, value), sub: SharedString::new(), on: value == seconds })
        .collect();
    page.set_choices(ModelRc::new(VecModel::from(choices)));
    page.set_toggles(ModelRc::default());
    page.set_explain(match kind {
        ChatKind::Group => page_words.invoke_auto_delete_group(),
        ChatKind::Channel => page_words.invoke_auto_delete_channel(),
        _ => page_words.invoke_auto_delete_private(),
    });
    page.set_locked(!may);
    page.set_lock_note(page_words.invoke_auto_delete_lock());
    if matches!(kind, ChatKind::User | ChatKind::Bot) {
        page.set_foot(page_words.invoke_auto_delete_foot(name.into()));
    }
}

fn draw_notifications(page: &InfoPage, state: &State, words: &InfoWords, page_words: &InfoPageWords) {
    clear_lists(page);
    page.set_kind(InfoPageKind::Notifications);
    page.set_title(words.invoke_mute_for());
    let Some((mute_for, mentions, pinned)) = store::with(|store| store.chats.get(&state.chat_id).map(|chat| store.notifications(chat))).flatten() else {
        return;
    };
    let pick = MUTE_PICKS.with(|picks| picks.borrow().get(&state.chat_id).copied());
    let timed = mute_for > 0 && mute_for < FOREVER;
    let current = if mute_for == 0 {
        Some(0)
    } else if !timed {
        Some(i32::MAX)
    } else {
        pick.filter(|pick| *pick > 0 && *pick < FOREVER)
    };
    // When the mute ends: today's clock, or another day's date and clock.
    let until = timed.then(|| {
        let now = Local::now();
        let end = now.timestamp() + i64::from(mute_for);
        let mut moment = time::moment(end as i32);
        let today = Local.timestamp_opt(end, 0).single().is_some_and(|when| when.date_naive() == now.date_naive());
        if !today {
            moment.day = Day::ThisYear;
        }
        page_words.invoke_muted_until(moment, today)
    });
    let labels = [page_words.invoke_not_muted(), page_words.invoke_one_hour(), page_words.invoke_eight_hours(), page_words.invoke_two_days(), page_words.invoke_forever()];
    let choices: Vec<InfoChoice> = MUTE
        .into_iter()
        .zip(labels)
        .map(|(value, label)| {
            let on = current == Some(value);
            let sub = if on && timed { until.clone().unwrap_or_default() } else { SharedString::new() };
            InfoChoice { id: value.to_string().into(), label, sub, on }
        })
        .collect();
    // A mute set elsewhere, its length unknown: when it ends, over the choices.
    if timed && current.is_none() {
        page.set_explain(until.unwrap_or_default());
    }
    page.set_choices_title(page_words.invoke_mute());
    page.set_choices(ModelRc::new(VecModel::from(choices)));
    page.set_toggles_title(page_words.invoke_notify_anyway());
    let toggle = |id: &str, label: SharedString, on: bool| ManageToggle { id: id.into(), label, note: SharedString::new(), on, enabled: true, lock: SharedString::new() };
    page.set_toggles(ModelRc::new(VecModel::from(vec![
        toggle("mentions", page_words.invoke_mentions(), mentions),
        toggle("pinned", page_words.invoke_pinned(), pinned),
    ])));
    page.set_foot(page_words.invoke_notifications_foot());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_grouped_by_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1284), "1,284");
        assert_eq!(grouped(32410), "32,410");
        assert_eq!(grouped(1234567), "1,234,567");
    }

    #[test]
    fn utf16_offsets_find_the_link() {
        let text = "看这个 https://t.me/kbd_daily/812 吧";
        let start = "看这个 ".encode_utf16().count() as i32;
        let length = "https://t.me/kbd_daily/812".encode_utf16().count() as i32;
        assert_eq!(utf16_slice(text, start, length), "https://t.me/kbd_daily/812");
    }

    #[test]
    fn rows_are_filled_to_three() {
        let rows = tile_rows(vec![InfoTile::default(); 4]);
        assert_eq!(rows.len(), 2);
    }
}
