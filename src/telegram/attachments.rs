//! Sending photos, videos and files (the design's sixth round, "FinchGram Desktop 发送附件"): the
//! paperclip's menu opens the system's open panel, files dragged onto the window or pasted come
//! the same way, and everything chosen is shown in a card before it goes, with a caption and the
//! choices Telegram's own apps give (as a photo or as a file, together as an album, a self-destruct
//! timer in a private chat, without sound). The sending is TDLib's: sendMessage, or
//! sendMessageAlbum for an album; the messages show themselves on their way (updateNewMessage,
//! updateFile).
//!
//! A picture goes as a photo and a video as a video only when its file is one Telegram takes as
//! such (JPEG, PNG, WebP; MP4, M4V, MOV); anything else goes as a file, as it is.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use slint::{ComponentHandle, Image, ModelRc, VecModel};

use super::api::ChatType;
use super::store::{self, SendRights};
use super::{actions, send, with_ui};
use crate::player::probe;
use crate::{AttachHint, AttachKind, AttachProblem, Attachment, Attachments, MainWindow, images, platform};

/// Telegram's limit on a file, without Premium and with it.
const FILE_LIMIT: i64 = 2 * 1024 * 1024 * 1024;
const FILE_LIMIT_PREMIUM: i64 = 4 * 1024 * 1024 * 1024;
/// An album holds up to this many photos and videos.
const ALBUM: usize = 10;
/// Pictures and videos sent as such: what Telegram takes as a photo and as a video.
const PHOTO_EXTENSIONS: [&str; 4] = ["jpg", "jpeg", "png", "webp"];
const VIDEO_EXTENSIONS: [&str; 3] = ["mp4", "m4v", "mov"];
/// Telegram wants a video's still at most this wide (src/player/probe.rs makes it so).
const STILL_WIDTH: i32 = 320;

/// What a file is, by its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Photo,
    Video,
    File,
}

struct Item {
    id: i32,
    path: PathBuf,
    name: String,
    size: i64,
    kind: Kind,
    /// Its file could not be read: it cannot be sent.
    unreadable: bool,
    width: i32,
    height: i32,
    duration: i32,
    /// A video's still, written by the probe, for its message.
    still: Option<PathBuf>,
    picture: Option<Image>,
}

#[derive(Default)]
struct State {
    /// The card is open for this chat.
    open: Option<i64>,
    items: Vec<Item>,
    next_id: i32,
    as_file: bool,
    grouped: bool,
    /// Seconds; 0 off; -1 view once.
    timer: i32,
    caption: String,
    /// The card opened for a screenshot just taken (src/screenshot/): while it is the only item,
    /// the card is titled "Send Screenshot".
    screenshot: bool,
    /// Files dragged over the window, as winit names them one by one; and dropped.
    hovering: Vec<PathBuf>,
    dropped: Vec<PathBuf>,
    flush_asked: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State { grouped: true, ..State::default() });
}

pub fn connect(ui: &MainWindow) {
    let attachments = ui.global::<Attachments>();
    attachments.on_choose(|kind| choose(kind == AttachKind::File));
    attachments.on_add_more(|| {
        let files_only = STATE.with(|state| {
            let state = state.borrow();
            state.as_file || state.items.iter().any(|item| item.kind == Kind::File)
        });
        choose(files_only);
    });
    attachments.on_remove(remove);
    attachments.on_set_as_file(|as_file| {
        STATE.with(|state| state.borrow_mut().as_file = as_file);
        refresh();
    });
    attachments.on_set_grouped(|grouped| {
        STATE.with(|state| state.borrow_mut().grouped = grouped);
        refresh();
    });
    attachments.on_set_timer(|timer| {
        STATE.with(|state| state.borrow_mut().timer = timer);
        refresh();
    });
    attachments.on_caption_edited(|words| {
        STATE.with(|state| state.borrow_mut().caption = words.to_string());
        refresh();
    });
    attachments.on_send(send_all);
    attachments.on_cancel(cancel);
    attachments.on_paste(paste);
}

/// The open panel: for pictures and videos, or for any files, which go as they are.
fn choose(files_only: bool) {
    if open_chat().is_none() {
        return;
    }
    platform::choose_files(!files_only, move |paths| {
        if !paths.is_empty() {
            add(paths, files_only);
        }
    });
}

