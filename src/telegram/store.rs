//! What TDLib has told us about the account: its chats, the users and groups in them, its folders,
//! and the messages of the chats that are open. It lives on the UI thread and changes there, from
//! updates, in batches (docs/architecture.md). After each batch, and after each change the pages
//! ask for, [`refresh`] brings the pages' models up to date; a model whose rows did not change is
//! left alone, so lists keep their scroll position.
//!
//! chats.rs, conversation.rs and account.rs add what the pages can do with it.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Image, Model, ModelRc, SharedPixelBuffer, SharedString, VecModel};

use super::api::{
    self, ChatList, ChatMemberStatus, ChatType, MessageContent as M, MessageOrigin, MessageReplyTo, MessageSender,
    MessageSendingState, NotificationSettingsScope, Update, UserStatus, UserType,
};
use super::avatars;
use super::rich_text;
use super::time;
use crate::images;
use crate::platform::NameOrder;
use crate::{
    Account, AdminPanel, AppState, BotCommand, ButtonKind, ButtonRow, ChatButton, ChatKind, ChatOrder, ChatRow, Chats, ContactCard, Content,
    Conversation, FileCard, FileState, FileType, Folder, FolderChoice, LinkPreview, MainWindow, Media, MessageRow, Moment, NotificationScopes,
    Reaction, ReplyQuote, RoundNote, RowKind, Status, Sticker, Tab, TopicRow, TreeRow, Viewer, ViewerItem, Words,
};

/// Commands shown at most when "/" is typed.
const COMMANDS_SHOWN: usize = 20;

/// How long someone counts as typing after TDLib last said so.
pub const TYPING_LASTS: Duration = Duration::from_secs(6);

/// Decoded pictures kept: those of the open chats and some more.
const PICTURES_KEPT: usize = 200;
const PREVIEWS_KEPT: usize = 1000;
/// The photos of chats and people kept (avatars.rs): every chat in the lists has one, a square of
/// 160 pixels, so enough for every chat an account is likely to have.
const AVATARS_KEPT: usize = 2048;
/// A photo is downloaded in the smallest size that is at least this large on its longer side:
/// still sharp at the size a chat shows it.
const PICTURE_SIDE: i32 = 640;

/// How many of the open chat's newest messages its rows show while the view is at the end. Older
/// ones join as the view nears the top (conversation.rs), and go again once it is back at the end.
/// The pages lay the rows out whole and measure every one of them again at each change among them,
/// so their number is what each new message, picture or edit costs.
pub const SHOWN: usize = 100;

/// The messages of an open chat that TDLib has given us, by id (oldest first).
pub struct History {
    pub messages: BTreeMap<i64, api::Message>,
    /// In a group with topics, the topic these messages are of (topics.rs): messages of other
    /// topics stay out.
    pub topic_id: Option<i32>,
    /// TDLib may have older messages than the oldest here.
    pub has_older: bool,
    /// TDLib has newer messages than the newest here: the rows are around an older message (a
    /// search match, a reply's original, conversation.rs), and the end of the chat is loaded again
    /// as the view comes down to it.
    pub has_newer: bool,
    /// A getChatHistory request is on its way.
    pub loading: bool,
    /// How many of the newest messages the rows show ([`SHOWN`] at the end).
    pub shown: usize,
}

impl Default for History {
    fn default() -> History {
        History { messages: BTreeMap::new(), topic_id: None, has_older: false, has_newer: false, loading: false, shown: SHOWN }
    }
}

impl History {
    /// The messages the rows show, oldest first: the newest `shown`, an album among them whole.
    pub fn shown_messages(&self) -> impl Iterator<Item = &api::Message> {
        let all: Vec<&api::Message> = self.messages.values().collect();
        let mut start = all.len().saturating_sub(self.shown);
        while start > 0 && all[start].media_album_id != 0 && all[start - 1].media_album_id == all[start].media_album_id {
            start -= 1;
        }
        self.messages.values().skip(start)
    }

    /// Messages older than the rows show are here already.
    pub fn hides_older(&self) -> bool {
        self.messages.len() > self.shown
    }

    /// Let the rows reach back to `message_id`; true when they did not before.
    pub fn show_from(&mut self, message_id: i64) -> bool {
        let needed = self.messages.range(message_id..).count();
        if needed > self.shown {
            self.shown = needed;
            true
        } else {
            false
        }
    }
}

/// Which models need building again at the next refresh.
#[derive(Default)]
pub struct Dirty {
    pub chats: bool,
    /// The open chat's messages (and its header with them).
    pub conversation: bool,
    /// The open chat's header alone: who is typing, how many are online, whether we may write.
    pub header: bool,
    pub account: bool,
    /// The media viewer's pictures (while it is open).
    pub viewer: bool,
    /// Settings → Notifications & sounds: whether each kind of chat notifies.
    pub notification_scopes: bool,
    /// The open chat should show its newest message.
    pub scroll_to_end: bool,
    /// The open group's list of topics (topics.rs).
    pub topics: bool,
}

/// The models the pages show, kept so that they can be updated in place.
struct Models {
    folders: Rc<VecModel<Folder>>,
    list: Rc<VecModel<ChatRow>>,
    tree: Rc<VecModel<TreeRow>>,
    channels: Rc<VecModel<ChatRow>>,
    official_bots: Rc<VecModel<ChatRow>>,
    bots: Rc<VecModel<ChatRow>>,
    tabs: Rc<VecModel<Tab>>,
    messages: Rc<VecModel<MessageRow>>,
}

pub struct Store {
    ui: slint::Weak<MainWindow>,
    pub my_id: i64,
    pub my_bio: String,
    pub chats: HashMap<i64, api::Chat>,
    pub users: HashMap<i64, api::User>,
    pub basic_groups: HashMap<i64, api::BasicGroup>,
    pub supergroups: HashMap<i64, api::Supergroup>,
    /// Members of supergroups and channels from their full info: the supergroup itself often says 0.
    pub supergroup_members: HashMap<i64, i32>,
    pub online_members: HashMap<i64, i32>,
    /// The notification settings of each kind of chat, which chats follow unless they say
    /// otherwise.
    pub scope_settings: HashMap<NotificationSettingsScope, api::ScopeNotificationSettings>,
    pub folders: Vec<api::ChatFolderInfo>,

    /// The folder Broadsheet and Terminal show: 0 is All chats, then the account's folders.
    pub shown_folder: usize,
    /// Workbench's tree: folders the user expanded or collapsed, by folder id (0 for All chats).
    pub expanded: HashMap<i32, bool>,

    /// Workbench's tabs, in order; the others show only the front one.
    pub tabs: Vec<i64>,
    pub open: Option<i64>,
    pub histories: HashMap<i64, History>,
    /// Who is typing where, and since when.
    pub typing: HashMap<i64, Vec<(MessageSender, Instant)>>,
    pub sponsored: HashMap<i64, Vec<api::SponsoredMessage>>,
    /// Pictures of photos, videos and GIFs, downloaded and decoded, by file id.
    pub pictures: images::Cache<i32>,
    /// The tiny previews in messages and in photos of chats, decoded once, by their bytes.
    previews: RefCell<images::Cache<u64>>,
    /// The photos of chats and people, downloaded and decoded, by file id (avatars.rs).
    pub avatars: images::Cache<i32>,
    /// Those asked for, each once: a photo that changes is a new file.
    avatars_asked: HashSet<i32>,
    /// Photos the rows wanted and did not have, to download after the refresh
    /// ([`fetch_wanted_photos`]).
    wanted_photos: RefCell<Vec<api::File>>,
    /// The account has Telegram Premium (TDLib's `is_premium` option): larger files may be sent.
    pub is_premium: bool,
    /// Telegram's limit on a caption (TDLib's `message_caption_length_max` option).
    pub caption_length_max: i32,
    /// How many members a basic group takes (TDLib's `basic_group_size_max` option); a new group
    /// with more is made as a supergroup (new_chat.rs).
    pub basic_group_size_max: i32,
    /// Files on their way up or down, by file id: how much has moved, and how much there is.
    transfers: HashMap<i32, (i64, i64)>,
    /// The files the rows shown hold, by id: a change to one of them redraws the rows.
    row_files: RefCell<HashSet<i32>>,
    /// The media list of each message row, by row id: the same model from one refresh to the next,
    /// so that a row changes only when its media do.
    media_models: RefCell<HashMap<SharedString, Rc<VecModel<Media>>>>,
    /// No picture: one empty image for every row without one. Slint's default image never equals
    /// another, not even itself, and a row that never compares equal is set again at each refresh,
    /// which lays the whole chat out again.
    blank: Image,

    /// The open chat's message rows as last shown, in order: each row's id (its first message) and
    /// its messages (several for an album).
    pub rows: Vec<(i64, Vec<i64>)>,
    /// The rows chosen while choosing messages (actions.rs).
    pub selected: HashSet<i64>,
    /// Messages replies answer that are not in the history, by chat and message: fetched once
    /// (conversation.rs); None, it was deleted.
    pub replied: HashMap<(i64, i64), Option<api::Message>>,
    /// Such messages the last refresh found missing, to fetch.
    pub missing_replies: Vec<(i64, i64)>,
    /// Such messages asked for.
    pub asking_replies: HashSet<(i64, i64)>,

    /// The newest pinned message of each chat opened, as TDLib said (bots.rs); None: it has none.
    pub pinned: HashMap<i64, Option<api::Message>>,
    /// The message whose keyboard a chat shows by the composer (a bot's), by chat; None: none.
    pub keyboards: HashMap<i64, Option<api::Message>>,
    /// Chats whose keyboard the user folded away, or that was used once (one_time).
    pub keyboard_hidden: HashSet<i64>,
    /// What bots say they can do, by their user id (their full info).
    pub bot_infos: HashMap<i64, api::BotInfo>,
    /// The commands of the bots in a group, by chat (the group's full info).
    pub group_commands: HashMap<i64, Vec<api::BotCommands>>,
    /// What is typed after "/" in the composer, lower case, while the draft is a command alone:
    /// the commands shown are those starting with it.
    pub command_filter: Option<String>,
    /// The buttons under each message row, by row id, with a hash of what they were built from:
    /// the same model while the buttons stay, so that a row changes only when they do.
    button_models: RefCell<HashMap<SharedString, (u64, ModelRc<ButtonRow>)>>,
    /// The keyboard's model, with the chat and message it was built from.
    keyboard_model: RefCell<Option<(i64, i64, ModelRc<ButtonRow>)>>,
    /// The commands shown, as last set.
    commands_shown: RefCell<Vec<BotCommand>>,

    /// The default reaction (updateDefaultReactionType), given by a double click (reactions.rs).
    pub default_reaction: api::ReactionType,
    /// Custom emoji by id: the emoji each stands for (getCustomEmojiStickers), for reactions and
    /// topics' icons; "" while it is being asked for.
    pub custom_emoji: HashMap<i64, String>,
    /// Custom emoji the rows met and did not know, to ask for after the refresh.
    custom_emoji_wanted: RefCell<Vec<i64>>,
    /// The topics of groups with topics, by chat, as TDLib orders them (topics.rs).
    pub topics: HashMap<i64, Vec<api::ForumTopic>>,
    /// Groups whose topics are being fetched.
    pub topics_loading: HashSet<i64>,
    /// The topic open in the front chat (a group with topics); None: its list of topics shows.
    pub open_topic: Option<i32>,
    /// The icons a new topic may take (getForumTopicDefaultIcons): each custom emoji's id and
    /// the emoji it shows.
    pub topic_icons: Vec<(i64, String)>,
    /// Supergroups' full info, by supergroup id: slow mode, and what administrators may change
    /// (admin.rs).
    pub supergroup_full: HashMap<i64, api::SupergroupFullInfo>,
    /// When slow mode lets us send again, by chat (admin.rs).
    pub slow_until: HashMap<i64, Instant>,
    /// The settings panel's control waiting for Telegram, and what Telegram refused (admin.rs).
    pub admin_busy: String,
    pub admin_error: String,
    /// Words the composer holds for chats not in front, until Telegram has them (drafts.rs).
    pub drafts: HashMap<i64, String>,
    /// Contacts added from cards this session, by phone number: their cards say so.
    pub added_contacts: HashSet<String>,
    /// The round video message playing, and the one whose file is on its way (viewer.rs).
    pub round_playing: Option<i64>,
    pub round_loading: Option<i64>,
    /// The reactions model of each row, by row id, with a hash of what it was built from.
    reaction_models: RefCell<HashMap<SharedString, (u64, ModelRc<Reaction>)>>,
    /// The topic rows as last set.
    topics_shown: RefCell<Vec<TopicRow>>,

    pub dirty: Dirty,
    models: Models,
}

thread_local! {
    static STORE: RefCell<Option<Store>> = const { RefCell::new(None) };
}

/// Set the store up for `ui` and give the pages its models. Call once, on the UI thread.
pub fn install(ui: &MainWindow) {
    let models = Models {
        folders: Rc::new(VecModel::default()),
        list: Rc::new(VecModel::default()),
        tree: Rc::new(VecModel::default()),
        channels: Rc::new(VecModel::default()),
        official_bots: Rc::new(VecModel::default()),
        bots: Rc::new(VecModel::default()),
        tabs: Rc::new(VecModel::default()),
        messages: Rc::new(VecModel::default()),
    };
    let chats = ui.global::<Chats>();
    chats.set_folders(ModelRc::from(models.folders.clone()));
    chats.set_list(ModelRc::from(models.list.clone()));
    chats.set_tree(ModelRc::from(models.tree.clone()));
    chats.set_channels(ModelRc::from(models.channels.clone()));
    chats.set_official_bots(ModelRc::from(models.official_bots.clone()));
    chats.set_bots(ModelRc::from(models.bots.clone()));
    let conversation = ui.global::<Conversation>();
    conversation.set_tabs(ModelRc::from(models.tabs.clone()));
    conversation.set_messages(ModelRc::from(models.messages.clone()));

    STORE.with(|store| {
        *store.borrow_mut() = Some(Store {
            ui: ui.as_weak(),
            my_id: 0,
            my_bio: String::new(),
            chats: HashMap::new(),
            users: HashMap::new(),
            basic_groups: HashMap::new(),
            supergroups: HashMap::new(),
            supergroup_members: HashMap::new(),
            online_members: HashMap::new(),
            scope_settings: HashMap::new(),
            folders: Vec::new(),
            shown_folder: 0,
            expanded: HashMap::new(),
            tabs: Vec::new(),
            open: None,
            histories: HashMap::new(),
            typing: HashMap::new(),
            sponsored: HashMap::new(),
            pictures: images::Cache::new(PICTURES_KEPT),
            previews: RefCell::new(images::Cache::new(PREVIEWS_KEPT)),
            avatars: images::Cache::new(AVATARS_KEPT),
            avatars_asked: HashSet::new(),
            wanted_photos: RefCell::new(Vec::new()),
            is_premium: false,
            caption_length_max: 1024,
            basic_group_size_max: 200,
            transfers: HashMap::new(),
            row_files: RefCell::new(HashSet::new()),
            media_models: RefCell::new(HashMap::new()),
            blank: Image::from_rgba8(SharedPixelBuffer::new(1, 1)),
            rows: Vec::new(),
            selected: HashSet::new(),
            replied: HashMap::new(),
            missing_replies: Vec::new(),
            asking_replies: HashSet::new(),
            pinned: HashMap::new(),
            keyboards: HashMap::new(),
            keyboard_hidden: HashSet::new(),
            bot_infos: HashMap::new(),
            group_commands: HashMap::new(),
            command_filter: None,
            button_models: RefCell::new(HashMap::new()),
            keyboard_model: RefCell::new(None),
            commands_shown: RefCell::new(Vec::new()),
            default_reaction: api::ReactionType::Emoji { emoji: "❤️".into() },
            custom_emoji: HashMap::new(),
            custom_emoji_wanted: RefCell::new(Vec::new()),
            topics: HashMap::new(),
            topics_loading: HashSet::new(),
            open_topic: None,
            topic_icons: Vec::new(),
            supergroup_full: HashMap::new(),
            slow_until: HashMap::new(),
            admin_busy: String::new(),
            admin_error: String::new(),
            drafts: HashMap::new(),
            added_contacts: HashSet::new(),
            round_playing: None,
            round_loading: None,
            reaction_models: RefCell::new(HashMap::new()),
            topics_shown: RefCell::new(Vec::new()),
            dirty: Dirty::default(),
            models,
        })
    });
}

