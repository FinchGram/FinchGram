//! How long the pages take to follow the store, measured with Slint's software renderer and made-up
//! data, the way the screenshots are drawn: an account with many chats and an open chat with many
//! messages, then what one new message, one downloaded picture and one user's status cost. Nothing
//! is asserted; the times are printed.
//!
//! ```sh
//! cargo test --release refresh_timing -- --ignored --nocapture
//! ```
//!
//! It takes over Slint's platform for the whole test process, so it only runs when asked for.

use std::rc::Rc;
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{Value, json};
use slint::ComponentHandle;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};

use super::api::Update;
use super::store::{self, History};
use crate::{AppState, MainWindow, Page, TelegramState};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 824;
const CHATS: i64 = 1000;
const FOLDERS: i32 = 4;
const MESSAGES: i64 = 600;
/// The account, and the open chat, a group.
const ME: i64 = 777;
const CHAT: i64 = -1_001_000_000_001;
const CHAT_SUPERGROUP: i64 = 1_000_000_001;

struct Timing {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Timing {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
}

#[test]
#[ignore = "takes over Slint's platform; run with: cargo test --release refresh_timing -- --ignored --nocapture"]
fn refresh_timing() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Timing { window: window.clone() })).expect("platform");
    crate::fonts::register();
    window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));

    let ui = MainWindow::new().expect("window");
    ui.show().expect("show");
    let app = ui.global::<AppState>();
    app.set_telegram_state(TelegramState::Ready);
    app.set_page(Page::Chats);
    store::install(&ui);

    let took = timed(|| {
        store::with(|store| {
            store.my_id = ME;
            for update in account() {
                store.apply(update);
            }
            let mut history = History { has_older: true, ..History::default() };
            for message in messages(1..=MESSAGES) {
                history.messages.insert(message.id, message);
            }
            store.histories.insert(CHAT, history);
            store.tabs = vec![CHAT, -1, -2];
            store.open = Some(CHAT);
            store.dirty.chats = true;
            store.dirty.conversation = true;
            store.dirty.account = true;
            store.dirty.scroll_to_end = true;
        });
    });
    eprintln!("made up {CHATS} chats in {FOLDERS} folders and {MESSAGES} messages: {took:?}");
    report("the first refresh", store::refresh);
    report("the first render (every row made)", || render(&window));
    report("a render with nothing changed", || render(&window));

    for round in 1..=3 {
        let id = MESSAGES + round;
        let message = messages(id..=id).remove(0);
        let update = Update::NewMessage { message: Box::new(message) };
        store::with(|store| store.apply(update));
        report(&format!("a new message, refresh ({round})"), store::refresh);
        report(&format!("a new message, render ({round})"), || render(&window));
    }

    store::with(|store| store.dirty.scroll_to_end = true);
    report("scrolling to the end, refresh", store::refresh);
    report("scrolling to the end, render", || render(&window));

    let update = parse(json!({
        "@type": "updateMessageContent", "chat_id": CHAT, "message_id": 14,
        "new_content": text("Message 14, edited: a line or so of ordinary words about the keyboards and the meetup, and a few more."),
    }));
    store::with(|store| store.apply(update));
    report("a message is edited, refresh", store::refresh);
    report("a message is edited, render", || render(&window));

    let update = parse(json!({ "@type": "updateUserStatus", "user_id": 5, "status": { "@type": "userStatusOnline", "expires": 2_000_000_000 } }));
    store::with(|store| store.apply(update));
    report("a user comes online, refresh", store::refresh);
    report("a user comes online, render", || render(&window));

    let update = parse(json!({ "@type": "updateChatAction", "chat_id": CHAT, "sender_id": { "@type": "messageSenderUser", "user_id": 5 }, "action": { "@type": "chatActionTyping" } }));
    store::with(|store| store.apply(update));
    report("someone types, refresh", store::refresh);
    report("someone types, render", || render(&window));

    let picture = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(800, 600));
    store::with(|store| {
        store.pictures.insert(photo_file(20) + 1, picture);
        store.dirty.conversation = true;
    });
    report("a picture arrives, refresh", store::refresh);
    report("a picture arrives, render", || render(&window));

    store::with(|store| {
        let history = store.histories.get_mut(&CHAT).expect("the open chat");
        for message in messages(-50..=0) {
            history.messages.insert(message.id, message);
        }
        store.dirty.conversation = true;
    });
    report("an older page of 50, refresh", store::refresh);
    report("an older page of 50, render", || render(&window));

    // The view up among the oldest messages, every one of them shown: each change costs with them.
    let all = store::with(|store| {
        let history = store.histories.get_mut(&CHAT).expect("the open chat");
        history.shown = history.messages.len();
        store.dirty.conversation = true;
        history.messages.len()
    })
    .expect("the store");
    report(&format!("every message shown ({all}), refresh"), store::refresh);
    report(&format!("every message shown ({all}), render"), || render(&window));
    let id = MESSAGES + 4;
    let message = messages(id..=id).remove(0);
    store::with(|store| store.apply(Update::NewMessage { message: Box::new(message) }));
    report(&format!("a new message among {all}, refresh"), store::refresh);
    report(&format!("a new message among {all}, render"), || render(&window));
    let picture = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(800, 600));
    store::with(|store| {
        store.pictures.insert(photo_file(40) + 1, picture);
        store.dirty.conversation = true;
    });
    report(&format!("a picture arrives among {all}, refresh"), store::refresh);
    report(&format!("a picture arrives among {all}, render"), || render(&window));
    save(&window, "timing-chat");
}