/// Files chosen, dropped or pasted: into the card, which opens for the chat in front if it was
/// closed. `as_files`: they were asked for as files, and go as such whatever they are.
fn add(paths: Vec<PathBuf>, as_files: bool) {
    let Some(chat_id) = open_chat() else { return };
    let mut loads = Vec::new();
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.open != Some(chat_id) {
            *state = State { open: Some(chat_id), grouped: true, ..State::default() };
        }
        if as_files {
            state.as_file = true;
        }
        for path in paths {
            let id = state.next_id;
            state.next_id += 1;
            let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
            let (size, unreadable) = match std::fs::metadata(&path) {
                Ok(metadata) if metadata.is_file() => (metadata.len() as i64, false),
                _ => (0, true),
            };
            let kind = if unreadable { Kind::File } else { kind_of(&path) };
            let (width, height) = if kind == Kind::Photo { image::image_dimensions(&path).map(|(w, h)| (w as i32, h as i32)).unwrap_or((0, 0)) } else { (0, 0) };
            state.items.push(Item { id, path: path.clone(), name, size, kind, unreadable, width, height, duration: 0, still: None, picture: None });
            if !unreadable {
                loads.push((id, path, kind));
            }
        }
    });
    with_ui(|ui| ui.global::<Attachments>().set_card_open(true));
    refresh();
    for (id, path, kind) in loads {
        match kind {
            Kind::Photo => images::load(path, move |picture| {
                set_picture(id, picture);
            }),
            Kind::Video => {
                let name = format!("still-{id}-{}", unix_now());
                probe::probe(path, scratch_dir(), name, move |info| {
                    let Some(info) = info else { return };
                    STATE.with(|state| {
                        if let Some(item) = state.borrow_mut().items.iter_mut().find(|item| item.id == id) {
                            item.duration = info.duration;
                            item.width = info.width;
                            item.height = info.height;
                            item.still = info.thumbnail.clone();
                        }
                    });
                    refresh();
                    if let Some(still) = info.thumbnail {
                        images::load(still, move |picture| set_picture(id, picture));
                    }
                });
            }
            Kind::File => {}
        }
    }
}

/// A screenshot just taken (src/screenshot/): into the card as a photo, which opens for the chat
/// in front with it alone, titled "Send Screenshot"; the caption takes the keyboard.
pub(super) fn add_screenshot(path: PathBuf) {
    if open_chat().is_none() {
        return;
    }
    add(vec![path], false);
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.screenshot = state.items.len() == 1;
    });
    refresh();
}

fn set_picture(id: i32, picture: Option<Image>) {
    let Some(picture) = picture else { return };
    STATE.with(|state| {
        if let Some(item) = state.borrow_mut().items.iter_mut().find(|item| item.id == id) {
            item.picture = Some(picture);
        }
    });
    refresh();
}

fn remove(id: i32) {
    let empty = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.items.retain(|item| item.id != id);
        state.items.is_empty()
    });
    if empty {
        cancel();
    } else {
        refresh();
    }
}

/// Close the card and forget what it held.
fn cancel() {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.open = None;
        state.items.clear();
        state.caption.clear();
    });
    with_ui(|ui| {
        let attachments = ui.global::<Attachments>();
        attachments.set_card_open(false);
        attachments.set_timer_open(false);
        attachments.set_send_menu_open(false);
        attachments.set_busy(false);
        attachments.set_items(ModelRc::default());
    });
    actions::focus_composer();
}

/// Esc: the card closes if it is open. False: it was not.
pub fn escape() -> bool {
    let open = STATE.with(|state| state.borrow().open.is_some());
    if open {
        cancel();
    }
    open
}

/// ⌘V in the composer: files copied in the Finder, or a picture, open the card.
fn paste() -> bool {
    let files = platform::pasteboard_files();
    if !files.is_empty() {
        add(files, false);
        return true;
    }
    let Some(png) = platform::pasteboard_image() else { return false };
    let path = scratch_dir().join(format!("pasted-{}.png", unix_now()));
    if std::fs::create_dir_all(scratch_dir()).and_then(|()| std::fs::write(&path, png)).is_err() {
        return false;
    }
    add(vec![path], false);
    true
}