/// Run `change` with the store, if it is installed.
pub fn with<R>(change: impl FnOnce(&mut Store) -> R) -> Option<R> {
    STORE.with(|store| store.borrow_mut().as_mut().map(change))
}

/// Forget the account (it logged out): every chat, user and message, and what the pages show.
pub fn clear() {
    with(|store| {
        store.my_id = 0;
        store.my_bio.clear();
        store.chats.clear();
        store.users.clear();
        store.basic_groups.clear();
        store.supergroups.clear();
        store.supergroup_members.clear();
        store.online_members.clear();
        store.scope_settings.clear();
        store.folders.clear();
        store.shown_folder = 0;
        store.expanded.clear();
        store.tabs.clear();
        store.open = None;
        store.histories.clear();
        store.typing.clear();
        store.sponsored.clear();
        store.pictures.clear();
        store.previews.borrow_mut().clear();
        store.avatars.clear();
        store.avatars_asked.clear();
        store.wanted_photos.borrow_mut().clear();
        store.media_models.borrow_mut().clear();
        store.rows.clear();
        store.selected.clear();
        store.replied.clear();
        store.missing_replies.clear();
        store.asking_replies.clear();
        store.pinned.clear();
        store.keyboards.clear();
        store.keyboard_hidden.clear();
        store.bot_infos.clear();
        store.group_commands.clear();
        store.command_filter = None;
        store.button_models.borrow_mut().clear();
        *store.keyboard_model.borrow_mut() = None;
        store.commands_shown.borrow_mut().clear();
        store.custom_emoji.clear();
        store.custom_emoji_wanted.borrow_mut().clear();
        store.topics.clear();
        store.topics_loading.clear();
        store.open_topic = None;
        store.topic_icons.clear();
        store.supergroup_full.clear();
        store.slow_until.clear();
        store.admin_busy.clear();
        store.admin_error.clear();
        store.drafts.clear();
        store.added_contacts.clear();
        store.round_playing = None;
        store.round_loading = None;
        store.reaction_models.borrow_mut().clear();
        store.topics_shown.borrow_mut().clear();
        store.dirty = Dirty {
            chats: true,
            conversation: true,
            header: true,
            account: true,
            viewer: true,
            notification_scopes: true,
            scroll_to_end: false,
            topics: true,
        };
    });
    refresh();
}

/// Bring the pages' models up to date with what changed.
pub fn refresh() {
    let Some(ui) = STORE.with(|store| store.borrow().as_ref().and_then(|store| store.ui.upgrade())) else {
        return;
    };
    let words = Names::from(&ui);
    let followups = with(|store| {
        let dirty = std::mem::take(&mut store.dirty);
        if dirty.chats {
            store.refresh_chats(&ui, &words);
        }
        // The header shows the chat as the lists do (its name, its mute, who is online): it follows
        // them; the message rows only follow the messages.
        if dirty.chats || dirty.header || dirty.conversation || dirty.topics {
            store.refresh_conversation(&ui, &words, dirty.conversation, dirty.scroll_to_end);
        }
        if dirty.account {
            store.refresh_account(&ui);
        }
        if dirty.notification_scopes {
            store.refresh_notification_scopes(&ui);
        }
        let mut close_viewer = false;
        if dirty.viewer || dirty.conversation {
            let viewer = ui.global::<Viewer>();
            if viewer.get_open() {
                let shown = usize::try_from(viewer.get_index()).ok().and_then(|index| viewer.get_items().row_data(index));
                match shown {
                    // A self-destructing photo or video is shown on its own until the viewer is
                    // closed (viewer.rs), or until its time runs out.
                    Some(item) if item.secret => close_viewer = store.secret_timed_out(&item.id),
                    _ => viewer.set_items(ModelRc::new(VecModel::from(store.viewer_items(&words)))),
                }
            }
        }
        (std::mem::take(&mut store.missing_replies), store.take_wanted_photos(), close_viewer, store.custom_emoji_wanted.take())
    });
    let (missing, wanted, close_viewer, custom_emoji) = followups.unwrap_or_default();
    if close_viewer {
        super::viewer::close();
    }
    if !missing.is_empty() {
        super::conversation::load_replied(missing);
    }
    if !wanted.is_empty() {
        avatars::fetch(wanted);
    }
    if !custom_emoji.is_empty() {
        super::reactions::fetch_custom_emoji(custom_emoji);
    }
}

/// Download the photos that rows built outside a refresh wanted (the forward picker's chats).
pub fn fetch_wanted_photos() {
    let wanted = with(|store| store.take_wanted_photos()).unwrap_or_default();
    if !wanted.is_empty() {
        avatars::fetch(wanted);
    }
}

/// Build everything again: the UI language changed, and with it the names Rust puts in rows.
pub fn refresh_all() {
    with(|store| {
        store.dirty.chats = true;
        store.dirty.conversation = true;
        store.dirty.account = true;
    });
    refresh();
}

/// The names Telegram does not give, in the UI language (the Words global), and how the pages
/// want the chat lists ordered.
pub struct Names {
    pub saved_messages: String,
    deleted_account: String,
    all_chats: String,
    pub list_separator: String,
    /// How a folder's chats are ordered under the pinned ones (Chats.order), and the order of
    /// names in the UI language, for the order by name (and for the lists of contacts).
    order: ChatOrder,
    pub name_order: NameOrder,
}

impl Names {
    pub fn from(ui: &MainWindow) -> Names {
        let words = ui.global::<Words>();
        Names {
            saved_messages: words.get_saved_messages().into(),
            deleted_account: words.get_deleted_account().into(),
            all_chats: words.get_all_chats().into(),
            list_separator: words.get_list_separator().into(),
            order: ui.global::<Chats>().get_order(),
            name_order: NameOrder::new(&ui.global::<AppState>().get_language()),
        }
    }
}

/// What a chat is ordered by in a list (chats_in).
struct SortKey {
    pinned: bool,
    order: i64,             // TDLib's: the newest message first, the pinned ones above
    id: i64,
    title: String,
    unread: i32,            // 0 for a muted chat, whatever it has
}

/// The order of two chats in a list: the pinned ones first, in Telegram's order; then the rest as
/// chosen, Telegram's order (the newest message first) breaking ties.
fn compare(a: &SortKey, b: &SortKey, order: ChatOrder, names: &NameOrder) -> Ordering {
    let telegram = b.order.cmp(&a.order).then(b.id.cmp(&a.id));
    b.pinned.cmp(&a.pinned).then_with(|| {
        if a.pinned {
            return telegram;
        }
        match order {
            ChatOrder::LatestMessage => telegram,
            ChatOrder::Name => names.compare(&a.title, &b.title).then(telegram),
            ChatOrder::Unread => b.unread.cmp(&a.unread).then(telegram),
        }
    })
}

/// Replace the rows of `model` with `rows`, touching only the rows that changed when the number of
/// rows stays the same. For the chat lists, which are ListViews: when the number changes they make
/// their visible rows again, and that is cheap.
fn sync<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    if model.row_count() != rows.len() {
        model.set_vec(rows);
        return;
    }
    for (index, row) in rows.into_iter().enumerate() {
        if model.row_data(index).as_ref() != Some(&row) {
            model.set_row_data(index, row);
        }
    }
}

/// Replace the rows of `model` with `rows` by their keys, which both have in the same order: a row
/// whose key stayed is kept (set again only when it changed), one whose key went is taken out, and
/// one with a new key is put in where it belongs. For the messages and the tabs, which the pages
/// lay out whole, every row an item of its own: a reset would make every item again at each new
/// message, and a message's item is a large one (its words, pictures, quote, preview, menu).
fn sync_keyed<T: Clone + PartialEq + 'static, K: Eq + Hash>(model: &VecModel<T>, rows: Vec<T>, key: impl Fn(&T) -> K) {
    let old: Vec<T> = model.iter().collect();
    if old.is_empty() || rows.is_empty() {
        model.set_vec(rows);
        return;
    }
    let old_keys: Vec<K> = old.iter().map(&key).collect();
    let new_keys: Vec<K> = rows.iter().map(&key).collect();
    let wanted: HashSet<&K> = new_keys.iter().collect();
    // New rows put in so far: an old row with one of their keys has moved, and goes.
    let mut placed: HashSet<&K> = HashSet::new();
    let mut next_old = 0;
    let mut at = 0;
    for (row, new_key) in rows.into_iter().zip(&new_keys) {
        while next_old < old.len() && (!wanted.contains(&old_keys[next_old]) || placed.contains(&old_keys[next_old])) {
            model.remove(at);
            next_old += 1;
        }
        if next_old < old.len() && old_keys[next_old] == *new_key {
            if old[next_old] != row {
                model.set_row_data(at, row);
            }
            next_old += 1;
        } else {
            model.insert(at, row);
        }
        placed.insert(new_key);
        at += 1;
    }
    while next_old < old.len() {
        model.remove(at);
        next_old += 1;
    }
}

/// What tells a message row from the others, from one refresh to the next.
#[derive(PartialEq, Eq, Hash)]
enum RowKey {
    Day(i32, i32, i32),
    Message(SharedString),
    Sponsored(SharedString),
}

fn row_key(row: &MessageRow) -> RowKey {
    match row.kind {
        RowKind::Day => RowKey::Day(row.day.year, row.day.month, row.day.date),
        RowKind::Sponsored => RowKey::Sponsored(row.id.clone()),
        RowKind::Message | RowKind::Service => RowKey::Message(row.id.clone()),
    }
}

/// The first letter of a name, for its letter square.
pub fn initial(name: &str) -> SharedString {
    name.chars().find(|c| !c.is_whitespace()).map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default().into()
}

/// Which file a photo is, to tell a changed photo from the same one.
fn photo_file(photo: Option<&api::ChatPhoto>) -> Option<i32> {
    photo.map(|photo| photo.small.id)
}

/// A colour for a sender, the same every time (Terminal's names).
fn colour_of(sender: &MessageSender) -> i32 {
    let id = match sender {
        MessageSender::User { user_id } => *user_id,
        MessageSender::Chat { chat_id } => *chat_id,
    };
    id.rem_euclid(8) as i32
}

impl Store {
    // ---- updates ------------------------------------------------------------------------------