/// The window as it is now, into target/screenshots/`name`.png: the rows as the store builds them,
/// which the screenshots (made-up rows) do not show.
fn save(window: &MinimalSoftwareWindow, name: &str) {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("screenshots");
    std::fs::create_dir_all(&directory).expect("target/screenshots");
    let mut pixels = vec![slint::Rgb8Pixel::default(); (WIDTH * HEIGHT) as usize];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, WIDTH as usize);
    });
    let bytes: Vec<u8> = pixels.iter().flat_map(|pixel| [pixel.r, pixel.g, pixel.b]).collect();
    image::save_buffer(directory.join(format!("{name}.png")), &bytes, WIDTH, HEIGHT, image::ColorType::Rgb8).expect("write the picture");
}

fn report(what: &str, work: impl FnOnce()) {
    let took = timed(work);
    eprintln!("{what}: {took:?}");
}

fn timed(work: impl FnOnce()) -> Duration {
    let start = Instant::now();
    work();
    start.elapsed()
}

fn render(window: &MinimalSoftwareWindow) {
    let mut pixels = vec![slint::Rgb8Pixel::default(); (WIDTH * HEIGHT) as usize];
    slint::platform::update_timers_and_animations();
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, WIDTH as usize);
    });
}

fn parse(value: Value) -> Update {
    serde_json::from_value(value).expect("an update")
}

/// The account: its users, its chats (private chats, groups and channels, every fifth in one of the
/// folders), its folders, and the open chat's group.
fn account() -> Vec<Update> {
    let mut updates = Vec::new();
    for id in 1..=CHATS {
        updates.push(parse(json!({
            "@type": "updateUser",
            "user": {
                "@type": "user", "id": id, "first_name": format!("User {id}"), "last_name": "Example", "phone_number": "",
                "status": { "@type": "userStatusOffline", "was_online": 1_790_000_000 },
                "type": { "@type": "userTypeRegular" },
            },
        })));
    }
    updates.push(parse(json!({
        "@type": "updateSupergroup",
        "supergroup": { "@type": "supergroup", "id": CHAT_SUPERGROUP, "status": { "@type": "chatMemberStatusMember" }, "member_count": 486 },
    })));
    // Chat 0 is the open one, a group; the others are private chats, groups and channels.
    for id in 0..=CHATS {
        let (chat_id, kind, title) = match id % 4 {
            0 if id == 0 => (CHAT, json!({ "@type": "chatTypeSupergroup", "supergroup_id": CHAT_SUPERGROUP, "is_channel": false }), "Keyboard Lab".to_string()),
            0 => (-1_001_000_000_000 - id, json!({ "@type": "chatTypeSupergroup", "supergroup_id": 1_000_000_000 + id, "is_channel": false }), format!("Group {id}")),
            1 => (-1_002_000_000_000 - id, json!({ "@type": "chatTypeSupergroup", "supergroup_id": 2_000_000_000 + id, "is_channel": true }), format!("Channel {id}")),
            _ => (id, json!({ "@type": "chatTypePrivate", "user_id": id }), format!("User {id} Example")),
        };
        let order = (CHATS - id + 1) * 1000;
        let mut positions = vec![json!({ "@type": "chatPosition", "list": { "@type": "chatListMain" }, "order": order.to_string(), "is_pinned": id <= 3 })];
        if id % 5 != 0 {
            let folder = (id % 5) as i32;
            positions.push(json!({ "@type": "chatPosition", "list": { "@type": "chatListFolder", "chat_folder_id": folder }, "order": order.to_string(), "is_pinned": false }));
        }
        updates.push(parse(json!({
            "@type": "updateNewChat",
            "chat": {
                "@type": "chat", "id": chat_id, "type": kind, "title": title,
                "permissions": { "@type": "chatPermissions", "can_send_basic_messages": true },
                "last_message": message(id, chat_id, id, 1_790_000_000 + id as i32, text(&format!("The last message of chat {id}, a line long enough to be cut short in the list")), None),
                "positions": positions, "is_marked_as_unread": false, "unread_count": if id % 7 == 0 { 3 } else { 0 },
                "last_read_inbox_message_id": 0, "last_read_outbox_message_id": 0, "unread_mention_count": 0,
                "notification_settings": { "@type": "chatNotificationSettings", "use_default_mute_for": true, "mute_for": 0 },
            },
        })));
    }
    let folders: Vec<Value> = (1..=FOLDERS)
        .map(|id| json!({ "@type": "chatFolderInfo", "id": id, "name": { "@type": "chatFolderName", "text": { "@type": "formattedText", "text": format!("Folder {id}"), "entities": [] } } }))
        .collect();
    updates.push(parse(json!({ "@type": "updateChatFolders", "chat_folders": folders })));
    updates
}