// ---- dragging files in -------------------------------------------------------------------------

/// A file dragged over the window (winit's HoveredFile, one per file).
pub fn hovering(path: PathBuf) {
    STATE.with(|state| state.borrow_mut().hovering.push(path));
    show_drop_zone();
}

/// The drag left the window without dropping.
pub fn hover_ended() {
    STATE.with(|state| state.borrow_mut().hovering.clear());
    show_drop_zone();
}

/// A file dropped on the window (winit's DroppedFile, one per file): taken with the others of the
/// same drop once they have all come.
pub fn dropped(path: PathBuf) {
    let first = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.dropped.push(path);
        !std::mem::replace(&mut state.flush_asked, true)
    });
    if first {
        slint::Timer::single_shot(std::time::Duration::ZERO, flush_dropped);
    }
}

fn flush_dropped() {
    let paths = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.flush_asked = false;
        state.hovering.clear();
        std::mem::take(&mut state.dropped)
    });
    show_drop_zone();
    if drop_allowed() && !paths.is_empty() {
        add(paths, false);
    }
}

/// Whether what is dragged could be sent to the chat in front.
fn drop_allowed() -> bool {
    open_chat().and_then(|chat_id| store::with(|store| store.chats.get(&chat_id).map(|chat| store.send_rights(chat).any()))).flatten().unwrap_or(false)
}

fn show_drop_zone() {
    let (dropping, photos) = STATE.with(|state| {
        let state = state.borrow();
        (!state.hovering.is_empty(), state.hovering.iter().all(|path| kind_of(path) != Kind::File))
    });
    let allowed = drop_allowed();
    with_ui(|ui| {
        let attachments = ui.global::<Attachments>();
        attachments.set_dropping(dropping && open_chat().is_some());
        attachments.set_drop_allowed(allowed);
        attachments.set_drop_photos(photos);
    });
}

// ---- the card ----------------------------------------------------------------------------------

/// Bring the card up to date with what it holds and what can be done with it.
fn refresh() {
    let Some((chat_id, rights, private, is_premium, caption_max)) = open_chat().and_then(|chat_id| {
        store::with(|store| {
            let chat = store.chats.get(&chat_id)?;
            let private = matches!(chat.kind, ChatType::Private { .. } | ChatType::Secret { .. });
            Some((chat_id, store.send_rights(chat), private, store.is_premium, store.caption_length_max))
        })
        .flatten()
    }) else {
        return;
    };
    let limit = if is_premium { FILE_LIMIT_PREMIUM } else { FILE_LIMIT };
    STATE.with(|state| {
        let state = state.borrow();
        if state.open != Some(chat_id) {
            return;
        }
        let all_media = !state.items.is_empty() && state.items.iter().all(|item| item.kind != Kind::File);
        let as_file = state.as_file || !all_media;
        let mut problems = false;
        let items: Vec<Attachment> = state
            .items
            .iter()
            .map(|item| {
                let goes_as = if as_file { Kind::File } else { item.kind };
                let problem = if item.unreadable {
                    AttachProblem::Unreadable
                } else if item.size > limit {
                    if is_premium { AttachProblem::TooBigPremium } else { AttachProblem::TooBig }
                } else if !allowed(goes_as, &rights) {
                    AttachProblem::NotAllowed
                } else {
                    AttachProblem::None
                };
                problems |= problem != AttachProblem::None;
                Attachment {
                    id: item.id,
                    kind: match goes_as {
                        Kind::Photo => AttachKind::Photo,
                        Kind::Video => AttachKind::Video,
                        Kind::File => AttachKind::File,
                    },
                    media: item.kind != Kind::File,
                    name: item.name.clone().into(),
                    size: store::size_text(item.size).into(),
                    picture: item.picture.clone().unwrap_or_default(),
                    has_picture: item.picture.is_some(),
                    width: item.width,
                    height: item.height,
                    duration: item.duration,
                    file_type: store::file_type_of(&item.name),
                    problem,
                }
            })
            .collect();
        let count = items.len() as i32;
        let videos = state.items.iter().filter(|item| item.kind == Kind::Video).count() as i32;
        let albums = if !as_file && state.grouped && state.items.len() > ALBUM { state.items.len().div_ceil(ALBUM) as i32 } else { 0 };
        let caption_length = state.caption.encode_utf16().count() as i32;
        let caption_long = caption_length > caption_max;
        let hint = if problems {
            AttachHint::RemoveMarked
        } else if caption_long {
            AttachHint::CaptionLong
        } else {
            AttachHint::Keys
        };
        let can_send = count > 0 && !problems && !caption_long;
        with_ui(|ui| {
            let attachments = ui.global::<Attachments>();
            attachments.set_items(ModelRc::new(VecModel::from(items)));
            attachments.set_count(count);
            attachments.set_videos(videos);
            attachments.set_albums(albums);
            attachments.set_all_media(all_media);
            attachments.set_as_file(as_file);
            attachments.set_screenshot(state.screenshot && state.items.len() == 1);
            attachments.set_grouped(state.grouped);
            attachments.set_timer_allowed(private && !as_file);
            attachments.set_timer(state.timer);
            attachments.set_caption_max(caption_max);
            attachments.set_can_send(can_send);
            attachments.set_hint(hint);
        });
    });
}