    /// Take in an update. Returns what it asks for beyond the store: see [`Followup`].
    pub fn apply(&mut self, update: Update) -> Followup {
        let mut followup = Followup::None;
        match update {
            Update::Option { name, value } => match (name.as_str(), value) {
                ("my_id", api::OptionValue::Integer { value }) => {
                    self.my_id = value;
                    self.dirty.chats = true;
                }
                ("is_premium", api::OptionValue::Boolean { value }) => self.is_premium = value,
                ("message_caption_length_max", api::OptionValue::Integer { value }) => {
                    self.caption_length_max = i32::try_from(value).unwrap_or(1024);
                }
                ("basic_group_size_max", api::OptionValue::Integer { value }) => {
                    self.basic_group_size_max = i32::try_from(value).unwrap_or(200);
                }
                _ => {}
            },
            Update::User { user } => {
                if user.id == self.my_id {
                    self.dirty.account = true;
                }
                // The message rows name their senders and show their photos: a name or a photo
                // that changed shows in them.
                let changed = self.users.get(&user.id).is_none_or(|known| {
                    known.first_name != user.first_name
                        || known.last_name != user.last_name
                        || known.kind != user.kind
                        || photo_file(known.profile_photo.as_ref()) != photo_file(user.profile_photo.as_ref())
                });
                self.users.insert(user.id, user);
                self.dirty.chats = true;
                self.dirty.conversation |= changed && self.open.is_some();
            }
            Update::UserStatus { user_id, status } => {
                if let Some(user) = self.users.get_mut(&user_id) {
                    user.status = status;
                    self.dirty.chats = true;
                }
            }
            Update::UserFullInfo { user_id, user_full_info } => {
                if user_id == self.my_id {
                    self.my_bio = user_full_info.bio.map(|bio| bio.text).unwrap_or_default();
                    self.dirty.account = true;
                }
                // A bot's menu button and commands, for the composer.
                if let Some(bot_info) = user_full_info.bot_info {
                    self.bot_infos.insert(user_id, bot_info);
                    self.dirty.header |= self.open.and_then(|id| self.chats.get(&id)).is_some_and(|chat| {
                        matches!(chat.kind, ChatType::Private { user_id: bot } if bot == user_id)
                    });
                }
            }
            Update::BasicGroup { basic_group } => {
                self.basic_groups.insert(basic_group.id, basic_group);
                self.dirty.chats = true;
            }
            Update::Supergroup { supergroup } => {
                self.supergroups.insert(supergroup.id, supergroup);
                self.dirty.chats = true;
                // Our status in it, its joining rule: the settings panel.
                self.dirty.header = true;
            }
            Update::SupergroupFullInfo { supergroup_id, supergroup_full_info } => {
                self.supergroup_members.insert(supergroup_id, supergroup_full_info.member_count);
                let open = self.open.and_then(|id| self.chats.get(&id)).is_some_and(|chat| {
                    matches!(chat.kind, ChatType::Supergroup { supergroup_id: id, .. } if id == supergroup_id)
                });
                self.supergroup_full.insert(supergroup_id, supergroup_full_info);
                self.dirty.chats = true;
                if open {
                    // Slow mode and the settings panel follow it (admin.rs).
                    self.dirty.header = true;
                    followup = Followup::SlowModeChanged;
                }
            }
            Update::NewChat { chat } => {
                self.chats.insert(chat.id, *chat);
                self.dirty.chats = true;
                // A forwarded message names the chat it came from.
                self.dirty.conversation |= self.open.is_some();
            }
            Update::ChatTitle { chat_id, title } => {
                if let Some(chat) = self.chats.get_mut(&chat_id) {
                    chat.title = title;
                    self.dirty.chats = true;
                    self.dirty.conversation |= self.open.is_some();
                }
            }
            Update::ChatPhoto { chat_id, photo } => {
                if let Some(chat) = self.chats.get_mut(&chat_id) {
                    chat.photo = photo;
                    self.dirty.chats = true;
                    self.dirty.account |= chat_id == self.my_id;
                    // A message sent in the chat's name (a channel's post, an anonymous admin's)
                    // shows the chat's photo.
                    self.dirty.conversation |= self.open.is_some();
                }
            }
            Update::ChatPermissions { chat_id, permissions } => {
                if let Some(chat) = self.chats.get_mut(&chat_id) {
                    chat.permissions = permissions;
                    self.dirty.header = true;
                }
            }
            Update::ChatLastMessage { chat_id, last_message, positions } => {
                if let Some(chat) = self.chats.get_mut(&chat_id) {
                    chat.last_message = last_message;
                    for position in positions {
                        set_position(chat, position);
                    }
                    self.dirty.chats = true;
                }
            }
            Update::ChatPosition { chat_id, position } => {
                if let Some(chat) = self.chats.get_mut(&chat_id) {
                    set_position(chat, position);
                    self.dirty.chats = true;
                }
            }
            Update::ChatDraftMessage { chat_id, draft_message, positions } => {
                if let Some(chat) = self.chats.get_mut(&chat_id) {
                    chat.draft_message = draft_message;
                    for position in positions {
                        set_position(chat, position);
                    }
                    // Telegram has it now: what was kept here goes.
                    self.drafts.remove(&chat_id);
                    self.dirty.chats = true;
                    if self.open == Some(chat_id) {
                        // Written on another device: the composer follows (drafts.rs).
                        followup = Followup::DraftChanged { chat_id };
                    }
                }
            }
            Update::ChatReadInbox { chat_id, last_read_inbox_message_id, unread_count } => {
                if let Some(chat) = self.chats.get_mut(&chat_id) {
                    chat.last_read_inbox_message_id = last_read_inbox_message_id;
                    chat.unread_count = unread_count;
                    self.dirty.chats = true;
                }
            }
            Update::ChatReadOutbox { chat_id, last_read_outbox_message_id } => {
                if let Some(chat) = self.chats.get_mut(&chat_id) {
                    chat.last_read_outbox_message_id = last_read_outbox_message_id;
                    self.dirty.conversation = true;
                }
            }
            Update::ChatUnreadMentionCount { chat_id, unread_mention_count }
            | Update::MessageMentionRead { chat_id, unread_mention_count } => {
                if let Some(chat) = self.chats.get_mut(&chat_id) {
                    chat.unread_mention_count = unread_mention_count;
                    self.dirty.chats = true;
                }
            }
            Update::ChatNotificationSettings { chat_id, notification_settings } => {
                if let Some(chat) = self.chats.get_mut(&chat_id) {
                    chat.notification_settings = notification_settings;
                    self.dirty.chats = true;
                }
            }
            Update::ScopeNotificationSettings { scope, notification_settings } => {
                self.scope_settings.insert(scope, notification_settings);
                self.dirty.chats = true;
                self.dirty.notification_scopes = true;
            }
            Update::ChatIsMarkedAsUnread { chat_id, is_marked_as_unread } => {
                if let Some(chat) = self.chats.get_mut(&chat_id) {
                    chat.is_marked_as_unread = is_marked_as_unread;
                    self.dirty.chats = true;
                }
            }
            Update::ChatBlockList { chat_id, block_list } => {
                if let Some(chat) = self.chats.get_mut(&chat_id) {
                    chat.block_list = block_list;
                    self.dirty.chats = true;
                }
            }
            Update::ChatFolders { chat_folders } => {
                self.folders = chat_folders;
                if self.shown_folder > self.folders.len() {
                    self.shown_folder = 0;
                }
                self.dirty.chats = true;
                followup = Followup::LoadFolders;
            }
            Update::ChatOnlineMemberCount { chat_id, online_member_count } => {
                self.online_members.insert(chat_id, online_member_count);
                self.dirty.header = true;
            }
            Update::ChatAction { chat_id, sender_id, action } => {
                let typing = self.typing.entry(chat_id).or_default();
                typing.retain(|(sender, _)| *sender != sender_id);
                if action == api::ChatAction::Typing {
                    typing.push((sender_id, Instant::now()));
                    followup = Followup::TypingExpires;
                }
                if self.open == Some(chat_id) {
                    self.dirty.header = true;
                }
            }
            Update::NewMessage { message } => {
                let chat_id = message.chat_id;
                let topic = message.forum_topic_id();
                // Someone who sends a message has stopped typing.
                if let Some(typing) = self.typing.get_mut(&chat_id) {
                    typing.retain(|(sender, _)| *sender != message.sender_id);
                }
                // In a group with topics, a message of another topic than the rows' leaves them
                // alone; its topic's row in the list is fetched anew (topics.rs).
                let other_topic = self.histories.get(&chat_id).and_then(|history| history.topic_id).is_some_and(|open| topic != Some(open))
                    || (!self.histories.contains_key(&chat_id) && self.topics.contains_key(&chat_id));
                if other_topic {
                    if let Some(topic_id) = topic {
                        followup = Followup::TopicChanged { chat_id, topic_id };
                    }
                    return followup;
                }
                if let Some(topic_id) = topic.filter(|_| self.topics.contains_key(&chat_id)) {
                    followup = Followup::TopicChanged { chat_id, topic_id };
                }
                // Rows around an older message leave it out: it joins them with the end of the chat.
                if let Some(history) = self.histories.get_mut(&chat_id).filter(|history| !history.has_newer) {
                    let outgoing = message.is_outgoing;
                    let id = message.id;
                    if history.messages.insert(id, *message).is_none() && history.shown > SHOWN {
                        // The view is up among older messages: the rows it shows stay.
                        history.shown += 1;
                    }
                    if self.open == Some(chat_id) {
                        self.dirty.conversation = true;
                        if outgoing {
                            self.dirty.scroll_to_end = true;
                        } else {
                            followup = Followup::View { chat_id, message_ids: vec![id] };
                        }
                    }
                }
            }
            Update::MessageSendSucceeded { message, old_message_id } | Update::MessageSendFailed { message, old_message_id } => {
                let chat_id = message.chat_id;
                if let Some(history) = self.histories.get_mut(&chat_id) {
                    history.messages.remove(&old_message_id);
                    history.messages.insert(message.id, *message);
                    self.dirty.conversation |= self.open == Some(chat_id);
                }
            }
            Update::MessageContent { chat_id, message_id, new_content } => {
                if let Some(message) = self.histories.get_mut(&chat_id).and_then(|history| history.messages.get_mut(&message_id)) {
                    message.content = new_content;
                    self.dirty.conversation |= self.open == Some(chat_id);
                }
            }
            Update::MessageEdited { chat_id, message_id, edit_date, reply_markup } => {
                if let Some(message) = self.histories.get_mut(&chat_id).and_then(|history| history.messages.get_mut(&message_id)) {
                    message.edit_date = edit_date;
                    // The buttons under it changed, or went.
                    message.reply_markup = reply_markup;
                    self.dirty.conversation |= self.open == Some(chat_id);
                }
            }
            Update::MessageIsPinned { chat_id, message_id, is_pinned } => {
                if let Some(message) = self.histories.get_mut(&chat_id).and_then(|history| history.messages.get_mut(&message_id)) {
                    message.is_pinned = is_pinned;
                }
                // The bar under the header shows the newest pinned message: asked again (bots.rs).
                if self.open == Some(chat_id) {
                    followup = Followup::PinnedChanged { chat_id };
                }
            }
            Update::ChatReplyMarkup { chat_id, reply_markup_message_id } => {
                if let Some(chat) = self.chats.get_mut(&chat_id) {
                    chat.reply_markup_message_id = reply_markup_message_id;
                }
                // A new keyboard shows, even where the user had folded the last away; the message
                // it is on is fetched anew (bots.rs) when the chat is in front.
                self.keyboard_hidden.remove(&chat_id);
                self.keyboards.remove(&chat_id);
                if reply_markup_message_id == 0 {
                    self.keyboards.insert(chat_id, None);
                }
                if self.open == Some(chat_id) {
                    self.dirty.header = true;
                    followup = Followup::KeyboardChanged { chat_id };
                }
            }
            Update::ChatHasProtectedContent { chat_id, has_protected_content } => {
                if let Some(chat) = self.chats.get_mut(&chat_id) {
                    chat.has_protected_content = has_protected_content;
                    if self.open == Some(chat_id) {
                        self.dirty.header = true;
                        followup = Followup::ProtectionChanged;
                    }
                }
            }
            Update::MessageInteractionInfo { chat_id, message_id, interaction_info } => {
                if let Some(message) = self.histories.get_mut(&chat_id).and_then(|history| history.messages.get_mut(&message_id)) {
                    message.interaction_info = interaction_info;
                    self.dirty.conversation |= self.open == Some(chat_id);
                }
            }
            Update::ChatAvailableReactions { chat_id, available_reactions } => {
                if let Some(chat) = self.chats.get_mut(&chat_id) {
                    chat.available_reactions = Some(available_reactions);
                    self.dirty.header |= self.open == Some(chat_id);
                }
            }
            Update::DefaultReactionType { reaction_type } => self.default_reaction = reaction_type,
            Update::ForumTopicInfo { info } => {
                let chat_id = info.chat_id;
                let topic_id = info.forum_topic_id;
                let open = self.open == Some(chat_id);
                let listed = self.topics.contains_key(&chat_id);
                let known = self.topics.get_mut(&chat_id).and_then(|topics| topics.iter_mut().find(|topic| topic.info.forum_topic_id == topic_id));
                match known {
                    Some(topic) => {
                        topic.info = info;
                        if open {
                            self.dirty.topics = true;
                            self.dirty.header = true;
                        }
                    }
                    // A new topic: its row is fetched whole (topics.rs).
                    None if listed => followup = Followup::TopicChanged { chat_id, topic_id },
                    None => {}
                }
            }
            Update::ForumTopic { chat_id, forum_topic_id, is_pinned, last_read_inbox_message_id, unread_mention_count } => {
                if let Some(topic) = self.topics.get_mut(&chat_id).and_then(|topics| topics.iter_mut().find(|topic| topic.info.forum_topic_id == forum_topic_id)) {
                    topic.is_pinned = is_pinned;
                    topic.last_read_inbox_message_id = last_read_inbox_message_id;
                    topic.unread_mention_count = unread_mention_count;
                    // Its unread count is not in the update: the topic is fetched anew.
                    followup = Followup::TopicChanged { chat_id, topic_id: forum_topic_id };
                }
            }
            Update::DeleteMessages { chat_id, message_ids, is_permanent } => {
                if is_permanent {
                    if let Some(history) = self.histories.get_mut(&chat_id) {
                        for id in &message_ids {
                            history.messages.remove(id);
                        }
                        self.dirty.conversation |= self.open == Some(chat_id);
                    }
                    // Replies to them say so.
                    for id in &message_ids {
                        self.replied.insert((chat_id, *id), None);
                    }
                    followup = Followup::Deleted { chat_id, message_ids };
                }
            }
            Update::AuthorizationState { .. }
            | Update::ConnectionState { .. }
            | Update::File { .. }
            | Update::NotificationGroup { .. }
            | Update::ActiveNotifications { .. }
            | Update::UnreadMessageCount { .. }
            | Update::Other => {}
        }
        followup
    }

    // ---- what a chat is -----------------------------------------------------------------------

    pub fn kind(&self, chat: &api::Chat) -> ChatKind {
        match chat.kind {
            ChatType::Private { user_id } | ChatType::Secret { user_id } => {
                if user_id == self.my_id {
                    ChatKind::Saved
                } else if self.users.get(&user_id).is_some_and(|user| user.kind == UserType::Bot) {
                    ChatKind::Bot
                } else {
                    ChatKind::User
                }
            }
            ChatType::BasicGroup { .. } | ChatType::Supergroup { is_channel: false, .. } => ChatKind::Group,
            ChatType::Supergroup { is_channel: true, .. } => ChatKind::Channel,
        }
    }

    /// The chat's name as the pages show it.
    pub fn title(&self, chat: &api::Chat, names: &Names) -> String {
        match chat.kind {
            ChatType::Private { user_id } | ChatType::Secret { user_id } => {
                if user_id == self.my_id {
                    return names.saved_messages.clone();
                }
                if self.users.get(&user_id).is_some_and(|user| user.kind == UserType::Deleted) {
                    return names.deleted_account.clone();
                }
                chat.title.clone()
            }
            _ => chat.title.clone(),
        }
    }

    fn user_of(&self, chat: &api::Chat) -> Option<&api::User> {
        match chat.kind {
            ChatType::Private { user_id } | ChatType::Secret { user_id } => self.users.get(&user_id),
            _ => None,
        }
    }

    fn supergroup_of(&self, chat: &api::Chat) -> Option<&api::Supergroup> {
        match chat.kind {
            ChatType::Supergroup { supergroup_id, .. } => self.supergroups.get(&supergroup_id),
            _ => None,
        }
    }

    pub fn muted(&self, chat: &api::Chat) -> bool {
        let settings = &chat.notification_settings;
        if !settings.use_default_mute_for {
            return settings.mute_for > 0;
        }
        self.scope_muted(scope_of(chat))
    }

    /// Whether the chats of a kind are muted, those that do not say otherwise.
    pub fn scope_muted(&self, scope: NotificationSettingsScope) -> bool {
        self.scope_settings.get(&scope).is_some_and(|settings| settings.mute_for > 0)
    }

    pub fn members(&self, chat: &api::Chat) -> i32 {
        match chat.kind {
            ChatType::BasicGroup { basic_group_id } => self.basic_groups.get(&basic_group_id).map_or(0, |group| group.member_count),
            ChatType::Supergroup { supergroup_id, .. } => self
                .supergroup_members
                .get(&supergroup_id)
                .copied()
                .filter(|count| *count > 0)
                .or_else(|| self.supergroups.get(&supergroup_id).map(|group| group.member_count))
                .unwrap_or(0),
            _ => 0,
        }
    }

    /// What the chat lets us send besides words: its permissions, and ours in it when we are
    /// restricted. Channels take posts from their administrators only.
    pub fn send_rights(&self, chat: &api::Chat) -> SendRights {
        let of = |permissions: &api::ChatPermissions| SendRights {
            photos: permissions.can_send_photos,
            videos: permissions.can_send_videos,
            documents: permissions.can_send_documents,
        };
        let everything = SendRights { photos: true, videos: true, documents: true };
        let nothing = SendRights::default();
        if !self.can_write(chat) {
            return nothing;
        }
        match chat.kind {
            ChatType::Private { .. } | ChatType::Secret { .. } => everything,
            ChatType::BasicGroup { basic_group_id } => match self.basic_groups.get(&basic_group_id).map(|group| &group.status) {
                Some(ChatMemberStatus::Creator | ChatMemberStatus::Administrator { .. }) => everything,
                Some(ChatMemberStatus::Member) => of(&chat.permissions),
                Some(ChatMemberStatus::Restricted { permissions, .. }) => of(permissions).and(&of(&chat.permissions)),
                _ => nothing,
            },
            ChatType::Supergroup { supergroup_id, is_channel } => {
                match self.supergroups.get(&supergroup_id).map(|group| &group.status) {
                    Some(ChatMemberStatus::Creator | ChatMemberStatus::Administrator { .. }) => everything,
                    Some(ChatMemberStatus::Member) if !is_channel => of(&chat.permissions),
                    Some(ChatMemberStatus::Restricted { permissions, .. }) if !is_channel => of(permissions).and(&of(&chat.permissions)),
                    _ => nothing,
                }
            }
        }
    }