/// Messages `ids` of the open chat: plain words, formatted words, photos and replies, from several
/// users over several days.
fn messages(ids: std::ops::RangeInclusive<i64>) -> Vec<super::api::Message> {
    let preview = preview();
    ids.map(|id| {
        let sender = if id % 6 == 0 { ME } else { 1 + id % 40 };
        let date = 1_790_000_000 + (id * 1800) as i32;
        let content = match id % 20 {
            0..=2 => photo(photo_file(id), &preview),
            3..=7 => formatted(&format!("Update {id}: the *group buy* closes soon, see https://example.org/orders/{id} for the list and the dates.")),
            8 => text("ok"),
            9 => text(&format!("A longer message number {id}, wrapping over several lines in the chat: the keycaps arrived this morning, the switches are still on their way, and the plates were cut yesterday, so Thursday still looks fine for everyone who ordered the second batch.")),
            10..=12 => text(&format!("第 {id} 条：今天早上键帽到了，轴体还在路上，定位板昨天切好了，周四的聚会照旧，第二批订了的都来一下。")),
            13 => text("好的，收到。"),
            _ => text(&format!("Message {id}, a line or so of ordinary words about the keyboards and the meetup.")),
        };
        let reply_to = (id % 10 == 5 && id > 1).then(|| id - 1);
        serde_json::from_value(message(id, CHAT, sender, date, content, reply_to)).expect("a message")
    })
    .collect()
}

fn message(id: i64, chat_id: i64, sender: i64, date: i32, content: Value, reply_to: Option<i64>) -> Value {
    let reply_to = reply_to.map(|message_id| {
        json!({ "@type": "messageReplyToMessage", "chat_id": chat_id, "message_id": message_id, "quote": null, "origin": null, "content": null })
    });
    json!({
        "@type": "message", "id": id, "sender_id": { "@type": "messageSenderUser", "user_id": sender }, "chat_id": chat_id,
        "is_outgoing": sender == ME, "date": date, "edit_date": 0, "media_album_id": "0", "reply_to": reply_to, "content": content,
    })
}

fn text(words: &str) -> Value {
    json!({ "@type": "messageText", "text": { "@type": "formattedText", "text": words, "entities": [] }, "link_preview": null })
}

/// The words with the part between the asterisks bold and the address a link.
fn formatted(words: &str) -> Value {
    let plain = words.replace('*', "");
    let bold_start = words.find('*').expect("a star") as i32;
    let bold_length = (words.rfind('*').expect("a star") as i32) - bold_start - 1;
    let link_start = plain.find("https://").expect("a link");
    let link_length = plain[link_start..].split(' ').next().expect("the link").len() as i32;
    json!({
        "@type": "messageText",
        "text": {
            "@type": "formattedText", "text": plain,
            "entities": [
                { "@type": "textEntity", "offset": bold_start, "length": bold_length, "type": { "@type": "textEntityTypeBold" } },
                { "@type": "textEntity", "offset": link_start, "length": link_length, "type": { "@type": "textEntityTypeUrl" } },
            ],
        },
        "link_preview": null,
    })
}

fn photo_file(message_id: i64) -> i32 {
    100_000 + message_id as i32
}

fn photo(file_id: i32, preview: &str) -> Value {
    let file = |id: i32| json!({ "@type": "file", "id": id, "size": 0, "local": { "@type": "localFile", "path": "", "is_downloading_completed": false } });
    json!({
        "@type": "messagePhoto",
        "photo": {
            "@type": "photo",
            "minithumbnail": { "@type": "minithumbnail", "width": 40, "height": 30, "data": preview },
            "sizes": [
                { "@type": "photoSize", "type": "m", "photo": file(file_id), "width": 320, "height": 240 },
                { "@type": "photoSize", "type": "x", "photo": file(file_id + 1), "width": 800, "height": 600 },
            ],
        },
        "caption": { "@type": "formattedText", "text": "", "entities": [] },
    })
}

/// A tiny JPEG, as the preview a message carries.
fn preview() -> String {
    let picture = image::RgbImage::from_pixel(40, 30, image::Rgb([74, 95, 193]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    picture.write_to(&mut bytes, image::ImageFormat::Jpeg).expect("a JPEG");
    base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())
}