fn allowed(kind: Kind, rights: &SendRights) -> bool {
    match kind {
        Kind::Photo => rights.photos,
        Kind::Video => rights.videos,
        Kind::File => rights.documents,
    }
}

// ---- sending -----------------------------------------------------------------------------------

/// Send what the card holds: photos and videos as an album of up to ten when grouped, else one by
/// one; files one by one; the caption on the first. `silent`: without sound.
fn send_all(silent: bool) {
    let Some((chat_id, items, as_file, grouped, timer, caption)) = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let chat_id = state.open?;
        let items = std::mem::take(&mut state.items);
        Some((chat_id, items, state.as_file, state.grouped, state.timer, std::mem::take(&mut state.caption)))
    }) else {
        return;
    };
    let all_media = items.iter().all(|item| item.kind != Kind::File);
    let as_file = as_file || !all_media;
    let reply_to = actions::take_reply(chat_id).map(|message_id| {
        json!({ "@type": "inputMessageReplyToMessage", "message_id": message_id, "quote": null, "checklist_task_id": 0, "poll_option_id": "" })
    });
    let options = silent.then(|| json!({ "@type": "messageSendOptions", "disable_notification": true }));
    let caption = caption.trim().to_string();
    let mut caption = Some(caption).filter(|caption| !caption.is_empty());
    let (media, files): (Vec<&Item>, Vec<&Item>) = items.iter().partition(|item| !as_file && item.kind != Kind::File);
    let chunk = if grouped { ALBUM } else { 1 };
    for album in media.chunks(chunk) {
        let contents: Vec<Value> = album.iter().map(|item| media_content(item, caption.take(), timer)).collect();
        send_contents(chat_id, contents, reply_to.clone(), options.clone());
    }
    for item in files {
        let content = file_content(item, caption.take(), as_file && item.kind != Kind::File);
        send_contents(chat_id, vec![content], reply_to.clone(), options.clone());
    }
    store::with(|store| store.dirty.scroll_to_end = true);
    cancel();
}

fn send_contents(chat_id: i64, mut contents: Vec<Value>, reply_to: Option<Value>, options: Option<Value>) {
    let request = if contents.len() == 1 {
        json!({
            "@type": "sendMessage", "chat_id": chat_id, "topic_id": null, "reply_to": reply_to, "options": options,
            "reply_markup": null, "input_message_content": contents.remove(0),
        })
    } else {
        json!({
            "@type": "sendMessageAlbum", "chat_id": chat_id, "topic_id": null, "reply_to": reply_to, "options": options,
            "input_message_contents": contents,
        })
    };
    send(request, |answer| {
        if let Err(err) = answer {
            eprintln!("attachments: cannot send: {err}");
        }
    });
}