    /// A file moved (updateFile): what the rows show of it follows.
    pub fn file_progress(&mut self, file: &api::File) {
        let moving = if file.remote.is_uploading_active && !file.remote.is_uploading_completed {
            Some((file.remote.uploaded_size, file.size_or_expected()))
        } else if file.local.is_downloading_active && !file.local.is_downloading_completed {
            Some((file.local.downloaded_size, file.size_or_expected()))
        } else {
            None
        };
        let changed = match moving {
            Some(progress) => self.transfers.insert(file.id, progress) != Some(progress),
            None => self.transfers.remove(&file.id).is_some(),
        };
        if changed && self.row_files.borrow().contains(&file.id) {
            self.dirty.conversation = true;
        }
    }

    /// Whether we may write in the chat.
    pub fn can_write(&self, chat: &api::Chat) -> bool {
        match chat.kind {
            ChatType::Private { user_id } | ChatType::Secret { user_id } => {
                self.users.get(&user_id).is_none_or(|user| user.kind != UserType::Deleted)
            }
            ChatType::BasicGroup { basic_group_id } => match self.basic_groups.get(&basic_group_id).map(|group| &group.status) {
                Some(ChatMemberStatus::Creator | ChatMemberStatus::Administrator { .. }) => true,
                Some(ChatMemberStatus::Member) => chat.permissions.can_send_basic_messages,
                _ => false,
            },
            ChatType::Supergroup { supergroup_id, is_channel } => {
                match self.supergroups.get(&supergroup_id).map(|group| &group.status) {
                    Some(ChatMemberStatus::Creator) => true,
                    Some(ChatMemberStatus::Administrator { rights }) => !is_channel || rights.can_post_messages,
                    Some(ChatMemberStatus::Member) => !is_channel && chat.permissions.can_send_basic_messages,
                    Some(ChatMemberStatus::Restricted { is_member, permissions }) => {
                        !is_channel && *is_member && permissions.can_send_basic_messages
                    }
                    _ => false,
                }
            }
        }
    }

    /// Who sent a message: a user's full name, or the chat's title.
    pub fn sender_name(&self, sender: &MessageSender, names: &Names) -> String {
        match sender {
            MessageSender::User { user_id } => self.user_name(*user_id, names),
            MessageSender::Chat { chat_id } => {
                self.chats.get(chat_id).map(|chat| self.title(chat, names)).unwrap_or_default()
            }
        }
    }

    pub fn user_name(&self, user_id: i64, names: &Names) -> String {
        match self.users.get(&user_id) {
            Some(user) if user.kind == UserType::Deleted => names.deleted_account.clone(),
            Some(user) => format!("{} {}", user.first_name, user.last_name).trim().to_string(),
            None => String::new(),
        }
    }

    /// The short name the chat list puts before a group's last message.
    fn sender_first_name(&self, sender: &MessageSender, names: &Names) -> String {
        match sender {
            MessageSender::User { user_id } => match self.users.get(user_id) {
                Some(user) if !user.first_name.is_empty() => user.first_name.clone(),
                _ => self.user_name(*user_id, names),
            },
            MessageSender::Chat { .. } => self.sender_name(sender, names),
        }
    }

    /// What a message holds: its kind, its words, and the detail the kind needs.
    pub fn content(&self, message: &api::Message, names: &Names) -> (Content, String, String) {
        self.content_of(&message.content, &message.sender_id, names)
    }

    /// The same for a message's content alone; `sender` is who sent it (who joined, who left).
    fn content_of(&self, content: &M, sender: &MessageSender, names: &Names) -> (Content, String, String) {
        let none = String::new;
        match content {
            M::Text { text, .. } => (Content::Text, text.text.clone(), none()),
            M::AnimatedEmoji { emoji } => (Content::Text, emoji.clone(), none()),
            M::Photo { caption, .. } => (Content::Photo, caption.text.clone(), none()),
            M::Video { caption, .. } => (Content::Video, caption.text.clone(), none()),
            M::Animation { caption, .. } => (Content::Animation, caption.text.clone(), none()),
            M::Audio { audio, caption } => {
                let name = match (audio.performer.is_empty(), audio.title.is_empty()) {
                    (false, false) => format!("{} – {}", audio.performer, audio.title),
                    (_, false) => audio.title.clone(),
                    _ => audio.file_name.clone(),
                };
                (Content::Audio, caption.text.clone(), name)
            }
            M::Document { document, caption } => (Content::Document, caption.text.clone(), document.file_name.clone()),
            M::VoiceNote { caption } => (Content::VoiceNote, caption.text.clone(), none()),
            M::VideoNote { .. } => (Content::VideoNote, none(), none()),
            M::ExpiredPhoto {} => (Content::ExpiredPhoto, none(), none()),
            M::ExpiredVideo {} => (Content::ExpiredVideo, none(), none()),
            M::ExpiredVideoNote {} => (Content::ExpiredVideoNote, none(), none()),
            M::ExpiredVoiceNote {} => (Content::ExpiredVoiceNote, none(), none()),
            M::Sticker { sticker } => (Content::Sticker, none(), sticker.emoji.clone()),
            M::Dice { emoji } => (Content::Dice, none(), emoji.clone()),
            M::Location {} => (Content::Location, none(), none()),
            M::Venue { venue } => (Content::Venue, none(), venue.title.clone()),
            M::Contact { contact } => (Content::Contact, none(), contact.name()),
            M::Poll { poll } => (Content::Poll, none(), poll.question.text.clone()),
            M::Game { game } => (Content::Game, none(), game.title.clone()),
            M::Invoice {} => (Content::Invoice, none(), none()),
            M::Call {} => (Content::Call, none(), none()),
            M::Story {} => (Content::Story, none(), none()),
            M::Gift {} => (Content::Gift, none(), none()),
            M::BasicGroupChatCreate { title } => (Content::ChatCreated, none(), title.clone()),
            M::SupergroupChatCreate { title } => {
                let channel = matches!(sender, MessageSender::Chat { chat_id } if self.chats.get(chat_id).is_some_and(|chat| self.kind(chat) == ChatKind::Channel));
                (if channel { Content::ChannelCreated } else { Content::ChatCreated }, none(), title.clone())
            }
            M::ChatChangeTitle { title } => (Content::TitleChanged, none(), title.clone()),
            M::ChatChangePhoto {} => (Content::PhotoChanged, none(), none()),
            M::ChatDeletePhoto {} => (Content::PhotoRemoved, none(), none()),
            M::ChatAddMembers { member_user_ids } => {
                if matches!(sender, MessageSender::User { user_id } if member_user_ids == &[*user_id]) {
                    (Content::MemberJoined, none(), none())
                } else {
                    let added: Vec<String> = member_user_ids.iter().map(|id| self.user_name(*id, names)).collect();
                    (Content::MembersAdded, none(), added.join(", "))
                }
            }
            M::ChatJoinByLink {} | M::ChatJoinByRequest {} => (Content::MemberJoined, none(), none()),
            M::ChatDeleteMember { user_id } => {
                if matches!(sender, MessageSender::User { user_id: sender } if sender == user_id) {
                    (Content::MemberLeft, none(), none())
                } else {
                    (Content::MemberRemoved, none(), self.user_name(*user_id, names))
                }
            }
            M::PinMessage {} => (Content::MessagePinned, none(), none()),
            M::ScreenshotTaken {} => (Content::ScreenshotTaken, none(), none()),
            M::ContactRegistered {} => (Content::ContactJoined, none(), none()),
            M::Other => (Content::Unsupported, none(), none()),
        }
    }

    // ---- rows ---------------------------------------------------------------------------------

    /// A chat as a row of a list, `list` being the list it is shown in (for its pin).
    pub fn chat_row(&self, chat: &api::Chat, list: ChatList, names: &Names) -> ChatRow {
        let kind = self.kind(chat);
        let title = self.title(chat, names);
        let last = chat.last_message.as_deref();
        let (content, text, detail) = last.map(|message| self.content(message, names)).unwrap_or((Content::Text, String::new(), String::new()));
        let service = is_service(content);
        let sender = match last {
            Some(message) if service => self.sender_first_name(&message.sender_id, names),
            Some(message) if kind == ChatKind::Group && !message.is_outgoing => self.sender_first_name(&message.sender_id, names),
            _ => String::new(),
        };
        let user = self.user_of(chat);
        let supergroup = self.supergroup_of(chat);
        let usernames = user.and_then(|user| user.usernames.as_ref()).or_else(|| supergroup.and_then(|group| group.usernames.as_ref()));
        let verification = user.and_then(|user| user.verification_status.as_ref()).or_else(|| supergroup.and_then(|group| group.verification_status.as_ref()));
        let (picture, has_picture) = self.avatar(chat.photo.as_ref(), false);
        ChatRow {
            id: chat.id.to_string().into(),
            initial: initial(&title),
            picture,
            has_picture,
            title: title.into(),
            kind,
            has_message: last.is_some(),
            time: last.map(|message| time::moment(message.date)).unwrap_or_default(),
            sender: sender.into(),
            outgoing: last.is_some_and(|message| message.is_outgoing),
            content,
            text: first_line(&text).into(),
            detail: detail.into(),
            unread: chat.unread_count,
            mention: chat.unread_mention_count > 0,
            draft: first_line(&self.draft_of(chat)).into(),
            marked: chat.unread_count == 0 && chat.is_marked_as_unread,
            muted: self.muted(chat),
            pinned: position(chat, list).is_some_and(|position| position.is_pinned),
            online: kind == ChatKind::User && user.is_some_and(|user| matches!(user.status, UserStatus::Online { .. })),
            verified: verification.is_some_and(|status| status.is_verified),
            members: self.members(chat),
            username: usernames.and_then(|names| names.active_usernames.first()).cloned().unwrap_or_default().into(),
            blocked: chat.block_list == Some(api::BlockList::Main),
            reportable: chat.can_be_reported,
            deletable: chat.can_be_deleted_only_for_self || chat.can_be_deleted_for_all_users,
        }
    }

    /// The chats pinned in any of `lists` (All chats and the folders), matching the search words:
    /// those of All chats first, in Telegram's order, then those of each folder.
    fn pinned_chats(&self, lists: &[ChatList], names: &Names) -> Vec<&api::Chat> {
        let mut seen = HashSet::new();
        lists
            .iter()
            .flat_map(|list| self.chats_in(*list, names).into_iter().filter(|chat| position(chat, *list).is_some_and(|position| position.is_pinned)))
            .filter(|chat| seen.insert(chat.id))
            .collect()
    }

    /// The chat list at `index` of the folder tabs: 0 is All chats (the main list).
    pub fn folder_list(&self, index: usize) -> ChatList {
        match index.checked_sub(1).and_then(|folder| self.folders.get(folder)) {
            Some(folder) => ChatList::Folder { chat_folder_id: folder.id },
            None => ChatList::Main,
        }
    }

    /// The account's own folders for a chat's menu: whether `chat_id` is in each.
    pub fn folder_choices(&self, chat_id: i64) -> Vec<FolderChoice> {
        let chat = self.chats.get(&chat_id);
        self.folders
            .iter()
            .enumerate()
            .map(|(index, folder)| FolderChoice {
                folder: index as i32 + 1,
                name: folder.name.text.text.clone().into(),
                inside: chat.is_some_and(|chat| position(chat, ChatList::Folder { chat_folder_id: folder.id }).is_some()),
            })
            .collect()
    }

    /// The chats of `list`, in the chosen order (`compare`: the pinned ones first, then by the
    /// newest message, by name or by unread messages). Saved Messages is left out: it is the fixed
    /// row above every list.
    pub fn chats_in(&self, list: ChatList, names: &Names) -> Vec<&api::Chat> {
        let mut chats: Vec<(&api::Chat, SortKey)> = self
            .chats
            .values()
            .filter(|chat| chat.id != self.my_id)
            .filter_map(|chat| position(chat, list).map(|position| (chat, position)))
            .map(|(chat, position)| (chat, SortKey { pinned: position.is_pinned, order: position.order, id: chat.id, title: self.title(chat, names), unread: self.unread_for_order(chat) }))
            .collect();
        chats.sort_by(|(_, a), (_, b)| compare(a, b, names.order, &names.name_order));
        chats.into_iter().map(|(chat, _)| chat).collect()
    }

    /// What a chat counts as unread for the order by unread messages: nothing when it is muted
    /// (its badge is grey), else its unread messages, or one when it was marked unread by hand.
    fn unread_for_order(&self, chat: &api::Chat) -> i32 {
        if self.muted(chat) { 0 } else { chat.unread_count.max(i32::from(chat.is_marked_as_unread)) }
    }

    fn refresh_chats(&mut self, ui: &MainWindow, names: &Names) {
        let lists: Vec<ChatList> = (0..=self.folders.len()).map(|index| self.folder_list(index)).collect();

        let folders: Vec<Folder> = lists
            .iter()
            .enumerate()
            .map(|(index, list)| Folder {
                id: if index == 0 { 0 } else { self.folders[index - 1].id },
                name: if index == 0 { names.all_chats.clone() } else { self.folders[index - 1].name.text.text.clone() }.into(),
                unread: self.chats_in(*list, names).iter().map(|chat| chat.unread_count).sum(),
            })
            .collect();

        let shown = self.shown_folder.min(lists.len() - 1);
        let list: Vec<ChatRow> = self.chats_in(lists[shown], names).iter().map(|chat| self.chat_row(chat, lists[shown], names)).collect();

        // Workbench's tree: the pinned chats on their own at the top, in no folder, then the
        // account's folders and All chats without them. Without folders, All chats is open; with
        // folders, they are open and All chats is closed, until the user says otherwise.
        let pinned = self.pinned_chats(&lists, names);
        let at_top: HashSet<i64> = pinned.iter().map(|chat| chat.id).collect();
        let mut tree: Vec<TreeRow> = pinned
            .iter()
            .map(|chat| TreeRow {
                header: false,
                folder: -1,
                expanded: true,
                count: 0,
                chat: ChatRow { pinned: true, ..self.chat_row(chat, ChatList::Main, names) },
            })
            .collect();
        let order: Vec<usize> = (1..lists.len()).chain(std::iter::once(0)).collect();
        for index in order {
            let chats: Vec<&api::Chat> = self.chats_in(lists[index], names).into_iter().filter(|chat| !at_top.contains(&chat.id)).collect();
            let id = folders[index].id;
            let expanded = self.expanded.get(&id).copied().unwrap_or(index > 0 || self.folders.is_empty());
            tree.push(TreeRow { header: true, folder: index as i32, expanded, count: chats.len() as i32, chat: self.blank_chat() });
            if expanded {
                tree.extend(chats.iter().map(|chat| TreeRow {
                    header: false,
                    folder: index as i32,
                    expanded,
                    count: 0,
                    chat: self.chat_row(chat, lists[index], names),
                }));
            }
        }

        let main = self.chats_in(ChatList::Main, names);
        let channels: Vec<ChatRow> = main
            .iter()
            .filter(|chat| self.kind(chat) == ChatKind::Channel)
            .map(|chat| self.chat_row(chat, ChatList::Main, names))
            .collect();
        let bots: Vec<ChatRow> =
            main.iter().filter(|chat| self.kind(chat) == ChatKind::Bot).map(|chat| self.chat_row(chat, ChatList::Main, names)).collect();
        let (official_bots, bots): (Vec<ChatRow>, Vec<ChatRow>) = bots.into_iter().partition(|bot| bot.verified);

        let chats = ui.global::<Chats>();
        // Saved Messages: the fixed row above every list, there before the account has the chat
        // (TDLib makes it when the row is clicked, conversation.rs).
        let saved_messages = match self.chats.get(&self.my_id) {
            Some(chat) => self.chat_row(chat, ChatList::Main, names),
            None => ChatRow {
                id: self.my_id.to_string().into(),
                title: names.saved_messages.clone().into(),
                initial: initial(&names.saved_messages),
                kind: ChatKind::Saved,
                ..self.blank_chat()
            },
        };
        chats.set_saved_messages(saved_messages);
        chats.set_saved_messages_shown(self.my_id != 0);
        // The dot on Workbench's channels button: muted channels do not count, as in Telegram's own
        // badges, or the dot never goes out.
        chats.set_unread_channels(channels.iter().filter(|channel| channel.unread > 0 && !channel.muted).count() as i32);
        chats.set_folder(shown as i32);
        sync(&self.models.folders, folders);
        sync(&self.models.list, list);
        sync(&self.models.tree, tree);
        sync(&self.models.channels, channels);
        sync(&self.models.official_bots, official_bots);
        sync(&self.models.bots, bots);

        // The tabs' names follow the chats', and their menus say whether the chat is muted.
        let tabs: Vec<Tab> = self
            .tabs
            .iter()
            .filter_map(|id| self.chats.get(id))
            .map(|chat| Tab {
                id: chat.id.to_string().into(),
                title: self.title(chat, names).into(),
                kind: self.kind(chat),
                muted: self.muted(chat),
            })
            .collect();
        sync_keyed(&self.models.tabs, tabs, |tab| tab.id.clone());
    }

    /// The open chat's header, and its message rows when `rows` (they take the longest: every row
    /// is made again and compared, so only what changed reaches the pages).
    fn refresh_conversation(&mut self, ui: &MainWindow, names: &Names, rows: bool, scroll_to_end: bool) {
        let conversation = ui.global::<Conversation>();
        let Some(chat) = self.open.and_then(|id| self.chats.get(&id)) else {
            conversation.set_chat_id(SharedString::new());
            self.models.messages.set_vec(Vec::new());
            return;
        };
        let kind = self.kind(chat);
        let title = self.title(chat, names);
        conversation.set_chat_id(chat.id.to_string().into());
        conversation.set_initial(initial(&title));
        conversation.set_title(title.into());
        conversation.set_kind(kind);
        conversation.set_can_write(self.can_write(chat));
        let rights = self.send_rights(chat);
        conversation.set_can_send_photos(rights.photos);
        conversation.set_can_send_videos(rights.videos);
        conversation.set_can_send_files(rights.documents);
        conversation.set_muted(self.muted(chat));
        conversation.set_pinned(!pinned_in(chat).is_empty());

        let (status, last_seen, members) = self.status(chat, kind);
        conversation.set_status(status);
        conversation.set_last_seen(last_seen);
        conversation.set_members(members);
        conversation.set_online_members(self.online_members.get(&chat.id).copied().unwrap_or(0));

        let typing: Vec<String> = self
            .typing
            .get(&chat.id)
            .map(|typing| {
                typing
                    .iter()
                    .filter(|(_, since)| since.elapsed() < TYPING_LASTS)
                    .map(|(sender, _)| self.sender_first_name(sender, names))
                    .collect()
            })
            .unwrap_or_default();
        conversation.set_typing(typing.join(", ").into());

        // The newest pinned message, the bot's keyboard, its commands and its menu button.
        let me = self.user_name(self.my_id, names);
        let pinned = match self.pinned.get(&chat.id) {
            Some(Some(message)) => self.quote(message, &me, names),
            _ => self.no_quote(),
        };
        if conversation.get_pinned_message() != pinned {
            conversation.set_pinned_message(pinned);
        }
        let (keyboard, placeholder) = self.keyboard(chat);
        conversation.set_keyboard(keyboard);
        conversation.set_keyboard_placeholder(placeholder.into());
        conversation.set_keyboard_hidden(self.keyboard_hidden.contains(&chat.id));
        let commands = self.bot_commands(chat);
        if *self.commands_shown.borrow() != commands {
            conversation.set_commands(ModelRc::new(VecModel::from(commands.clone())));
            *self.commands_shown.borrow_mut() = commands;
        }
        conversation.set_menu_button(self.menu_button(chat).into());

        // Topics, slow mode, how many reactions one may give, the settings panel's group part.
        let is_forum = self.is_forum(chat);
        conversation.set_is_forum(is_forum);
        conversation.set_showing_topics(is_forum && self.open_topic.is_none());
        conversation.set_topics_loading(self.topics_loading.contains(&chat.id));
        let topic = self.open_topic.and_then(|id| self.topics.get(&chat.id)?.iter().find(|topic| topic.info.forum_topic_id == id));
        conversation.set_topic_id(self.open_topic.map(|id| id.to_string()).unwrap_or_default().into());
        conversation.set_topic_title(topic.map(|topic| topic.info.name.clone()).unwrap_or_default().into());
        conversation.set_topic_icon(topic.map(|topic| self.topic_emoji(&topic.info)).unwrap_or_default().into());
        conversation.set_topic_color(topic.map(|topic| topic_colour(topic.info.icon.color)).unwrap_or_default());
        conversation.set_topic_closed(topic.is_some_and(|topic| topic.info.is_closed));
        conversation.set_topic_count(self.topics.get(&chat.id).map_or(0, |topics| topics.iter().filter(|topic| !topic.info.is_hidden).count() as i32));
        if is_forum {
            let topic_rows = self.topic_rows(chat, names);
            if *self.topics_shown.borrow() != topic_rows {
                conversation.set_topics(ModelRc::new(VecModel::from(topic_rows.clone())));
                *self.topics_shown.borrow_mut() = topic_rows;
            }
        } else if !self.topics_shown.borrow().is_empty() {
            conversation.set_topics(ModelRc::default());
            self.topics_shown.borrow_mut().clear();
        }
        let (slow_mode, limited) = self.slow_mode(chat);
        conversation.set_slow_mode(slow_mode);
        conversation.set_slow_limited(limited);
        let wait = self.slow_until.get(&chat.id).map_or(0, |until| until.saturating_duration_since(Instant::now()).as_secs_f64().ceil() as i32);
        conversation.set_slow_wait(if limited { wait } else { 0 });
        conversation.set_reaction_max(self.reaction_max(chat));
        let admin = self.admin_panel(chat);
        if conversation.get_admin() != admin {
            conversation.set_admin(admin);
        }
        if !rows {
            return;
        }

        let history = self.histories.get(&chat.id);
        conversation.set_loading(history.is_none_or(|history| history.loading && history.messages.is_empty()));
        conversation.set_has_older(history.is_some_and(|history| history.has_older || history.hides_older()));
        conversation.set_has_newer(history.is_some_and(|history| history.has_newer));
        let (rows, members, missing) = self.message_rows(chat, names);
        self.rows = members;
        self.missing_replies = missing;
        sync_keyed(&self.models.messages, rows, row_key);
        if scroll_to_end {
            conversation.set_scroll_to_end(conversation.get_scroll_to_end() + 1);
        }
    }

    /// How the header describes the chat.
    fn status(&self, chat: &api::Chat, kind: ChatKind) -> (Status, Moment, i32) {
        match kind {
            ChatKind::Saved => (Status::Saved, Moment::default(), 0),
            ChatKind::Bot => (Status::Bot, Moment::default(), 0),
            ChatKind::User => match self.user_of(chat) {
                Some(user) => {
                    let (status, last_seen) = user_status(user);
                    (status, last_seen, 0)
                }
                None => (Status::None, Moment::default(), 0),
            },
            ChatKind::Group => (Status::Members, Moment::default(), self.members(chat)),
            ChatKind::Channel => (Status::Subscribers, Moment::default(), self.members(chat)),
        }
    }

    /// How a chat is described in a list of people and chats found (global_search.rs): a person's
    /// status, a group's or channel's size.
    pub fn status_of(&self, chat: &api::Chat) -> (Status, Moment, i32) {
        self.status(chat, self.kind(chat))
    }

    /// The photo of a person, as far as it is here, and whether there is one (else their letter).
    pub fn user_avatar(&self, user: &api::User) -> (Image, bool) {
        self.avatar(user.profile_photo.as_ref(), false)
    }

    /// The photo of a chat, the same way.
    pub fn chat_avatar(&self, chat: &api::Chat) -> (Image, bool) {
        self.avatar(chat.photo.as_ref(), false)
    }

    /// No picture: the one empty image every row without a picture shares.
    pub fn blank_image(&self) -> Image {
        self.blank.clone()
    }

    /// The open chat's rows; the messages of each message row (several for an album); and the
    /// messages replies answer that are still to be fetched.
    #[allow(clippy::type_complexity)]
    fn message_rows(&self, chat: &api::Chat, names: &Names) -> (Vec<MessageRow>, Vec<(i64, Vec<i64>)>, Vec<(i64, i64)>) {
        let Some(history) = self.histories.get(&chat.id) else { return (Vec::new(), Vec::new(), Vec::new()) };
        let me = self.user_name(self.my_id, names);
        let channel = self.kind(chat) == ChatKind::Channel;
        let shown = history.messages.len().min(history.shown);
        let mut rows = Vec::with_capacity(shown + 8);
        let mut members: Vec<(i64, Vec<i64>)> = Vec::with_capacity(shown);
        let mut missing = Vec::new();
        // Each row's media, turned into its model once every row is known.
        let mut media: Vec<Vec<Media>> = Vec::with_capacity(rows.capacity());
        let mut previous_day = None;
        // The album of the row before, whose next messages join it.
        let mut album = 0;
        let mut row_files = HashSet::new();
        for message in history.shown_messages() {
            let pending = matches!(message.sending_state, Some(MessageSendingState::Pending));
            row_files.extend(message_files(&message.content));
            let day = time::day(message.date);
            if day != previous_day {
                rows.push(MessageRow { kind: RowKind::Day, day: time::moment(message.date), ..self.blank_row() });
                media.push(Vec::new());
                previous_day = day;
                album = 0;
            }
            let (content, text, detail) = self.content(message, names);
            let item = picture(&message.content).map(|picture| Media {
                id: message.id.to_string().into(),
                picture: self.picture_image(&picture),
                width: picture.width,
                height: picture.height,
                video: !matches!(message.content, M::Photo { .. }),
                duration: picture.duration,
                name: file_name(&message.content).into(),
                secret: is_secret(&message.content),
                uploading: pending && self.transfer_of(original(&message.content)).is_some(),
                progress: self.transfer_of(original(&message.content)).map_or(0.0, |(moved, total)| part(moved, total)),
                size: self.transfer_of(original(&message.content)).map_or(SharedString::new(), |(_, total)| size_text(total).into()),
                moved: self.transfer_of(original(&message.content)).map_or(SharedString::new(), |(moved, _)| size_text(moved).into()),
            });
            let files: Vec<FileCard> = match &message.content {
                M::Document { document, .. } => vec![self.file_card(message, document, pending)],
                _ => Vec::new(),
            };
            let rich_text = formatted(&message.content).and_then(rich_text::styled_text);
            let sticker = sticker_picture(&message.content)
                .map(|picture| Sticker { picture: self.picture_image(&picture), width: picture.width.max(1), height: picture.height.max(1) })
                .unwrap_or_else(|| self.no_sticker());
            // An album: its messages after the first join the first one's row, which takes a
            // caption from whichever has one.
            if message.media_album_id != 0
                && message.media_album_id == album
                && let (Some(row), Some(list)) = (rows.last_mut(), media.last_mut())
            {
                list.extend(item);
                if row.text.is_empty() {
                    row.text = text.into();
                    row.rich = rich_text.is_some();
                    row.rich_text = rich_text.unwrap_or_default();
                }
                if let Some((_, ids)) = members.last_mut() {
                    ids.push(message.id);
                }
                continue;
            }
            album = message.media_album_id;
            media.push(item.into_iter().collect());
            let sender = if message.is_outgoing && !me.is_empty() { me.clone() } else { self.sender_name(&message.sender_id, names) };
            let (sender_picture, has_sender_picture) = self.sender_avatar(&message.sender_id);
            let service = is_service(content);
            if !service {
                members.push((message.id, vec![message.id]));
            }
            rows.push(MessageRow {
                kind: if service { RowKind::Service } else { RowKind::Message },
                id: message.id.to_string().into(),
                outgoing: message.is_outgoing,
                sender_initial: initial(&sender),
                sender_picture,
                has_sender_picture,
                sender: sender.into(),
                sender_color: colour_of(&message.sender_id),
                content,
                text: text.into(),
                detail: detail.into(),
                time: time::clock(message.date).into(),
                day: Moment::default(),
                edited: message.edit_date > 0,
                sending: matches!(message.sending_state, Some(MessageSendingState::Pending)),
                failed: matches!(message.sending_state, Some(MessageSendingState::Failed { .. })),
                seen: message.is_outgoing && message.id <= chat.last_read_outbox_message_id,
                button: SharedString::new(),
                media: ModelRc::default(),
                files: if files.is_empty() { ModelRc::default() } else { ModelRc::new(VecModel::from(files)) },
                buttons: self.button_rows(message),
                reactions: self.reaction_rows(message),
                round: self.round_note(message),
                contact: self.contact_card(message),
                can_share: channel && !service && !pending,
                sticker,
                rich: rich_text.is_some(),
                rich_text: rich_text.unwrap_or_default(),
                preview: link_preview(&message.content),
                forwarded_from: message.forward_info.as_ref().map(|info| self.origin_name(&info.origin, names)).unwrap_or_default().into(),
                reply: self.reply_quote(message, &me, names, &mut missing),
                selected: self.selected.contains(&message.id),
            });
        }
        self.attach_media(&mut rows, media);
        *self.row_files.borrow_mut() = row_files;
        // Telegram's sponsored message, after the newest post of a channel (not among older ones).
        if let Some(sponsored) = self.sponsored.get(&chat.id).and_then(|sponsored| sponsored.first()).filter(|_| !history.has_newer) {
            let text = match &sponsored.content {
                M::Text { text, .. } => text.text.clone(),
                other => self.content_of(other, &MessageSender::Chat { chat_id: chat.id }, names).1,
            };
            rows.push(MessageRow {
                kind: RowKind::Sponsored,
                id: sponsored.message_id.to_string().into(),
                sender: sponsored.title.clone().into(),
                text: text.into(),
                button: sponsored.button_text.clone().into(),
                ..self.blank_row()
            });
        }
        (rows, members, missing)
    }

    /// Who first wrote a forwarded message, or one a reply from another chat answers.
    pub fn origin_name(&self, origin: &MessageOrigin, names: &Names) -> String {
        match origin {
            MessageOrigin::User { sender_user_id } => self.user_name(*sender_user_id, names),
            MessageOrigin::HiddenUser { sender_name } => sender_name.clone(),
            MessageOrigin::Chat { sender_chat_id: chat_id } | MessageOrigin::Channel { chat_id } => {
                self.chats.get(chat_id).map(|chat| self.title(chat, names)).unwrap_or_default()
            }
        }
    }