fn media_content(item: &Item, caption: Option<String>, timer: i32) -> Value {
    let caption = formatted(caption);
    let self_destruct = match timer {
        -1 => json!({ "@type": "messageSelfDestructTypeImmediately" }),
        seconds if seconds > 0 => json!({ "@type": "messageSelfDestructTypeTimer", "self_destruct_time": seconds }),
        _ => Value::Null,
    };
    match item.kind {
        Kind::Video => {
            let thumbnail = item.still.as_ref().map(|still| {
                let width = item.width.clamp(1, STILL_WIDTH);
                let height = if item.width > 0 { (i64::from(item.height) * i64::from(width) / i64::from(item.width)) as i32 } else { 0 };
                json!({ "@type": "inputThumbnail", "thumbnail": local(still), "width": width, "height": height })
            });
            json!({
                "@type": "inputMessageVideo",
                "video": {
                    "@type": "inputVideo", "video": local(&item.path), "thumbnail": thumbnail, "cover": null, "start_timestamp": 0,
                    "added_sticker_file_ids": [], "duration": item.duration, "width": item.width, "height": item.height,
                    "supports_streaming": true,
                },
                "caption": caption, "show_caption_above_media": false, "self_destruct_type": self_destruct, "has_spoiler": false,
            })
        }
        _ => json!({
            "@type": "inputMessagePhoto",
            "photo": {
                "@type": "inputPhoto", "photo": local(&item.path), "thumbnail": null, "video": null, "added_sticker_file_ids": [],
                "width": item.width, "height": item.height,
            },
            "caption": caption, "show_caption_above_media": false, "self_destruct_type": self_destruct, "has_spoiler": false,
        }),
    }
}

/// A file as it is. `as_file`: a picture or video the user chose to send as a file, which TDLib
/// would otherwise turn into a photo or video anyway.
fn file_content(item: &Item, caption: Option<String>, as_file: bool) -> Value {
    json!({
        "@type": "inputMessageDocument",
        "document": { "@type": "inputDocument", "document": local(&item.path), "thumbnail": null, "disable_content_type_detection": as_file },
        "caption": formatted(caption),
    })
}

fn local(path: &Path) -> Value {
    json!({ "@type": "inputFileLocal", "path": path.to_string_lossy() })
}

fn formatted(caption: Option<String>) -> Value {
    json!({ "@type": "formattedText", "text": caption.unwrap_or_default(), "entities": [] })
}

// ---- helpers -----------------------------------------------------------------------------------

fn open_chat() -> Option<i64> {
    store::with(|store| store.open).flatten()
}

/// What a file goes as, by its extension.
fn kind_of(path: &Path) -> Kind {
    let extension = path.extension().map(|extension| extension.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if PHOTO_EXTENSIONS.contains(&extension.as_str()) {
        Kind::Photo
    } else if VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        Kind::Video
    } else {
        Kind::File
    }
}

/// Where pasted pictures and videos' stills are written before they are sent.
fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join("finchgram-attachments")
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|since| since.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_go_as_what_their_names_say() {
        assert_eq!(kind_of(Path::new("/a/IMG_2041.JPG")), Kind::Photo);
        assert_eq!(kind_of(Path::new("/a/clip.mov")), Kind::Video);
        assert_eq!(kind_of(Path::new("/a/notes.pdf")), Kind::File);
        assert_eq!(kind_of(Path::new("/a/README")), Kind::File);
    }

    #[test]
    fn a_view_once_photo_and_a_timed_video_say_so() {
        let photo = Item { id: 1, path: "/a/p.jpg".into(), name: "p.jpg".into(), size: 1, kind: Kind::Photo, unreadable: false, width: 4, height: 3, duration: 0, still: None, picture: None };
        let content = media_content(&photo, Some("hi".into()), -1);
        assert_eq!(content["@type"], "inputMessagePhoto");
        assert_eq!(content["self_destruct_type"]["@type"], "messageSelfDestructTypeImmediately");
        assert_eq!(content["caption"]["text"], "hi");
        let video = Item { kind: Kind::Video, duration: 42, width: 1280, height: 720, still: Some("/a/still.jpg".into()), ..photo };
        let content = media_content(&video, None, 10);
        assert_eq!(content["@type"], "inputMessageVideo");
        assert_eq!(content["self_destruct_type"]["self_destruct_time"], 10);
        assert_eq!(content["video"]["thumbnail"]["width"], 320);
        assert_eq!(content["video"]["thumbnail"]["height"], 180);
    }
}