    /// What a reply shows of the message it answers. One in this chat that is not in the history
    /// is added to `missing`, to fetch; until then the reply shows none.
    fn reply_quote(&self, message: &api::Message, me: &str, names: &Names, missing: &mut Vec<(i64, i64)>) -> ReplyQuote {
        let Some(MessageReplyTo::Message { chat_id, message_id, quote, origin, content }) = &message.reply_to else {
            return self.no_quote();
        };
        let quoted = quote.as_ref().map(|quote| first_line(&quote.text.text).to_string()).filter(|text| !text.is_empty());
        // From another chat: the reply itself says who wrote it and what it holds.
        if let Some(origin) = origin {
            let (kind, text, detail) = content
                .as_deref()
                .map(|content| self.content_of(content, &MessageSender::Chat { chat_id: *chat_id }, names))
                .unwrap_or((Content::Text, String::new(), String::new()));
            let picture = content.as_deref().and_then(picture).map(|picture| self.picture_image(&picture));
            return ReplyQuote {
                shown: true,
                id: SharedString::new(),
                sender: self.origin_name(origin, names).into(),
                content: kind,
                text: quoted.unwrap_or_else(|| first_line(&text).to_string()).into(),
                detail: detail.into(),
                gone: false,
                has_picture: picture.is_some(),
                picture: picture.unwrap_or_else(|| self.blank.clone()),
            };
        }
        if *message_id == 0 {
            return self.no_quote();
        }
        let chat_id = if *chat_id == 0 { message.chat_id } else { *chat_id };
        let original = match self.histories.get(&chat_id).and_then(|history| history.messages.get(message_id)) {
            Some(original) => original,
            None => match self.replied.get(&(chat_id, *message_id)) {
                Some(Some(original)) => original,
                Some(None) => return ReplyQuote { shown: true, gone: true, ..self.no_quote() },
                None => {
                    if !self.asking_replies.contains(&(chat_id, *message_id)) {
                        missing.push((chat_id, *message_id));
                    }
                    return self.no_quote();
                }
            },
        };
        let shown = self.quote(original, me, names);
        ReplyQuote {
            id: if chat_id == message.chat_id { shown.id } else { SharedString::new() },
            text: quoted.map(SharedString::from).unwrap_or(shown.text),
            ..shown
        }
    }

    /// A message as a reply shows it, and the strip above the composer: who wrote it (`me` for
    /// ours), its first line, its picture.
    pub fn quote(&self, message: &api::Message, me: &str, names: &Names) -> ReplyQuote {
        let (kind, text, detail) = self.content(message, names);
        let sender = if message.is_outgoing && !me.is_empty() { me.to_string() } else { self.sender_name(&message.sender_id, names) };
        let picture = picture(&message.content).map(|picture| self.picture_image(&picture));
        ReplyQuote {
            shown: true,
            id: message.id.to_string().into(),
            sender: sender.into(),
            content: kind,
            text: first_line(&text).into(),
            detail: detail.into(),
            gone: false,
            has_picture: picture.is_some(),
            picture: picture.unwrap_or_else(|| self.blank.clone()),
        }
    }

    /// The open chat's photos, videos and GIFs for the media viewer, oldest first.
    pub fn viewer_items(&self, names: &Names) -> Vec<ViewerItem> {
        let Some(history) = self.open.and_then(|chat_id| self.histories.get(&chat_id)) else { return Vec::new() };
        // A self-destructing photo or video is not among them: the viewer shows it on its own, once
        // the user opens it from the chat (secret_viewer_item).
        history.messages.values().filter(|message| !is_secret(&message.content)).filter_map(|message| self.viewer_item(message, names)).collect()
    }

    /// The viewer's item for a self-destructing photo or video of the open chat (viewer.rs): its
    /// blurred preview, until the viewer has its file.
    pub fn secret_viewer_item(&self, message_id: i64, names: &Names) -> Option<ViewerItem> {
        let message = self.histories.get(&self.open?)?.messages.get(&message_id)?;
        if !is_secret(&message.content) {
            return None;
        }
        self.viewer_item(message, names)
    }

    fn viewer_item(&self, message: &api::Message, names: &Names) -> Option<ViewerItem> {
        let picture = picture(&message.content)?;
        let me = self.user_name(self.my_id, names);
        let sender = if message.is_outgoing && !me.is_empty() { me } else { self.sender_name(&message.sender_id, names) };
        let image = original(&message.content).and_then(|file| self.pictures.get(&file.id)).unwrap_or_else(|| self.picture_image(&picture));
        let (sender_picture, has_sender_picture) = self.sender_avatar(&message.sender_id);
        Some(ViewerItem {
            id: message.id.to_string().into(),
            picture: image,
            width: picture.width,
            height: picture.height,
            video: !matches!(message.content, M::Photo { .. }),
            duration: picture.duration,
            name: file_name(&message.content).into(),
            sender_initial: initial(&sender),
            sender_picture,
            has_sender_picture,
            sender: sender.into(),
            time: time::moment(message.date),
            caption: self.content(message, names).1.into(),
            secret: is_secret(&message.content),
        })
    }

    /// The self-destructing message the viewer shows has run out its time: its content expired
    /// after it was opened. One to be seen once expires the moment it is opened, and stays until
    /// the viewer is closed.
    fn secret_timed_out(&self, message_id: &str) -> bool {
        let Some(message) = message_id.parse().ok().and_then(|id| self.histories.get(&self.open?)?.messages.get(&id)) else { return false };
        is_expired(&message.content) && matches!(message.self_destruct_type, Some(api::MessageSelfDestructType::Timer { .. }))
    }

    /// A bot's buttons under a message, row by row; none for most messages. The model is kept
    /// from one refresh to the next while the buttons stay the same, so that the row does not
    /// count as changed (a model compares by identity).
    fn button_rows(&self, message: &api::Message) -> ModelRc<ButtonRow> {
        let Some(api::ReplyMarkup::InlineKeyboard { rows }) = &message.reply_markup else { return ModelRc::default() };
        if rows.is_empty() {
            return ModelRc::default();
        }
        let id: SharedString = message.id.to_string().into();
        let mut hasher = DefaultHasher::new();
        format!("{rows:?}").hash(&mut hasher);
        let hash = hasher.finish();
        let mut models = self.button_models.borrow_mut();
        if let Some((kept, model)) = models.get(&id)
            && *kept == hash
        {
            return model.clone();
        }
        let built: Vec<ButtonRow> = rows
            .iter()
            .enumerate()
            .map(|(row, buttons)| ButtonRow {
                buttons: ModelRc::new(VecModel::from(
                    buttons
                        .iter()
                        .enumerate()
                        .map(|(column, button)| ChatButton {
                            text: button.text.clone().into(),
                            kind: inline_kind(&button.kind),
                            row: row as i32,
                            column: column as i32,
                        })
                        .collect::<Vec<_>>(),
                )),
            })
            .collect();
        let model = ModelRc::new(VecModel::from(built));
        models.insert(id, (hash, model.clone()));
        model
    }

    /// The keyboard a bot asked the chat to show by the composer, row by row, and the words it
    /// puts in the empty composer; nothing when the chat has none, or it was taken away.
    fn keyboard(&self, chat: &api::Chat) -> (ModelRc<ButtonRow>, String) {
        let Some(Some(message)) = self.keyboards.get(&chat.id) else { return (ModelRc::default(), String::new()) };
        let (rows, placeholder) = match &message.reply_markup {
            Some(api::ReplyMarkup::ShowKeyboard { rows, input_field_placeholder, .. }) => (rows, input_field_placeholder.clone()),
            Some(api::ReplyMarkup::ForceReply { input_field_placeholder, .. }) => return (ModelRc::default(), input_field_placeholder.clone()),
            _ => return (ModelRc::default(), String::new()),
        };
        if let Some((chat_id, message_id, model)) = &*self.keyboard_model.borrow()
            && *chat_id == chat.id
            && *message_id == message.id
        {
            return (model.clone(), placeholder);
        }
        let built: Vec<ButtonRow> = rows
            .iter()
            .enumerate()
            .map(|(row, buttons)| ButtonRow {
                buttons: ModelRc::new(VecModel::from(
                    buttons
                        .iter()
                        .enumerate()
                        .map(|(column, button)| ChatButton {
                            text: button.text.clone().into(),
                            kind: key_kind(&button.kind),
                            row: row as i32,
                            column: column as i32,
                        })
                        .collect::<Vec<_>>(),
                )),
            })
            .collect();
        let model = ModelRc::new(VecModel::from(built));
        *self.keyboard_model.borrow_mut() = Some((chat.id, message.id, model.clone()));
        (model, placeholder)
    }

    /// The commands of the chat's bot (or bots, in a group) that start with what is typed after
    /// "/"; none while the draft is no command.
    fn bot_commands(&self, chat: &api::Chat) -> Vec<BotCommand> {
        let Some(filter) = &self.command_filter else { return Vec::new() };
        let commands: Vec<&api::BotCommand> = match chat.kind {
            ChatType::Private { user_id } | ChatType::Secret { user_id } => {
                self.bot_infos.get(&user_id).map(|info| info.commands.iter().collect()).unwrap_or_default()
            }
            _ => self.group_commands.get(&chat.id).map(|bots| bots.iter().flat_map(|bot| bot.commands.iter()).collect()).unwrap_or_default(),
        };
        commands
            .into_iter()
            .filter(|command| command.command.to_lowercase().starts_with(filter.as_str()))
            .take(COMMANDS_SHOWN)
            .map(|command| BotCommand { command: format!("/{}", command.command).into(), description: command.description.clone().into() })
            .collect()
    }

    /// The bot's menu button at the composer's left: a web app's name, "/" for the commands
    /// menu (the pages word it), "" for none (not a bot, or a bot with neither).
    fn menu_button(&self, chat: &api::Chat) -> String {
        let (ChatType::Private { user_id } | ChatType::Secret { user_id }) = chat.kind else { return String::new() };
        let Some(info) = self.bot_infos.get(&user_id) else { return String::new() };
        match &info.menu_button {
            Some(button) if !button.url.is_empty() => if button.text.is_empty() { "/".to_string() } else { button.text.clone() },
            _ if !info.commands.is_empty() => "/".to_string(),
            _ => String::new(),
        }
    }

    /// Give each row the model of its media, the same one as last time, brought up to date.
    fn attach_media(&self, rows: &mut [MessageRow], media: Vec<Vec<Media>>) {
        let mut models = self.media_models.borrow_mut();
        let mut kept = HashMap::new();
        for (row, list) in rows.iter_mut().zip(media) {
            if list.is_empty() {
                continue;
            }
            let model = models.remove(&row.id).unwrap_or_else(|| Rc::new(VecModel::default()));
            sync(&model, list);
            row.media = ModelRc::from(model.clone());
            kept.insert(row.id.clone(), model);
        }
        *models = kept;
    }

    /// How much of a file has moved, and how much there is, while it is on its way (updateFile).
    fn transfer_of(&self, file: Option<&api::File>) -> Option<(i64, i64)> {
        self.transfers.get(&file?.id).copied()
    }

    /// A message's file as the design's card: where it is, and how far it has got.
    fn file_card(&self, message: &api::Message, document: &api::Document, pending: bool) -> FileCard {
        let file = &document.document;
        let transfer = self.transfer_of(Some(file));
        let state = if pending && !file.remote.is_uploading_completed {
            FileState::Uploading
        } else if super::files::completed(file).is_some() {
            FileState::Done
        } else if transfer.is_some() || file.local.is_downloading_active {
            FileState::Downloading
        } else {
            FileState::Remote
        };
        let (moved, total) = transfer.unwrap_or((0, file.size_or_expected()));
        FileCard {
            id: message.id.to_string().into(),
            name: document.file_name.clone().into(),
            size: size_text(file.size_or_expected()).into(),
            file_type: file_type_of(&document.file_name),
            state,
            progress: if matches!(state, FileState::Uploading | FileState::Downloading) { part(moved, total) } else { 0.0 },
            moved: size_text(moved).into(),
        }
    }

    /// A row with no pictures anywhere, to build the others from.
    fn blank_row(&self) -> MessageRow {
        MessageRow {
            sender_picture: self.blank.clone(),
            sticker: self.no_sticker(),
            reply: self.no_quote(),
            round: self.no_round(),
            ..MessageRow::default()
        }
    }

    fn no_round(&self) -> RoundNote {
        RoundNote { picture: self.blank.clone(), ..RoundNote::default() }
    }

    // ---- the fifth round: reactions, round video messages, contact cards, topics, admin ------

    /// The reactions to a message, most given first, as the row shows them; the model is kept
    /// while they stay the same (a model compares by identity).
    fn reaction_rows(&self, message: &api::Message) -> ModelRc<Reaction> {
        let reactions = message.reactions();
        if reactions.is_empty() {
            return ModelRc::default();
        }
        let built: Vec<Reaction> = reactions
            .iter()
            .filter(|reaction| reaction.total_count > 0)
            .map(|reaction| Reaction {
                emoji: self.reaction_emoji(&reaction.kind).into(),
                count: count_text(reaction.total_count).into(),
                chosen: reaction.is_chosen,
            })
            .collect();
        if built.is_empty() {
            return ModelRc::default();
        }
        let id: SharedString = message.id.to_string().into();
        let mut hasher = DefaultHasher::new();
        for reaction in &built {
            reaction.emoji.hash(&mut hasher);
            reaction.count.hash(&mut hasher);
            reaction.chosen.hash(&mut hasher);
        }
        let hash = hasher.finish();
        let mut models = self.reaction_models.borrow_mut();
        if let Some((kept, model)) = models.get(&id)
            && *kept == hash
        {
            return model.clone();
        }
        let model = ModelRc::new(VecModel::from(built));
        models.insert(id, (hash, model.clone()));
        model
    }

    /// What a reaction looks like: its emoji; a custom emoji's, once TDLib has told us which
    /// (asked for after the refresh), "✨" until then; "⭐" for a paid one.
    pub fn reaction_emoji(&self, kind: &api::ReactionType) -> String {
        match kind {
            api::ReactionType::Emoji { emoji } => emoji.clone(),
            api::ReactionType::CustomEmoji { custom_emoji_id } => self.custom_emoji_text(*custom_emoji_id).unwrap_or_else(|| "✨".into()),
            api::ReactionType::Paid => "⭐".into(),
        }
    }

    /// The emoji a custom emoji stands for, when known; else it is noted to ask for.
    fn custom_emoji_text(&self, id: i64) -> Option<String> {
        if id == 0 {
            return None;
        }
        match self.custom_emoji.get(&id) {
            Some(emoji) if !emoji.is_empty() => Some(emoji.clone()),
            Some(_) => None,
            None => {
                self.custom_emoji_wanted.borrow_mut().push(id);
                None
            }
        }
    }

    /// A round video message as the row shows it: its still, its length, whether it plays.
    fn round_note(&self, message: &api::Message) -> RoundNote {
        let Some(still) = round_picture(&message.content) else { return self.no_round() };
        let picture = self.picture_image(&still);
        RoundNote {
            shown: true,
            has_picture: still.file.is_some_and(|file| self.pictures.get(&file.id).is_some()) || still.preview.is_some(),
            picture,
            duration: duration_text(still.duration).into(),
            playing: self.round_playing == Some(message.id),
            loading: self.round_loading == Some(message.id),
        }
    }

    /// A contact card as the row shows it.
    fn contact_card(&self, message: &api::Message) -> ContactCard {
        let M::Contact { contact } = &message.content else { return ContactCard::default() };
        let name = contact.name();
        let registered = contact.user_id != 0;
        let added = self.added_contacts.contains(&contact.phone_number)
            || (registered && self.users.get(&contact.user_id).is_some_and(|user| user.is_contact));
        ContactCard {
            shown: true,
            initial: initial(&name),
            name: name.into(),
            phone: phone_text(&contact.phone_number).into(),
            registered,
            added,
        }
    }

    /// Words left in a chat's composer: those kept here until Telegram has them, else Telegram's.
    pub fn draft_of(&self, chat: &api::Chat) -> String {
        if let Some(words) = self.drafts.get(&chat.id) {
            return words.clone();
        }
        chat.draft_message.as_ref().map(|draft| draft.words().to_string()).unwrap_or_default()
    }

    /// A group with topics (topics.rs).
    pub fn is_forum(&self, chat: &api::Chat) -> bool {
        self.supergroup_of(chat).is_some_and(|group| group.is_forum)
    }

    /// We own or administer the group: slow mode does not hold us, and the settings panel lets
    /// us change the group's rules.
    pub fn is_admin(&self, chat: &api::Chat) -> bool {
        let status = match chat.kind {
            ChatType::BasicGroup { basic_group_id } => self.basic_groups.get(&basic_group_id).map(|group| &group.status),
            ChatType::Supergroup { supergroup_id, .. } => self.supergroups.get(&supergroup_id).map(|group| &group.status),
            _ => None,
        };
        matches!(status, Some(ChatMemberStatus::Creator | ChatMemberStatus::Administrator { .. }))
    }

    /// Slow mode: the seconds a member waits between messages (0: off), and whether that holds us.
    pub fn slow_mode(&self, chat: &api::Chat) -> (i32, bool) {
        let ChatType::Supergroup { supergroup_id, .. } = chat.kind else { return (0, false) };
        let delay = self.supergroup_full.get(&supergroup_id).map_or(0, |info| info.slow_mode_delay);
        (delay, delay > 0 && !self.is_admin(chat))
    }

    /// How many reactions one person may give a message in the chat.
    pub fn reaction_max(&self, chat: &api::Chat) -> i32 {
        match &chat.available_reactions {
            Some(api::ChatAvailableReactions::All { max_reaction_count } | api::ChatAvailableReactions::Some { max_reaction_count, .. }) => {
                (*max_reaction_count).max(1)
            }
            None => 1,
        }
    }

    /// The topics of a group with topics, as rows, in TDLib's order.
    fn topic_rows(&self, chat: &api::Chat, names: &Names) -> Vec<TopicRow> {
        let Some(topics) = self.topics.get(&chat.id) else { return Vec::new() };
        topics
            .iter()
            .filter(|topic| !topic.info.is_hidden)
            .map(|topic| {
                let last = topic.last_message.as_deref();
                let last_line = last
                    .map(|message| {
                        let (content, text, detail) = self.content(message, names);
                        let words = if text.is_empty() { detail } else { text };
                        let sender = self.sender_first_name(&message.sender_id, names);
                        let line = if words.is_empty() { format!("{content:?}") } else { first_line(&words).to_string() };
                        if sender.is_empty() || is_service(content) { line } else { format!("{sender}: {line}") }
                    })
                    .unwrap_or_default();
                TopicRow {
                    id: topic.info.forum_topic_id.to_string().into(),
                    icon: self.topic_emoji(&topic.info).into(),
                    color: topic_colour(topic.info.icon.color),
                    title: topic.info.name.clone().into(),
                    last: last_line.into(),
                    time: last.map(|message| time::moment(message.date)).unwrap_or_default(),
                    unread: topic.unread_count,
                    pinned: topic.is_pinned,
                    closed: topic.info.is_closed,
                }
            })
            .collect()
    }

    /// A topic's icon as an emoji: its custom emoji's, once known; "" for the general topic and
    /// for one with a colour alone (the pages draw "#").
    pub fn topic_emoji(&self, info: &api::ForumTopicInfo) -> String {
        if info.is_general {
            return String::new();
        }
        self.custom_emoji_text(info.icon.custom_emoji_id).unwrap_or_default()
    }

    /// The settings panel's group part: the rules a member reads, or what an administrator may
    /// change (admin.rs).
    fn admin_panel(&self, chat: &api::Chat) -> AdminPanel {
        let supergroup = match chat.kind {
            ChatType::Supergroup { supergroup_id, is_channel: false } => Some(supergroup_id),
            ChatType::BasicGroup { .. } => None,
            _ => return AdminPanel::default(),
        };
        let full = supergroup.and_then(|id| self.supergroup_full.get(&id));
        let group = supergroup.and_then(|id| self.supergroups.get(&id));
        let permissions = &chat.permissions;
        AdminPanel {
            shown: true,
            admin: self.is_admin(chat),
            supergroup: supergroup.is_some(),
            members: self.members(chat),
            slow_mode: full.map_or(0, |info| info.slow_mode_delay),
            can_send: permissions.can_send_basic_messages,
            can_media: permissions.can_send_photos || permissions.can_send_videos,
            can_stickers: permissions.can_send_other_messages,
            can_links: permissions.can_add_link_previews,
            can_polls: permissions.can_send_polls,
            can_invite: permissions.can_invite_users,
            can_pin: permissions.can_pin_messages,
            can_info: permissions.can_change_info,
            join_request: group.is_some_and(|group| group.join_by_request),
            protected: chat.has_protected_content,
            history: full.is_none_or(|info| info.is_all_history_available),
            hidden_members: full.is_some_and(|info| info.has_hidden_members),
            hidden_members_allowed: full.is_some_and(|info| info.can_hide_members || info.has_hidden_members),
            anti_spam: full.is_some_and(|info| info.has_aggressive_anti_spam_enabled),
            anti_spam_allowed: full.is_some_and(|info| info.can_toggle_aggressive_anti_spam || info.has_aggressive_anti_spam_enabled),
            busy: self.admin_busy.clone().into(),
            error: self.admin_error.clone().into(),
        }
    }

    /// A chat row with no picture (a folder's header).
    fn blank_chat(&self) -> ChatRow {
        ChatRow { picture: self.blank.clone(), ..ChatRow::default() }
    }

    /// The photo of a chat or a person as far as it is here, and whether there is one to show
    /// (else its letter): the photo once downloaded, until then its tiny preview. `large`: in its
    /// big size, for the profile page, the small one until that is here. A photo not here yet is
    /// noted, to download after the refresh (avatars.rs).
    fn avatar(&self, photo: Option<&api::ChatPhoto>, large: bool) -> (Image, bool) {
        let Some(photo) = photo else { return (self.blank.clone(), false) };
        let file = if large { &photo.big } else { &photo.small };
        if let Some(image) = self.avatars.get(&file.id) {
            return (image, true);
        }
        if !self.avatars_asked.contains(&file.id) {
            self.wanted_photos.borrow_mut().push(file.clone());
        }
        if let Some(image) = large.then(|| self.avatars.get(&photo.small.id)).flatten() {
            return (image, true);
        }
        match &photo.minithumbnail {
            Some(preview) => (self.preview_image(preview), true),
            None => (self.blank.clone(), false),
        }
    }

    /// The photo of a message's sender: the person's, or the chat's for a message sent in its name.
    fn sender_avatar(&self, sender: &MessageSender) -> (Image, bool) {
        let photo = match sender {
            MessageSender::User { user_id } => self.users.get(user_id).and_then(|user| user.profile_photo.as_ref()),
            MessageSender::Chat { chat_id } => self.chats.get(chat_id).and_then(|chat| chat.photo.as_ref()),
        };
        self.avatar(photo, false)
    }

    /// The photos the rows built since the last time wanted and did not have, each once, to
    /// download.
    pub fn take_wanted_photos(&mut self) -> Vec<api::File> {
        let wanted = self.wanted_photos.take();
        wanted.into_iter().filter(|file| self.avatars_asked.insert(file.id)).collect()
    }

    /// Photos asked for and not here will not come (finchgram-tdlib ended): the rows ask again.
    pub fn ask_photos_again(&mut self) {
        self.avatars_asked.clear();
    }

    fn no_sticker(&self) -> Sticker {
        Sticker { picture: self.blank.clone(), ..Sticker::default() }
    }

    fn no_quote(&self) -> ReplyQuote {
        ReplyQuote { picture: self.blank.clone(), ..ReplyQuote::default() }
    }

    /// The best picture there is so far: the downloaded one, else the message's tiny preview.
    fn picture_image(&self, picture: &Picture) -> Image {
        if let Some(image) = picture.file.and_then(|file| self.pictures.get(&file.id)) {
            return image;
        }
        match picture.preview {
            Some(preview) => self.preview_image(preview),
            None => self.blank.clone(),
        }
    }

    /// A tiny preview as a picture, decoded the first time it is seen.
    fn preview_image(&self, preview: &api::Minithumbnail) -> Image {
        let mut hasher = DefaultHasher::new();
        preview.data.hash(&mut hasher);
        let key = hasher.finish();
        let mut previews = self.previews.borrow_mut();
        if let Some(image) = previews.get(&key) {
            return image;
        }
        let image = images::preview(&preview.data).unwrap_or_else(|| self.blank.clone());
        previews.insert(key, image.clone());
        image
    }

    /// Settings → Notifications & sounds → By chat type, once TDLib has said all three.
    fn refresh_notification_scopes(&self, ui: &MainWindow) {
        let scopes = ui.global::<NotificationScopes>();
        let known = |scope| self.scope_settings.contains_key(&scope);
        scopes.set_loaded(
            known(NotificationSettingsScope::Private) && known(NotificationSettingsScope::Group) && known(NotificationSettingsScope::Channel),
        );
        scopes.set_private_chats(!self.scope_muted(NotificationSettingsScope::Private));
        scopes.set_groups(!self.scope_muted(NotificationSettingsScope::Group));
        scopes.set_channels(!self.scope_muted(NotificationSettingsScope::Channel));
    }

    fn refresh_account(&mut self, ui: &MainWindow) {
        let account = ui.global::<Account>();
        let Some(me) = self.users.get(&self.my_id) else {
            account.set_name(SharedString::new());
            account.set_id(SharedString::new());
            account.set_initial(SharedString::new());
            account.set_picture(self.blank.clone());
            account.set_large_picture(self.blank.clone());
            account.set_has_picture(false);
            return;
        };
        let name = format!("{} {}", me.first_name, me.last_name).trim().to_string();
        account.set_initial(initial(&name));
        let (picture, has_picture) = self.avatar(me.profile_photo.as_ref(), false);
        let (large_picture, _) = self.avatar(me.profile_photo.as_ref(), true);
        account.set_picture(picture);
        account.set_large_picture(large_picture);
        account.set_has_picture(has_picture);
        account.set_name(name.into());
        account.set_id(me.id.to_string().into());
        account.set_first_name(me.first_name.clone().into());
        account.set_last_name(me.last_name.clone().into());
        account.set_username(me.usernames.as_ref().map(|names| names.editable_username.clone()).unwrap_or_default().into());
        account.set_bio(self.my_bio.clone().into());
        if account.get_phone().is_empty() {
            account.set_phone(format!("+{}", me.phone_number).into());
        }
    }
}

/// A person's status, as the pages say it: online, when they were last seen, or a bot.
pub fn user_status(user: &api::User) -> (Status, Moment) {
    if user.kind == UserType::Bot {
        return (Status::Bot, Moment::default());
    }
    match user.status {
        UserStatus::Online { .. } => (Status::Online, Moment::default()),
        UserStatus::Offline { was_online } => (Status::LastSeen, time::moment(was_online)),
        UserStatus::Recently => (Status::Recently, Moment::default()),
        UserStatus::LastWeek => (Status::LastWeek, Moment::default()),
        UserStatus::LastMonth => (Status::LastMonth, Moment::default()),
        UserStatus::Empty => (Status::LongAgo, Moment::default()),
    }
}

/// What a button under a message does, for the pages.
fn inline_kind(kind: &api::InlineKeyboardButtonType) -> ButtonKind {
    use api::InlineKeyboardButtonType as T;
    match kind {
        T::Url { .. } => ButtonKind::Url,
        T::LoginUrl { .. } => ButtonKind::Login,
        T::WebApp { .. } => ButtonKind::WebApp,
        T::Callback { .. } | T::CallbackWithPassword { .. } => ButtonKind::Callback,
        T::CallbackGame => ButtonKind::Game,
        T::SwitchInline { .. } => ButtonKind::SwitchInline,
        T::Buy => ButtonKind::Buy,
        T::User { .. } => ButtonKind::User,
        T::CopyText { .. } => ButtonKind::Copy,
        T::Disabled => ButtonKind::Disabled,
    }
}

/// What a key of a bot's keyboard does, for the pages.
fn key_kind(kind: &api::KeyboardButtonType) -> ButtonKind {
    use api::KeyboardButtonType as T;
    match kind {
        T::Text | T::Other => ButtonKind::Text,
        T::RequestPhoneNumber => ButtonKind::RequestPhone,
        T::RequestLocation => ButtonKind::RequestLocation,
        T::RequestPoll {} => ButtonKind::RequestPoll,
        T::RequestUsers {} => ButtonKind::RequestUsers,
        T::RequestChat {} => ButtonKind::RequestChat,
        T::WebApp { .. } => ButtonKind::WebApp,
    }
}

/// What an update asks for beyond the store; telegram/mod.rs sends it.
pub enum Followup {
    None,
    /// The folders changed: load their chats.
    LoadFolders,
    /// New messages in the open chat: they are seen.
    View { chat_id: i64, message_ids: Vec<i64> },
    /// Someone is typing: look again once it may have stopped.
    TypingExpires,
    /// Messages were deleted: what was being done with them stops (actions.rs).
    Deleted { chat_id: i64, message_ids: Vec<i64> },
    /// A message of the open chat was pinned or unpinned: the bar asks for the newest (bots.rs).
    PinnedChanged { chat_id: i64 },
    /// The open chat's keyboard is on another message, or gone: fetched anew (bots.rs).
    KeyboardChanged { chat_id: i64 },
    /// The open chat's content became protected, or stopped being: the window follows
    /// (conversation.rs).
    ProtectionChanged,
    /// A topic of a group whose topics are here changed, or has a new message: its row is
    /// fetched anew (topics.rs).
    TopicChanged { chat_id: i64, topic_id: i32 },
    /// The open chat's draft came from another device: the composer takes it (drafts.rs).
    DraftChanged { chat_id: i64 },
    /// The open supergroup's slow mode may have changed: the wait is counted anew (admin.rs).
    SlowModeChanged,
}

/// What a message with a photo, a video or a GIF shows before it is played: the file with its
/// picture (a photo in a suitable size, a video's still frame), its own size and length, and the
/// tiny preview the message carries.
pub struct Picture<'a> {
    pub file: Option<&'a api::File>,
    pub width: i32,
    pub height: i32,
    pub duration: i32,
    pub preview: Option<&'a api::Minithumbnail>,
}

pub fn picture(content: &M) -> Option<Picture<'_>> {
    match content {
        M::Photo { photo, .. } => {
            let largest = photo.sizes.last()?;
            let size = photo.sizes.iter().find(|size| size.width.max(size.height) >= PICTURE_SIDE).unwrap_or(largest);
            Some(Picture {
                file: Some(&size.photo),
                width: largest.width,
                height: largest.height,
                duration: 0,
                preview: photo.minithumbnail.as_ref(),
            })
        }
        M::Video { video, .. } => Some(still(video.thumbnail.as_ref(), (video.width, video.height), video.duration, video.minithumbnail.as_ref())),
        M::Animation { animation, .. } => Some(still(
            animation.thumbnail.as_ref(),
            (animation.width, animation.height),
            animation.duration,
            animation.minithumbnail.as_ref(),
        )),
        _ => None,
    }
}

/// A round video message's still: its thumbnail, square, with its length.
pub fn round_picture(content: &M) -> Option<Picture<'_>> {
    match content {
        M::VideoNote { video_note, .. } => Some(still(
            video_note.thumbnail.as_ref(),
            (video_note.length, video_note.length),
            video_note.duration,
            video_note.minithumbnail.as_ref(),
        )),
        _ => None,
    }
}

/// A count as a reaction's chip writes it: 999, 1.2K, 12K, 1.2M.
pub fn count_text(count: i32) -> String {
    let short = |value: f64, unit: &str| {
        let rounded = (value * 10.0).round() / 10.0;
        if rounded >= 10.0 || rounded.fract() == 0.0 { format!("{}{unit}", rounded.round() as i64) } else { format!("{rounded:.1}{unit}") }
    };
    match count {
        ..=999 => count.to_string(),
        1000..=999_999 => short(f64::from(count) / 1000.0, "K"),
        _ => short(f64::from(count) / 1_000_000.0, "M"),
    }
}

/// Seconds as "0:14" or "1:02:03".
pub fn duration_text(seconds: i32) -> String {
    let seconds = seconds.max(0);
    if seconds >= 3600 {
        format!("{}:{:02}:{:02}", seconds / 3600, seconds % 3600 / 60, seconds % 60)
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

/// A topic's colour, which TDLib gives as 0xRRGGBB.
pub fn topic_colour(color: i32) -> slint::Color {
    let value = u32::try_from(color).unwrap_or(0x6FB9F0);
    slint::Color::from_rgb_u8((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// A phone number with spaces: "+86 139 0571 2233" for 8613905712233 (country code, then groups
/// of three, four, four as Telegram's apps print them), "+" and the digits as they are when
/// the number is shorter.
pub fn phone_text(number: &str) -> String {
    let digits: String = number.chars().filter(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return number.to_string();
    }
    let country = match digits.len() {
        12.. => digits.len() - 10,
        _ => return format!("+{digits}"),
    };
    let (code, rest) = digits.split_at(country.min(3));
    let (a, rest) = rest.split_at(rest.len().saturating_sub(8));
    let (b, c) = rest.split_at(rest.len().saturating_sub(4));
    format!("+{code} {a} {b} {c}").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A message's own words with their formatting: a text, or the caption of a photo, video, GIF,
/// audio file, file or voice message.
pub fn formatted(content: &M) -> Option<&api::FormattedText> {
    match content {
        M::Text { text, .. } => Some(text),
        M::Photo { caption, .. }
        | M::Video { caption, .. }
        | M::Animation { caption, .. }
        | M::Audio { caption, .. }
        | M::Document { caption, .. }
        | M::VoiceNote { caption } => Some(caption),
        _ => None,
    }
}

/// A text's link preview for the card under it: the site (else the address's host), the page's
/// title (else the address), and a line about it (its author, else its description's first line).
fn link_preview(content: &M) -> LinkPreview {
    let M::Text { link_preview: Some(preview), .. } = content else { return LinkPreview::default() };
    let address = if preview.display_url.is_empty() { preview.url.as_str() } else { preview.display_url.as_str() };
    let host = address.trim_start_matches("https://").trim_start_matches("http://").split('/').next().unwrap_or_default();
    LinkPreview {
        url: preview.url.as_str().into(),
        site: if preview.site_name.is_empty() { host } else { &preview.site_name }.into(),
        title: if preview.title.is_empty() { address } else { &preview.title }.into(),
        about: if preview.author.is_empty() { first_line(&preview.description.text) } else { &preview.author }.into(),
        instant_view: preview.instant_view_version > 0,
    }
}

/// What a sticker shows: a still sticker itself, an animated one its still thumbnail (moving
/// stickers need a player of their own). None: there is only its emoji to show.
pub fn sticker_picture(content: &M) -> Option<Picture<'_>> {
    let M::Sticker { sticker } = content else { return None };
    let file = match sticker.format {
        api::StickerFormat::Webp => &sticker.sticker,
        api::StickerFormat::Tgs | api::StickerFormat::Webm => {
            &sticker.thumbnail.as_ref().filter(|thumbnail| thumbnail.format != api::ThumbnailFormat::Other)?.file
        }
    };
    Some(Picture { file: Some(file), width: sticker.width, height: sticker.height, duration: 0, preview: None })
}

/// A video's or a GIF's own file name; photos have none.
pub fn file_name(content: &M) -> &str {
    match content {
        M::Video { video, .. } => video.file_name.trim(),
        M::Animation { animation, .. } => animation.file_name.trim(),
        _ => "",
    }
}

/// The file itself: a photo's largest size, the video, the GIF.
pub fn original(content: &M) -> Option<&api::File> {
    match content {
        M::Photo { photo, .. } => photo.sizes.last().map(|size| &size.photo),
        M::Video { video, .. } => Some(&video.video),
        M::Animation { animation, .. } => Some(&animation.animation),
        _ => None,
    }
}

/// Sent to self-destruct (seen once, or for a time): shown blurred, and only once opened, as TDLib
/// asks (`is_secret`); the viewer opens it (viewer.rs).
pub fn is_secret(content: &M) -> bool {
    matches!(content, M::Photo { is_secret: true, .. } | M::Video { is_secret: true, .. } | M::Animation { is_secret: true, .. })
}

/// Self-destructing content that is gone.
pub fn is_expired(content: &M) -> bool {
    matches!(content, M::ExpiredPhoto {} | M::ExpiredVideo {} | M::ExpiredVideoNote {} | M::ExpiredVoiceNote {})
}

/// What the chat lets us send besides words.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SendRights {
    pub photos: bool,
    pub videos: bool,
    pub documents: bool,
}

impl SendRights {
    pub fn any(&self) -> bool {
        self.photos || self.videos || self.documents
    }

    fn and(&self, other: &SendRights) -> SendRights {
        SendRights { photos: self.photos && other.photos, videos: self.videos && other.videos, documents: self.documents && other.documents }
    }
}

/// The files a message's content holds that move (up or down): its photo's largest size, its video,
/// its GIF, its document.
fn message_files(content: &M) -> Vec<i32> {
    match content {
        M::Document { document, .. } => vec![document.document.id],
        other => original(other).map(|file| vec![file.id]).unwrap_or_default(),
    }
}

/// `moved` of `total`, 0 … 1.
fn part(moved: i64, total: i64) -> f32 {
    if total <= 0 { 0.0 } else { (moved as f64 / total as f64).clamp(0.0, 1.0) as f32 }
}

/// A size in the units the Finder uses: "640 KB", "2.1 MB", "2.6 GB".
pub fn size_text(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    if bytes < 1000.0 {
        format!("{} B", bytes as i64)
    } else if bytes < 1_000_000.0 {
        format!("{:.0} KB", bytes / 1000.0)
    } else if bytes < 1_000_000_000.0 {
        format!("{:.1} MB", bytes / 1_000_000.0)
    } else {
        format!("{:.1} GB", bytes / 1_000_000_000.0)
    }
}

/// A file's kind for its icon, by its name's extension.
pub fn file_type_of(name: &str) -> FileType {
    let extension = name.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase()).unwrap_or_default();
    match extension.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "heif" | "tif" | "tiff" | "bmp" | "svg" | "avif" => FileType::Image,
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "dmg" | "iso" => FileType::Archive,
        "pdf" => FileType::Pdf,
        _ => FileType::Other,
    }
}

/// A video's or a GIF's still frame, when it is a picture (not a moving one).
fn still<'a>(
    thumbnail: Option<&'a api::Thumbnail>,
    (width, height): (i32, i32),
    duration: i32,
    preview: Option<&'a api::Minithumbnail>,
) -> Picture<'a> {
    let thumbnail = thumbnail.filter(|thumbnail| thumbnail.format != api::ThumbnailFormat::Other);
    let (width, height) = match thumbnail {
        Some(thumbnail) if width <= 0 || height <= 0 => (thumbnail.width, thumbnail.height),
        _ => (width, height),
    };
    Picture { file: thumbnail.map(|thumbnail| &thumbnail.file), width, height, duration, preview }
}

/// The lists `chat` is pinned in: All chats and folders (the archive does not count).
pub fn pinned_in(chat: &api::Chat) -> Vec<ChatList> {
    chat.positions.iter().filter(|position| position.is_pinned && position.list != ChatList::Archive).map(|position| position.list).collect()
}

/// Where `chat` is in `list`, if it is in it.
pub fn position(chat: &api::Chat, list: ChatList) -> Option<&api::ChatPosition> {
    chat.positions.iter().find(|position| position.list == list && position.order != 0)
}

/// The kind of chat whose notification settings `chat` follows unless it says otherwise.
fn scope_of(chat: &api::Chat) -> NotificationSettingsScope {
    match chat.kind {
        ChatType::Private { .. } | ChatType::Secret { .. } => NotificationSettingsScope::Private,
        ChatType::BasicGroup { .. } | ChatType::Supergroup { is_channel: false, .. } => NotificationSettingsScope::Group,
        ChatType::Supergroup { is_channel: true, .. } => NotificationSettingsScope::Channel,
    }
}

fn set_position(chat: &mut api::Chat, position: api::ChatPosition) {
    chat.positions.retain(|known| known.list != position.list);
    if position.order != 0 {
        chat.positions.push(position);
    }
}

pub fn is_service(content: Content) -> bool {
    matches!(
        content,
        Content::ChatCreated
            | Content::ChannelCreated
            | Content::TitleChanged
            | Content::PhotoChanged
            | Content::PhotoRemoved
            | Content::MembersAdded
            | Content::MemberJoined
            | Content::MemberLeft
            | Content::MemberRemoved
            | Content::MessagePinned
            | Content::ScreenshotTaken
            | Content::ContactJoined
            | Content::OtherService
    )
}

/// The first line of a message, for a list's preview.
fn first_line(text: &str) -> &str {
    text.lines().find(|line| !line.trim().is_empty()).unwrap_or("").trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: i64, order: i64) -> api::Chat {
        serde_json::from_value(serde_json::json!({
            "@type": "chat", "id": id, "type": { "@type": "chatTypeBasicGroup", "basic_group_id": id },
            "title": format!("chat {id}"), "permissions": { "@type": "chatPermissions", "can_send_basic_messages": true },
            "positions": [{ "@type": "chatPosition", "list": { "@type": "chatListMain" }, "order": order.to_string(), "is_pinned": false }],
            "is_marked_as_unread": false, "unread_count": 0, "last_read_inbox_message_id": 0,
            "last_read_outbox_message_id": 0, "unread_mention_count": 0,
            "notification_settings": { "@type": "chatNotificationSettings", "use_default_mute_for": true, "mute_for": 0 }
        }))
        .expect("a chat")
    }

    fn key(pinned: bool, order: i64, title: &str, unread: i32) -> SortKey {
        SortKey { pinned, order, id: order, title: title.into(), unread }
    }

    #[test]
    fn the_pinned_chats_come_first_in_every_order_then_the_chosen_one() {
        let names = NameOrder::new("en");
        let pinned = key(true, 1, "zed", 0);
        let newest = key(false, 9, "mike", 0);
        let busy = key(false, 5, "alice", 7);
        for order in [ChatOrder::LatestMessage, ChatOrder::Name, ChatOrder::Unread] {
            assert_eq!(compare(&pinned, &newest, order, &names), Ordering::Less);
            assert_eq!(compare(&busy, &pinned, order, &names), Ordering::Greater);
        }
        assert_eq!(compare(&newest, &busy, ChatOrder::LatestMessage, &names), Ordering::Less);
        assert_eq!(compare(&newest, &busy, ChatOrder::Name, &names), Ordering::Greater);
        assert_eq!(compare(&newest, &busy, ChatOrder::Unread, &names), Ordering::Greater);
        // Two pinned chats keep Telegram's order; ties among the rest go to the newest message.
        assert_eq!(compare(&key(true, 1, "a", 9), &key(true, 2, "b", 0), ChatOrder::Unread, &names), Ordering::Greater);
        assert_eq!(compare(&key(false, 9, "same", 2), &key(false, 5, "same", 2), ChatOrder::Unread, &names), Ordering::Less);
    }

    #[test]
    fn positions_are_replaced_per_list_and_removed_at_zero() {
        let mut chat = chat(1, 10);
        set_position(&mut chat, api::ChatPosition { list: ChatList::Main, order: 20, is_pinned: true });
        assert_eq!(position(&chat, ChatList::Main).map(|position| position.order), Some(20));
        set_position(&mut chat, api::ChatPosition { list: ChatList::Folder { chat_folder_id: 2 }, order: 5, is_pinned: false });
        assert_eq!(chat.positions.len(), 2);
        set_position(&mut chat, api::ChatPosition { list: ChatList::Main, order: 0, is_pinned: false });
        assert!(position(&chat, ChatList::Main).is_none());
        assert_eq!(chat.positions.len(), 1);
    }

    #[test]
    fn a_sticker_shows_itself_or_its_still_thumbnail() {
        let file = |id: i32| api::File { id, size: 0, expected_size: 0, local: api::LocalFile::default(), remote: api::RemoteFile::default() };
        let sticker = |format: api::StickerFormat, thumbnail: Option<api::ThumbnailFormat>| M::Sticker {
            sticker: api::Sticker {
                id: 0,
                width: 512,
                height: 480,
                emoji: "😀".into(),
                format,
                thumbnail: thumbnail.map(|format| api::Thumbnail { format, width: 128, height: 120, file: file(1) }),
                sticker: file(2),
            },
        };
        let shown = |content: &M| sticker_picture(content).map(|picture| (picture.file.map(|file| file.id), picture.width, picture.height));
        assert_eq!(shown(&sticker(api::StickerFormat::Webp, Some(api::ThumbnailFormat::Webp))), Some((Some(2), 512, 480)));
        assert_eq!(shown(&sticker(api::StickerFormat::Tgs, Some(api::ThumbnailFormat::Webp))), Some((Some(1), 512, 480)));
        assert_eq!(shown(&sticker(api::StickerFormat::Webm, Some(api::ThumbnailFormat::Other))), None);
        assert_eq!(shown(&sticker(api::StickerFormat::Tgs, None)), None);
        assert!(picture(&sticker(api::StickerFormat::Webp, None)).is_none(), "stickers stay out of the media viewer");
    }

    #[test]
    fn the_first_line_is_the_preview() {
        assert_eq!(first_line("\n  New rules\n\nThe details"), "New rules");
        assert_eq!(first_line(""), "");
    }
}
