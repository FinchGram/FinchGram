//! Pictures of every page in every theme, light and dark, drawn with Slint's software renderer and
//! made-up data: a way to look at the UI without Telegram, an account or a window.
//!
//! ```sh
//! cargo test screenshots -- --ignored
//! ```
//!
//! writes them to `target/screenshots/<theme>-<appearance>-<page>.png`. It takes over Slint's
//! platform for the whole test process, so it only runs when asked for.

use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Key, PointerEventButton, Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use crate::*;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
/// The menu bar Slint draws inside the window when there is no native one.
const MENU_BAR: u32 = 24;

struct Screenshots {
    window: Rc<MinimalSoftwareWindow>,
}

thread_local! {
    /// The pictures' own clock: each picture is a second after the one before, so that what moves
    /// (a colour fading in) has arrived.
    static CLOCK: std::cell::Cell<std::time::Duration> = const { std::cell::Cell::new(std::time::Duration::ZERO) };
    /// The main window has its adapter; the screenshot tool's overlay (a second window) gets one of
    /// its own, kept here for the test to draw.
    static OVERLAY: std::cell::RefCell<Option<Rc<MinimalSoftwareWindow>>> = const { std::cell::RefCell::new(None) };
    /// The main window has been given its adapter.
    static HANDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl Platform for Screenshots {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        if !HANDED.replace(true) {
            return Ok(self.window.clone());
        }
        let overlay = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        OVERLAY.with(|slot| *slot.borrow_mut() = Some(overlay.clone()));
        Ok(overlay)
    }

    fn duration_since_start(&self) -> std::time::Duration {
        CLOCK.get()
    }
}

/// Draw the window as it is now into `name`.png. Without a native menu bar, Slint draws the menu
/// bar at the top of the window: the window is that much taller, and the picture leaves it out.
fn save(window: &MinimalSoftwareWindow, name: &str) {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("screenshots");
    std::fs::create_dir_all(&directory).expect("target/screenshots");
    let mut pixels = vec![slint::Rgb8Pixel::default(); (WIDTH * (HEIGHT + MENU_BAR)) as usize];
    CLOCK.set(CLOCK.get() + std::time::Duration::from_secs(1));
    slint::platform::update_timers_and_animations();
    // Twice: text that wraps knows its height only once it has been laid out.
    for _ in 0..2 {
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, WIDTH as usize);
        });
    }
    let bytes: Vec<u8> = pixels[(WIDTH * MENU_BAR) as usize..].iter().flat_map(|pixel| [pixel.r, pixel.g, pixel.b]).collect();
    image::save_buffer(directory.join(format!("{name}.png")), &bytes, WIDTH, HEIGHT, image::ColorType::Rgb8)
        .expect("write the screenshot");
}

/// Draw the screenshot tool's overlay, which has no menu bar, into `name`.png.
fn save_overlay(window: &MinimalSoftwareWindow, name: &str) {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("screenshots");
    std::fs::create_dir_all(&directory).expect("target/screenshots");
    let mut pixels = vec![slint::Rgb8Pixel::default(); (WIDTH * HEIGHT) as usize];
    CLOCK.set(CLOCK.get() + std::time::Duration::from_secs(1));
    slint::platform::update_timers_and_animations();
    for _ in 0..2 {
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, WIDTH as usize);
        });
    }
    let bytes: Vec<u8> = pixels.iter().flat_map(|pixel| [pixel.r, pixel.g, pixel.b]).collect();
    image::save_buffer(directory.join(format!("{name}.png")), &bytes, WIDTH, HEIGHT, image::ColorType::Rgb8)
        .expect("write the screenshot");
}

/// A right click at `x`, `y` in the picture, where the pointer then stays.
fn right_click(window: &MinimalSoftwareWindow, x: f32, y: f32) {
    click_with(window, x, y, PointerEventButton::Right);
}

/// A click at `x`, `y` in the picture, where the pointer then stays.
fn click(window: &MinimalSoftwareWindow, x: f32, y: f32) {
    click_with(window, x, y, PointerEventButton::Left);
}

fn click_with(window: &MinimalSoftwareWindow, x: f32, y: f32, button: PointerEventButton) {
    let position = slint::LogicalPosition::new(x, y + MENU_BAR as f32);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

/// How far under a chat menu's top its fourth entry, Add to folder, is: the menu's padding and
/// heading, then three entries and a half.
fn add_to_folder(theme: Theme) -> f32 {
    match theme {
        Theme::Workbench => 32.0 + 3.5 * 32.0,
        Theme::Broadsheet => 32.0 + 3.5 * 34.0,
        Theme::Terminal => 28.0 + 3.5 * 26.0,
    }
}

/// Escape (closing a menu), and the pointer out of the window, so that nothing stays hovered.
fn escape(window: &MinimalSoftwareWindow) {
    window.dispatch_event(WindowEvent::KeyPressed { text: Key::Escape.into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: Key::Escape.into() });
    window.dispatch_event(WindowEvent::PointerExited);
}

fn model<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(rows))
}

fn country(code: &str, name: &str, calling_code: &str) -> Country {
    Country { code: code.into(), name: name.into(), calling_code: calling_code.into() }
}

fn languages() -> ModelRc<Language> {
    model(
        crate::i18n::LANGUAGES
            .iter()
            .map(|language| Language {
                code: language.code.into(),
                native: language.native.into(),
                english: language.english.into(),
            })
            .collect(),
    )
}

/// The made-up account: someone in a mechanical keyboard club, as in the design.
fn fill(ui: &MainWindow) {
    let app = ui.global::<AppState>();
    app.set_languages(languages());
    app.set_connection(Connection::Ready);
    app.set_app_version(crate::update::CURRENT_VERSION.into());
    app.set_tdlib_version(crate::telegram::TDLIB_VERSION.into());
    // Settings → General as on a Mac: launching at login on, the rest as it comes.
    app.set_launch_at_login_available(true);
    app.set_launch_at_login(true);
    app.set_menu_bar_available(true);
    app.set_keeps_running_available(true);
    // Settings → Notifications & sounds as in the design: channels switched off for the account.
    let scopes = ui.global::<NotificationScopes>();
    scopes.set_loaded(true);
    scopes.set_channels(false);

    let login = ui.global::<Login>();
    login.set_country(country("CN", "China", "86"));
    login.set_countries(model(vec![country("CN", "China", "86"), country("DE", "Germany", "49")]));
    login.set_phone("138 0013 2046".into());
    login.set_code_phone("+86 138 0013 2046".into());
    login.set_delivery(CodeDelivery::Telegram);
    login.set_code_length(5);
    login.set_can_resend(true);
    login.set_password_hint("bird".into());
    login.set_has_recovery_email(true);
    login.set_recovery_email("z**@gmail.com".into());
    login.set_reset_date(reset_date());
    login.on_initial(|name| name.chars().next().map(|first| first.to_uppercase().collect::<String>()).unwrap_or_default().into());

    let password = ui.global::<PasswordSettings>();
    password.set_loaded(true);
    password.set_has_password(true);
    password.set_hint("bird".into());
    password.set_has_recovery_email(true);
    password.set_recovery_email("z•••@gmail.com".into());
    password.set_reset_date(reset_date());
}

/// When a reset asked for today can be completed, as in the design.
fn reset_date() -> Moment {
    Moment { day: Day::Earlier, hour: 14, minute: 20, weekday: 3, month: 10, date: 7, year: 2026 }
}

fn moment(day: Day, hour: i32, minute: i32) -> Moment {
    Moment { day, hour, minute, weekday: 5, month: 9, date: 20, year: 2026 }
}

#[allow(clippy::too_many_arguments)]
fn chat(id: &str, title: &str, kind: ChatKind, time: Moment, sender: &str, text: &str, unread: i32, members: i32) -> ChatRow {
    ChatRow {
        id: id.into(),
        title: title.into(),
        initial: title.chars().next().map(String::from).unwrap_or_default().into(),
        picture: Image::default(),
        has_picture: false,
        kind,
        has_message: true,
        time,
        sender: sender.into(),
        outgoing: false,
        content: Content::Text,
        text: text.into(),
        detail: SharedString::new(),
        unread,
        mention: id == "keyboards",
        muted: id == "news",
        pinned: false,
        online: id == "linxia",
        verified: false,
        members,
        username: SharedString::new(),
        blocked: false,
        reportable: kind != ChatKind::Saved,
        deletable: kind != ChatKind::Saved,
    }
}

/// The chat with its photo: a made-up one around `hue`.
fn with_portrait(chat: ChatRow, hue: f32) -> ChatRow {
    ChatRow { picture: portrait(hue), has_picture: true, ..chat }
}

/// A made-up photo of a chat or a person: a square gradient around `hue`, 160 pixels as TDLib's
/// small chat photos are.
fn portrait(hue: f32) -> Image {
    gradient(160, 160, hue)
}

fn message(id: &str, sender: &str, time: &str, text: &str) -> MessageRow {
    // Zhou Ye (the account) and Jie have photos; the others show their letters.
    let picture = match sender {
        "Zhou Ye" => Some(portrait(330.0)),
        "Jie" => Some(portrait(80.0)),
        _ => None,
    };
    MessageRow {
        kind: RowKind::Message,
        id: id.into(),
        outgoing: sender == "Zhou Ye",
        sender: sender.into(),
        sender_initial: sender.chars().next().map(String::from).unwrap_or_default().into(),
        has_sender_picture: picture.is_some(),
        sender_picture: picture.unwrap_or_default(),
        sender_color: (sender.len() % 8) as i32,
        content: Content::Text,
        text: text.into(),
        detail: SharedString::new(),
        time: time.into(),
        day: moment(Day::Today, 0, 0),
        edited: false,
        sending: false,
        failed: false,
        seen: true,
        button: SharedString::new(),
        ..MessageRow::default()
    }
}

/// A made-up picture: a diagonal gradient around `hue`, as the design draws its placeholders.
fn gradient(width: u32, height: u32, hue: f32) -> Image {
    fn colour(hue: f32, lightness: f32) -> [f32; 3] {
        // A soft colour of that hue: HSL with a little saturation.
        let hue = hue.rem_euclid(360.0) / 60.0;
        let chroma = 0.35 * (1.0 - (2.0 * lightness - 1.0).abs());
        let second = chroma * (1.0 - (hue % 2.0 - 1.0).abs());
        let (r, g, b) = match hue as u32 {
            0 => (chroma, second, 0.0),
            1 => (second, chroma, 0.0),
            2 => (0.0, chroma, second),
            3 => (0.0, second, chroma),
            4 => (second, 0.0, chroma),
            _ => (chroma, 0.0, second),
        };
        let m = lightness - chroma / 2.0;
        [r + m, g + m, b + m]
    }
    let (light, middle, dark) = (colour(hue, 0.80), colour(hue + 50.0, 0.62), colour(hue + 90.0, 0.44));
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    let pixels = buffer.make_mut_slice();
    for y in 0..height {
        for x in 0..width {
            let t = (x as f32 / width as f32) * 0.6 + (y as f32 / height as f32) * 0.4;
            let (from, to, part) = if t < 0.55 { (light, middle, t / 0.55) } else { (middle, dark, (t - 0.55) / 0.45) };
            let channel = |i: usize| ((from[i] + (to[i] - from[i]) * part) * 255.0) as u8;
            pixels[(y * width + x) as usize] = Rgba8Pixel { r: channel(0), g: channel(1), b: channel(2), a: 255 };
        }
    }
    Image::from_rgba8(buffer)
}

fn media(id: &str, width: i32, height: i32, hue: f32, video: bool) -> Media {
    Media {
        id: id.into(),
        picture: gradient(width as u32, height as u32, hue),
        width,
        height,
        video,
        duration: if video { 42 } else { 0 },
        name: if video { "typing-sound-test.mp4".into() } else { SharedString::new() },
        secret: false,
        uploading: false,
        progress: 0.0,
        size: SharedString::new(),
        moved: SharedString::new(),
    }
}

/// A made-up file in a message, as the design's card.
fn file_card(id: &str, name: &str, size: &str, kind: FileType, state: FileState, progress: f32, moved: &str) -> FileCard {
    FileCard { id: id.into(), name: name.into(), size: size.into(), file_type: kind, state, progress, moved: moved.into() }
}

/// A made-up file in the card before sending.
fn attachment(id: i32, name: &str, size: &str, kind: AttachKind, hue: Option<f32>, problem: AttachProblem) -> Attachment {
    Attachment {
        id,
        kind,
        media: hue.is_some(),
        name: name.into(),
        size: size.into(),
        picture: hue.map(|hue| gradient(400, 300, hue)).unwrap_or_default(),
        has_picture: hue.is_some(),
        width: 400,
        height: 300,
        duration: if kind == AttachKind::Video { 42 } else { 0 },
        file_type: match name.rsplit_once('.').map(|(_, extension)| extension) {
            Some("pdf") => FileType::Pdf,
            Some("zip") => FileType::Archive,
            Some("jpg" | "png") => FileType::Image,
            _ => FileType::Other,
        },
        problem,
    }
}

/// A made-up screen for the screenshot tool to freeze: a wallpaper, a light window with grey lines
/// of text, a dark one with coloured lines (an editor), as the design shows it.
fn desktop() -> Image {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(WIDTH, HEIGHT);
    let pixels = buffer.make_mut_slice();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let (t, u) = (x as f32 / WIDTH as f32, y as f32 / HEIGHT as f32);
            pixels[(y * WIDTH + x) as usize] =
                Rgba8Pixel { r: (46.0 + 70.0 * t) as u8, g: (96.0 + 50.0 * u) as u8, b: (150.0 + 70.0 * (1.0 - t)) as u8, a: 255 };
        }
    }
    let mut fill = |x: u32, y: u32, w: u32, h: u32, [r, g, b]: [u8; 3]| {
        for row in y..(y + h).min(HEIGHT) {
            for column in x..(x + w).min(WIDTH) {
                pixels[(row * WIDTH + column) as usize] = Rgba8Pixel { r, g, b, a: 255 };
            }
        }
    };
    fill(60, 80, 560, 420, [255, 255, 255]);
    fill(60, 80, 560, 34, [243, 244, 246]);
    for (i, width) in [420, 380, 440, 300, 460, 340, 400].into_iter().enumerate() {
        fill(84, 134 + i as u32 * 40, width, 10, [214, 216, 220]);
    }
    fill(660, 120, 560, 420, [17, 18, 20]);
    fill(660, 120, 560, 34, [30, 33, 38]);
    let colours = [[122, 162, 247], [158, 206, 106], [224, 175, 104], [187, 154, 247], [86, 95, 137], [192, 202, 245], [158, 206, 106]];
    for (i, (width, colour)) in [360, 420, 300, 460, 320, 400, 280].into_iter().zip(colours).enumerate() {
        fill(684 + (i as u32 % 3) * 14, 174 + i as u32 * 40, width, 10, colour);
    }
    Image::from_rgba8(buffer)
}

/// A made-up sticker: a round face cut out with a white edge, on nothing, as stickers are.
fn sticker() -> Sticker {
    const SIDE: u32 = 256;
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(SIDE, SIDE);
    let pixels = buffer.make_mut_slice();
    let centre = SIDE as f32 / 2.0;
    for y in 0..SIDE {
        for x in 0..SIDE {
            let (dx, dy) = (x as f32 - centre, y as f32 - centre);
            let distance = (dx * dx + dy * dy).sqrt();
            let eye = |eye_x: f32| (x as f32 - eye_x).hypot(y as f32 - 104.0) < 14.0;
            let smile = (distance - 70.0).abs() < 7.0 && dy > 30.0;
            pixels[(y * SIDE + x) as usize] = if distance > 120.0 {
                Rgba8Pixel { r: 0, g: 0, b: 0, a: 0 }
            } else if distance > 110.0 {
                Rgba8Pixel { r: 255, g: 255, b: 255, a: 255 }
            } else if eye(96.0) || eye(160.0) || smile {
                Rgba8Pixel { r: 74, g: 52, b: 32, a: 255 }
            } else {
                let t = y as f32 / SIDE as f32;
                Rgba8Pixel { r: 255, g: (214.0 - 50.0 * t) as u8, b: (92.0 - 40.0 * t) as u8, a: 255 }
            };
        }
    }
    Sticker { picture: Image::from_rgba8(buffer), width: 512, height: 512 }
}

/// The album Jie sent: two photos and a video, as in the design.
fn album() -> Vec<Media> {
    vec![media("6", 400, 300, 25.0, false), media("6a", 300, 400, 200.0, false), media("6b", 640, 360, 290.0, true)]
}

/// The media viewer open on the album, and a wide video after it, at `index`.
fn open_viewer(ui: &MainWindow, index: i32) {
    let viewer = ui.global::<Viewer>();
    let item = |media: Media| ViewerItem {
        id: media.id,
        picture: media.picture,
        width: media.width,
        height: media.height,
        video: media.video,
        duration: media.duration,
        name: media.name,
        sender: "Jie".into(),
        sender_initial: "J".into(),
        sender_picture: portrait(80.0),
        has_sender_picture: true,
        time: moment(Day::Today, 14, 19),
        caption: "The lubed switch comparison is in the group album, have a look.".into(),
        secret: false,
    };
    let wide = Media { id: "8".into(), duration: 95, name: "screen-recording.mp4".into(), ..media("8", 1280, 544, 200.0, true) };
    // The wide video has a long caption, as a channel's posts do: the viewer shows a few lines of
    // it and keeps the room for the picture.
    let mut items: Vec<ViewerItem> = album().into_iter().map(item).collect();
    items.push(ViewerItem {
        caption: "Switch comparison, part two: the same five linear switches, lubed with Krytox 205g0 this time. \
            Recorded on the plate with the case open, then closed, so you can hear what the foam does. \
            Timestamps for each switch are in the pinned message, and the spreadsheet with the force \
            curves is in the files tab of the group."
            .into(),
        ..item(wide)
    });
    viewer.set_items(model(items));
    viewer.set_can_save(true);
    viewer.set_index(index);
    viewer.set_zoom(0);
    viewer.set_playing(false);
    viewer.set_position(0.0);
    viewer.set_open(true);
}

/// A reply's quote of message `id`.
fn quote(id: &str, sender: &str, text: &str) -> ReplyQuote {
    ReplyQuote { shown: true, id: id.into(), sender: sender.into(), content: Content::Text, text: text.into(), ..ReplyQuote::default() }
}

fn day_row(day: Day) -> MessageRow {
    MessageRow { kind: RowKind::Day, day: moment(day, 0, 0), ..message("", "", "", "") }
}

/// The chats of the design's first round, in English.
fn fill_chats(ui: &MainWindow) {
    let account = ui.global::<Account>();
    account.set_name("Zhou Ye".into());
    account.set_first_name("Zhou".into());
    account.set_last_name("Ye".into());
    account.set_initial("Z".into());
    account.set_picture(portrait(330.0));
    account.set_large_picture(portrait(330.0));
    account.set_has_picture(true);
    account.set_username("zhouye".into());
    account.set_phone("+86 138 0013 2046".into());
    account.set_bio("Design, and a little code. Shanghai.".into());

    // Some chats have photos, the others their letters, as in any account.
    let keyboards = ChatRow {
        pinned: true,
        ..chat("keyboards", "Keyboard Lab", ChatKind::Group, moment(Day::Today, 14, 20), "Jie", "@Zhou Ye do you still sell the dark keycaps?", 42, 486)
    };
    let alex = with_portrait(chat("alex", "Alex Chen", ChatKind::User, moment(Day::Today, 12, 8), "", "Can we move the call to Thursday?", 1, 0), 120.0);
    let linxia = with_portrait(chat("linxia", "Lin Xia", ChatKind::User, moment(Day::Today, 13, 52), "", "Is that place open tonight?", 2, 0), 200.0);
    let news = with_portrait(chat("news", "Tech Morning", ChatKind::Channel, moment(Day::Today, 7, 30), "", "New chip export rules take effect", 12, 82413), 25.0);
    let mom = chat("mom", "Mom", ChatKind::User, moment(Day::Yesterday, 18, 32), "", "Never mind, if you are busy don't reply", 0, 0);
    let finch = with_portrait(chat("finch", "FinchGram Updates", ChatKind::Channel, moment(Day::Yesterday, 20, 0), "", "FinchGram 0.1: the design arrives", 0, 41000), 260.0);
    let books = chat("books", "Wednesday Book Club", ChatKind::Group, moment(Day::ThisWeek, 20, 15), "Chen", "Next up: Invisible Cities", 0, 9);
    let saved = ChatRow { kind: ChatKind::Saved, ..chat("saved", "Saved Messages", ChatKind::Saved, moment(Day::ThisYear, 22, 42), "", "Kyoto, check-in October 3", 0, 0) };
    let personal = vec![keyboards.clone(), linxia.clone(), mom.clone(), books.clone(), saved.clone()];
    let work = vec![alex.clone()];
    let channels = vec![news.clone(), finch.clone()];
    let all = vec![keyboards.clone(), linxia.clone(), alex.clone(), news.clone(), mom.clone(), finch.clone(), books.clone(), saved.clone()];

    let chats = ui.global::<Chats>();
    let folder = |id: i32, name: &str, unread: i32| Folder { id, name: name.into(), unread };
    chats.set_folders(model(vec![folder(0, "All chats", 57), folder(1, "Personal", 44), folder(2, "Work", 1), folder(3, "Channels", 12)]));
    // A chat's menu, Add to folder: Keyboard Lab, which is in Personal.
    let choice = |folder: i32, name: &str, inside: bool| FolderChoice { folder, name: name.into(), inside };
    chats.set_menu_folders(model(vec![choice(1, "Personal", true), choice(2, "Work", false), choice(3, "Channels", false)]));
    let header = |folder: i32, count: usize, expanded: bool| TreeRow { header: true, folder, expanded, count: count as i32, chat: ChatRow::default() };
    let line = |folder: i32, chat: &ChatRow| TreeRow { header: false, folder, expanded: true, count: 0, chat: chat.clone() };
    // The pinned chats at the top, in no folder, and not again below.
    let mut tree: Vec<TreeRow> = all.iter().filter(|chat| chat.pinned).map(|chat| line(-1, chat)).collect();
    for (index, folder_chats) in [(1, &personal), (2, &work), (3, &channels), (0, &all)] {
        let unpinned: Vec<&ChatRow> = folder_chats.iter().filter(|chat| !chat.pinned).collect();
        tree.push(header(index, unpinned.len(), index > 0));
        if index > 0 {
            tree.extend(unpinned.into_iter().map(|chat| line(index, chat)));
        }
    }
    chats.set_tree(model(tree));
    chats.set_list(model(all));
    chats.set_channels(model(channels));
    chats.set_unread_channels(1);
    let bot = |id: &str, title: &str, username: &str, verified: bool| ChatRow {
        verified,
        username: username.into(),
        ..chat(id, title, ChatKind::Bot, moment(Day::ThisWeek, 13, 2), "", "", 0, 0)
    };
    chats.set_official_bots(model(vec![bot("botfather", "BotFather", "BotFather", true), bot("stickers", "Stickers", "Stickers", true)]));
    chats.set_bots(model(vec![with_portrait(bot("groupbuy", "Group-buy Helper", "keeb_gb_bot", false), 45.0)]));
    chats.set_loaded(true);

    let conversation = ui.global::<Conversation>();
    let tab = |chat: &ChatRow| Tab { id: chat.id.clone(), title: chat.title.clone(), kind: chat.kind, muted: chat.muted };
    conversation.set_tabs(model(vec![tab(&keyboards), tab(&news), tab(&alex)]));
}

fn open_keyboards(ui: &MainWindow) {
    let conversation = ui.global::<Conversation>();
    conversation.set_chat_id("keyboards".into());
    conversation.set_title("Keyboard Lab".into());
    conversation.set_initial("K".into());
    conversation.set_kind(ChatKind::Group);
    conversation.set_status(Status::Members);
    conversation.set_members(486);
    conversation.set_online_members(32);
    conversation.set_can_write(true);
    conversation.set_can_send_photos(true);
    conversation.set_can_send_videos(true);
    conversation.set_can_send_files(true);
    conversation.set_messages(model(vec![
        day_row(Day::Today),
        // Formatting and a link.
        MessageRow {
            rich: true,
            rich_text: slint::StyledText::from_markdown(
                "Morning! This week's **group-buy keycaps** arrived; the [delivery list](https://geekhack.org/topic/12045) is in the pinned post.",
            )
            .expect("Markdown"),
            ..message("1", "Mi", "09:12", "Morning! This week's group-buy keycaps arrived; the delivery list is in the pinned post.")
        },
        message("2", "Jie", "10:30", "I'm lubing my 65% today, photos later."),
        MessageRow { reply: quote("2", "Jie", "I'm lubing my 65% today, photos later."), ..message("3", "Zhou Ye", "10:31", "Nice, @ me when it's done.") },
        MessageRow { content: Content::Sticker, detail: "👍".into(), sticker: sticker(), ..message("3a", "Zhou Ye", "10:31", "") },
        message("4", "Mika", "11:02", "I can make the meetup on Thursday, but only after 3pm."),
        message("5", "Mi", "11:05", "Thursday 15:00 at the usual place. Who's coming?"),
        MessageRow {
            forwarded_from: "Keyboard Lab Notices".into(),
            ..message("5a", "Mi", "11:40", "October group buy: dark PBT keycaps, orders close October 8.")
        },
        MessageRow {
            reply: ReplyQuote { shown: true, gone: true, ..ReplyQuote::default() },
            ..message("5b", "Mika", "14:05", "Saw it before it was deleted, looks great.")
        },
        MessageRow {
            content: Content::Photo,
            media: model(album()),
            ..message("6", "Jie", "14:19", "The lubed switch comparison is in the group album, have a look.")
        },
        // A photo sent to be seen once: its blurred preview until it is opened, then gone.
        MessageRow {
            content: Content::Photo,
            media: model(vec![Media { secret: true, picture: gradient(12, 9, 120.0), ..media("6c", 400, 300, 120.0, false) }]),
            ..message("6c", "Mika", "14:21", "")
        },
        MessageRow { content: Content::ExpiredPhoto, ..message("6d", "Jie", "14:22", "") },
        // A mention, a link and its preview.
        MessageRow {
            rich: true,
            rich_text: slint::StyledText::from_markdown(
                "[@Zhou Ye](tg://resolve?domain=zhouye) do you still sell the dark keycaps? The thread: [geekhack.org/topic/12045](https://geekhack.org/topic/12045)",
            )
            .expect("Markdown"),
            preview: LinkPreview {
                url: "https://geekhack.org/topic/12045".into(),
                site: "geekhack".into(),
                title: "[GB] Dark keycaps: the delivery list".into(),
                about: "Posted by Mi · 42 replies".into(),
                instant_view: false,
            },
            ..message("7", "Jie", "14:20", "@Zhou Ye do you still sell the dark keycaps?")
        },
    ]));
}

fn open_news(ui: &MainWindow) {
    let conversation = ui.global::<Conversation>();
    conversation.set_chat_id("news".into());
    conversation.set_title("Tech Morning".into());
    conversation.set_initial("T".into());
    conversation.set_kind(ChatKind::Channel);
    conversation.set_status(Status::Subscribers);
    conversation.set_members(82413);
    conversation.set_can_write(false);
    let post = |id: &str, time: &str, text: &str| MessageRow { sender: "Tech Morning".into(), ..message(id, "Tech Morning", time, text) };
    conversation.set_messages(model(vec![
        day_row(Day::Today),
        // The design's post with an article in Instant View.
        MessageRow {
            preview: LinkPreview {
                url: "https://techmorning.example/2026/09/28/chip-rules".into(),
                site: "Tech Morning".into(),
                title: "The new chip export rules in full: which equipment is affected".into(),
                about: "Tech Morning desk · Sep 28 · 4 min read".into(),
                instant_view: true,
            },
            ..post("1", "07:00", "New chip export rules take effect\n\nApprovals for advanced equipment are now split into three tiers by type and end use, with a clear upper limit on how long a review may take. Most orders already in transit are unaffected.")
        },
        MessageRow {
            kind: RowKind::Sponsored,
            sender: "Keyboard Autumn Launch".into(),
            text: "Telegram's ad slot; it has nothing to do with this channel.".into(),
            button: "Learn more".into(),
            ..message("s1", "", "07:15", "")
        },
        MessageRow {
            content: Content::Photo,
            media: model(vec![media("2", 1280, 720, 150.0, false)]),
            ..post("2", "07:30", "Four flagship phones launch this week; cameras and on-device features lead, prices hold.")
        },
    ]));
}

/// What can be done with a message (the design's fourth round): its menu (someone else's message,
/// ours, a photo), replying, editing, forwarding, reporting, choosing, deleting, and a notice.
fn message_actions(ui: &MainWindow, window: &MinimalSoftwareWindow, name: &dyn Fn(&str) -> String) {
    use MessageAction as A;
    let actions = ui.global::<Actions>();
    let conversation = ui.global::<Conversation>();
    let menu = |row: &str, items: Vec<MessageAction>, head: &str| {
        actions.set_menu_row(row.into());
        actions.set_menu_items(model(items));
        actions.set_menu_head(head.into());
        actions.set_menu_x(640.0);
        actions.set_menu_y(430.0);
        actions.set_menu_open(true);
    };
    menu("7", vec![A::Reply, A::Copy, A::CopyLink, A::Forward, A::Report, A::Select], "Jie · 14:20");
    save(window, &name("message-menu"));
    menu("3", vec![A::Reply, A::Edit, A::Copy, A::CopyLink, A::Forward, A::Delete, A::Select], "Zhou Ye · 10:31");
    save(window, &name("message-menu-own"));
    menu("6", vec![A::Reply, A::CopyImage, A::SaveImage, A::CopyText, A::CopyLink, A::Forward, A::Report, A::Select], "Jie · 14:19");
    save(window, &name("message-menu-photo"));
    actions.set_menu_open(false);
    actions.set_menu_row(SharedString::new());

    let bar = |kind: ComposeBar, name: &str, text: &str, picture: Option<Image>| {
        actions.set_bar_name(name.into());
        actions.set_bar_count(1);
        actions.set_bar_content(if picture.is_some() { Content::Photo } else { Content::Text });
        actions.set_bar_text(text.into());
        actions.set_bar_detail(SharedString::new());
        actions.set_bar_has_picture(picture.is_some());
        actions.set_bar_picture(picture.unwrap_or_default());
        actions.set_bar(kind);
    };
    bar(ComposeBar::Reply, "Jie", "@Zhou Ye do you still sell the dark keycaps?", None);
    save(window, &name("reply"));
    bar(ComposeBar::Edit, "Zhou Ye", "Nice, @ me when it's done.", None);
    conversation.set_draft("Nice, @ me when it's done, I'll bring the 75%.".into());
    save(window, &name("edit"));
    conversation.set_draft(SharedString::new());

    actions.set_picker_saved(true);
    actions.set_picker_chats(model(vec![
        chat("keyboards", "Keyboard Lab", ChatKind::Group, moment(Day::Today, 14, 20), "", "", 0, 486),
        with_portrait(chat("linxia", "Lin Xia", ChatKind::User, moment(Day::Today, 13, 52), "", "", 0, 0), 200.0),
        chat("mika", "Mika", ChatKind::User, moment(Day::Today, 11, 3), "", "", 0, 0),
        chat("books", "Wednesday Book Club", ChatKind::Group, moment(Day::ThisWeek, 20, 15), "", "", 0, 12),
        chat("mom", "Mom", ChatKind::User, moment(Day::Yesterday, 18, 32), "", "", 0, 0),
        with_portrait(chat("alex", "Alex Chen", ChatKind::User, moment(Day::Today, 12, 8), "", "", 0, 0), 120.0),
    ]));
    actions.set_bar(ComposeBar::None);
    actions.set_picker_open(true);
    save(window, &name("forward-picker"));
    actions.set_picker_open(false);
    bar(ComposeBar::Forward, "Jie", "", Some(gradient(80, 60, 25.0)));
    actions.set_bar_count(3);
    save(window, &name("forward"));
    actions.set_bar(ComposeBar::None);

    let reasons = ["I don't like it", "Child abuse", "Violence", "Illegal goods", "Personal data", "Scam or spam", "Copyright", "Other"];
    actions.set_report_first(true);
    actions.set_report_title("Report".into());
    actions.set_report_options(model(reasons.iter().map(|reason| SharedString::from(*reason)).collect()));
    actions.set_report_comment(false);
    actions.set_report_open(true);
    save(window, &name("report"));
    actions.set_report_first(false);
    actions.set_report_title("Copyright".into());
    actions.set_report_comment(true);
    actions.set_report_comment_required(true);
    save(window, &name("report-comment"));
    actions.set_report_open(false);

    // Choosing: three messages, the album among them.
    let messages = conversation.get_messages();
    let chosen = ["4", "5", "6"];
    let marked = |on: bool| {
        for index in 0..messages.row_count() {
            if let Some(row) = messages.row_data(index) {
                let selected = on && chosen.contains(&row.id.as_str());
                messages.set_row_data(index, MessageRow { selected, ..row });
            }
        }
    };
    marked(true);
    actions.set_selected_count(5);
    actions.set_can_forward(true);
    actions.set_can_copy(true);
    actions.set_can_delete(false);
    actions.set_can_report(true);
    actions.set_report_offered(true);
    actions.set_selecting(true);
    save(window, &name("select"));
    actions.set_selecting(false);
    marked(false);

    actions.set_delete_count(1);
    actions.set_delete_choice(DeleteChoice::ForEveryone);
    actions.set_revoke(true);
    actions.set_delete_open(true);
    save(window, &name("delete"));
    actions.set_delete_open(false);

    actions.set_flash("3".into());
    actions.set_notice(ActionNotice::LinkCopied);
    save(window, &name("notice"));
    actions.set_notice(ActionNotice::None);
    actions.set_flash(SharedString::new());
}

/// The screenshot tool (the design's seventh round): in the window, the card when the system has
/// not let FinchGram see the screen, the send card with a screenshot and the notice after saving
/// one; on the overlay, each step from the window under the pointer to the annotations.
fn screenshot_tool(ui: &MainWindow, window: &MinimalSoftwareWindow, overlay: &ShotWindow, overlay_window: &MinimalSoftwareWindow, name: &dyn Fn(&str) -> String) {
    let screenshot = ui.global::<Screenshot>();
    let attachments = ui.global::<Attachments>();
    let actions = ui.global::<Actions>();
    screenshot.set_permission_card(true);
    save(window, &name("shot-permission"));
    screenshot.set_permission_card(false);

    attachments.set_items(model(vec![attachment(0, "Screenshot 2026-10-07 at 11.02.15.png", "412 KB", AttachKind::Photo, Some(200.0), AttachProblem::None)]));
    attachments.set_count(1);
    attachments.set_videos(0);
    attachments.set_all_media(true);
    attachments.set_as_file(false);
    attachments.set_screenshot(true);
    attachments.set_timer_allowed(true);
    attachments.set_caption_max(1024);
    attachments.set_hint(AttachHint::Keys);
    attachments.set_can_send(true);
    attachments.set_card_open(true);
    save(window, &name("shot-send-card"));
    attachments.set_card_open(false);
    attachments.set_screenshot(false);
    attachments.set_items(model(Vec::new()));
    attachments.set_hint(AttachHint::None);

    actions.set_saved_name("Screenshot 2026-10-07 at 11.02.15.png".into());
    actions.set_notice(ActionNotice::Saved);
    save(window, &name("shot-saved"));
    actions.set_notice(ActionNotice::None);
    actions.set_saved_name(SharedString::new());

    // The overlay: the same theme and appearance, in its own window.
    let app = ui.global::<AppState>();
    overlay.global::<AppState>().set_theme(app.get_theme());
    overlay.global::<AppState>().set_appearance(app.get_appearance());
    let shot = overlay.global::<Shot>();
    shot.set_scale(1.0);
    shot.set_has_selection(false);
    shot.set_settled(false);
    shot.set_adjusting(false);
    shot.set_show_magnifier(false);
    shot.set_editing(false);
    shot.set_tool(ShotTool::None);
    shot.set_cursor(ShotCursor::Cross);
    shot.set_boxes(model(Vec::new()));
    shot.set_paths(model(Vec::new()));
    shot.set_texts(model(Vec::new()));
    shot.set_cells(model(Vec::new()));
    // Waiting: the screen dimmed all over, nothing chosen yet.
    shot.set_label(SharedString::new());
    shot.set_pointer_x(300.0);
    shot.set_pointer_y(300.0);
    save_overlay(overlay_window, &name("shot-waiting"));
    // Dragging a selection: the magnifier by the pointer.
    shot.set_has_selection(true);
    shot.set_sel_x(120.0);
    shot.set_sel_y(140.0);
    shot.set_sel_w(300.0);
    shot.set_sel_h(200.0);
    shot.set_adjusting(true);
    shot.set_label("300 × 200".into());
    shot.set_show_magnifier(true);
    shot.set_pointer_x(420.0);
    shot.set_pointer_y(340.0);
    shot.set_mag_pos("x 420  y 340".into());
    shot.set_mag_hex("#D6D8DC".into());
    shot.set_mag_color(slint::Color::from_rgb_u8(0xd6, 0xd8, 0xdc));
    save_overlay(overlay_window, &name("shot-dragging"));
    // Selected: handles, and the toolbar under the selection.
    shot.set_show_magnifier(false);
    shot.set_adjusting(false);
    shot.set_settled(true);
    shot.set_sel_w(440.0);
    shot.set_sel_h(300.0);
    shot.set_label("440 × 300".into());
    shot.set_pointer_x(700.0);
    shot.set_pointer_y(600.0);
    save_overlay(overlay_window, &name("shot-selected"));
    // Annotating with the arrow: its options, and marks of every kind on the picture.
    shot.set_tool(ShotTool::Arrow);
    shot.set_size(1);
    shot.set_color_index(0);
    shot.set_can_undo(true);
    let red = slint::Color::from_rgb_u8(0xf2, 0x35, 0x2b);
    let blue = slint::Color::from_rgb_u8(0x1f, 0x7b, 0xff);
    let yellow = slint::Color::from_rgb_u8(0xff, 0xcc, 0x00);
    let light = slint::Color::from_argb_u8(217, 255, 255, 255);
    let dark = slint::Color::from_argb_u8(153, 0, 0, 0);
    shot.set_boxes(model(vec![
        ShotBox { x: 150.0, y: 170.0, w: 160.0, h: 90.0, color: red, halo: light, width: 4.0, ellipse: false },
        ShotBox { x: 340.0, y: 180.0, w: 110.0, h: 110.0, color: blue, halo: light, width: 4.0, ellipse: true },
    ]));
    shot.set_paths(model(vec![
        ShotPath { commands: "M 170 400 L 332 322".into(), color: red, halo: light, width: 4.0, fill: false },
        ShotPath { commands: "M 350 314 L 332 329 L 326 316 Z".into(), color: red, halo: light, width: 1.0, fill: true },
    ]));
    shot.set_texts(model(vec![ShotText { x: 160.0, y: 290.0, text: "Check this".into(), color: yellow, halo: dark, size: 20.0 }]));
    let greys = [[0xd6, 0xd8, 0xdc], [0xff, 0xff, 0xff], [0xe6, 0xe8, 0xeb], [0xf3, 0xf4, 0xf6]];
    let cells: Vec<ShotCell> = (0..120)
        .map(|i| {
            let [r, g, b] = greys[(i * 7 % 4) as usize];
            ShotCell { x: 400.0 + (i % 12) as f32 * 4.0, y: 336.0 + (i / 12) as f32 * 4.0, size: 4.0, color: slint::Color::from_rgb_u8(r, g, b) }
        })
        .collect();
    shot.set_cells(model(cells));
    save_overlay(overlay_window, &name("shot-annotated"));
    // Writing on the picture: the text tool's box, with its words so far.
    shot.set_tool(ShotTool::Text);
    shot.set_editing(true);
    shot.set_text_x(200.0);
    shot.set_text_y(380.0);
    shot.set_text_size(20.0);
    shot.set_text_color(red);
    shot.set_text_halo(light);
    shot.set_text_draft("Meet here".into());
    save_overlay(overlay_window, &name("shot-text"));
    shot.set_editing(false);
    shot.set_text_draft(SharedString::new());
    // The mosaic: its brush under the pointer, and its own options.
    shot.set_tool(ShotTool::Mosaic);
    shot.set_cursor(ShotCursor::Brush);
    shot.set_brush(24.0);
    shot.set_pointer_x(470.0);
    shot.set_pointer_y(360.0);
    save_overlay(overlay_window, &name("shot-mosaic"));
    shot.set_cursor(ShotCursor::Cross);
    // A selection at the bottom of the screen: the toolbar goes above it.
    shot.set_tool(ShotTool::None);
    shot.set_can_undo(false);
    shot.set_boxes(model(Vec::new()));
    shot.set_paths(model(Vec::new()));
    shot.set_texts(model(Vec::new()));
    shot.set_cells(model(Vec::new()));
    shot.set_sel_x(120.0);
    shot.set_sel_y(560.0);
    shot.set_sel_w(500.0);
    shot.set_sel_h(200.0);
    shot.set_label("500 × 200".into());
    save_overlay(overlay_window, &name("shot-toolbar-above"));
    shot.set_has_selection(false);
    shot.set_settled(false);
}

/// Sending attachments (the design's sixth round): the paperclip's menu, the card with photos,
/// with one photo and a timer, with files (one too big), the drop zone, and the messages' states on
/// their way (uploading, a file received, failed).
fn attachments(ui: &MainWindow, window: &MinimalSoftwareWindow, theme: Theme, name: &dyn Fn(&str) -> String) {
    let attachments = ui.global::<Attachments>();
    let conversation = ui.global::<Conversation>();
    // The paperclip's menu, above the paperclip of each theme's composer.
    let (x, y) = match theme {
        Theme::Workbench => (330.0, 732.0),
        Theme::Broadsheet => (1148.0, 740.0),
        Theme::Terminal => (300.0, 752.0),
    };
    attachments.set_menu_x(x);
    attachments.set_menu_y(y);
    attachments.set_menu_open(true);
    save(window, &name("attach-menu"));
    attachments.set_menu_keyboard(true);
    attachments.set_menu_focus(1);
    save(window, &name("attach-menu-keys"));
    attachments.set_menu_open(false);
    attachments.set_menu_keyboard(false);
    attachments.set_menu_focus(-1);

    let photos = |n: i32| -> Vec<Attachment> {
        (0..n).map(|i| attachment(i, &format!("IMG_{}.jpg", 2041 + i), "2.1 MB", AttachKind::Photo, Some(30.0 + 70.0 * i as f32), AttachProblem::None)).collect()
    };
    attachments.set_caption_max(1024);
    attachments.set_hint(AttachHint::Keys);
    attachments.set_can_send(true);
    attachments.set_grouped(true);
    // Three photos, as an album, in a group (no timer there).
    attachments.set_items(model(photos(3)));
    attachments.set_count(3);
    attachments.set_videos(0);
    attachments.set_all_media(true);
    attachments.set_as_file(false);
    attachments.set_timer_allowed(false);
    attachments.set_card_open(true);
    save(window, &name("send-card"));
    // One photo, in a private chat, to be seen once; the timer's choices open.
    attachments.set_items(model(photos(1)));
    attachments.set_count(1);
    attachments.set_timer_allowed(true);
    attachments.set_timer(-1);
    attachments.set_caption("Meetup photos".into());
    attachments.set_timer_open(true);
    save(window, &name("send-card-timer"));
    attachments.set_timer_open(false);
    attachments.set_timer(0);
    attachments.set_caption(SharedString::new());
    // Two files, one too big: it is marked, and Send waits.
    attachments.set_items(model(vec![
        attachment(0, "October screenings.pdf", "1.2 MB", AttachKind::File, None, AttachProblem::None),
        attachment(1, "club-archive-2025.zip", "2.6 GB", AttachKind::File, None, AttachProblem::TooBig),
    ]));
    attachments.set_count(2);
    attachments.set_all_media(false);
    attachments.set_as_file(true);
    attachments.set_timer_allowed(false);
    attachments.set_hint(AttachHint::RemoveMarked);
    attachments.set_can_send(false);
    save(window, &name("send-card-files"));
    attachments.set_card_open(false);
    attachments.set_items(model(Vec::new()));
    attachments.set_hint(AttachHint::None);

    // Files dragged over the window: pictures.
    attachments.set_dropping(true);
    attachments.set_drop_allowed(true);
    attachments.set_drop_photos(true);
    save(window, &name("drop-zone"));
    attachments.set_dropping(false);

    // Messages on their way: a photo and a file going up, a file received (not downloaded, coming
    // down, on disk), and one that was not sent.
    let before = conversation.get_messages();
    let mut up = media("u1", 400, 300, 25.0, false);
    up.uploading = true;
    up.progress = 0.42;
    up.size = "2.1 MB".into();
    up.moved = "0.9 MB".into();
    conversation.set_messages(model(vec![
        day_row(Day::Today),
        message("1", "Mi", "18:40", "Poster draft for Saturday is up."),
        MessageRow {
            content: Content::Document,
            files: model(vec![file_card("2", "October screenings.pdf", "1.2 MB", FileType::Pdf, FileState::Remote, 0.0, "")]),
            ..message("2", "Jie", "18:44", "Here’s the schedule.")
        },
        MessageRow {
            content: Content::Document,
            files: model(vec![file_card("3", "subtitles-pack.zip", "640 KB", FileType::Archive, FileState::Downloading, 0.6, "384 KB")]),
            ..message("3", "Mika", "18:46", "")
        },
        MessageRow {
            content: Content::Document,
            files: model(vec![file_card("4", "cover-art.png", "3.4 MB", FileType::Image, FileState::Done, 0.0, "")]),
            ..message("4", "Zhou Ye", "18:50", "")
        },
        MessageRow {
            content: Content::Document,
            files: model(vec![file_card("6", "notes.txt", "12 KB", FileType::Other, FileState::Uploading, 0.35, "4 KB")]),
            ..message("6", "Zhou Ye", "18:53", "")
        },
    ]));
    save(window, &name("files"));
    conversation.set_messages(model(vec![
        day_row(Day::Today),
        message("1", "Mi", "18:40", "Poster draft for Saturday is up."),
        MessageRow { content: Content::Photo, media: model(vec![up]), ..message("5", "Zhou Ye", "18:52", "") },
        MessageRow { failed: true, content: Content::Photo, media: model(vec![media("7", 400, 300, 200.0, false)]), ..message("7", "Zhou Ye", "18:54", "") },
    ]));
    save(window, &name("uploading"));
    conversation.set_messages(before);
}

/// Settings → Privacy & security, and two-step verification: on, with a reset on its way and a
/// change just made; off; asking for the password; a new password.
fn two_step(ui: &MainWindow, window: &MinimalSoftwareWindow, name: &dyn Fn(&str) -> String) {
    let app = ui.global::<AppState>();
    let password = ui.global::<PasswordSettings>();
    app.set_settings_section(SettingsSection::Privacy);
    password.set_open(false);
    save(window, &name("settings-privacy"));
    password.set_open(true);
    password.set_step(PasswordStep::Overview);
    password.set_reset_pending(true);
    password.set_done(PasswordChange::Password);
    save(window, &name("settings-two-step"));
    password.set_reset_pending(false);
    password.set_done(PasswordChange::Nothing);
    password.set_has_password(false);
    save(window, &name("settings-two-step-off"));
    password.set_has_password(true);
    password.set_step(PasswordStep::Verify);
    password.set_error("PASSWORD_HASH_INVALID".into());
    save(window, &name("settings-two-step-verify"));
    password.set_error(SharedString::new());
    password.set_step(PasswordStep::NewPassword);
    save(window, &name("settings-two-step-new-password"));
    password.set_step(PasswordStep::Overview);
    password.set_open(false);
}

#[test]
#[ignore = "takes over Slint's platform; run with: cargo test screenshots -- --ignored"]
fn screenshots() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Screenshots { window: window.clone() })).expect("platform");
    crate::fonts::register();
    window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT + MENU_BAR));

    let ui = MainWindow::new().expect("window");
    ui.show().expect("show");
    fill(&ui);
    fill_chats(&ui);
    // The screenshot tool's overlay: a window of its own, over a made-up screen.
    let overlay = ShotWindow::new().expect("overlay window");
    let overlay_window = OVERLAY.with(|slot| slot.borrow().clone()).expect("the overlay's window");
    overlay_window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    overlay.global::<Shot>().set_frame(desktop());
    overlay.show().expect("show the overlay");
    let app = ui.global::<AppState>();
    let login = ui.global::<Login>();

    for (theme, theme_name) in [(Theme::Workbench, "workbench"), (Theme::Broadsheet, "broadsheet"), (Theme::Terminal, "terminal")] {
        app.set_theme(theme);
        for appearance in ["light", "dark"] {
            app.set_appearance(appearance.into());
            let name = |page: &str| format!("{theme_name}-{appearance}-{page}");

            app.set_telegram_state(TelegramState::WaitPhoneNumber);
            login.set_step(LoginStep::Phone);
            save(&window, &name("login-phone"));
            login.set_step(LoginStep::Code);
            save(&window, &name("login-code"));
            login.set_step(LoginStep::Password);
            login.set_error("Incorrect password".into());
            save(&window, &name("login-password"));
            login.set_error(SharedString::new());
            login.set_step(LoginStep::RecoveryCode);
            save(&window, &name("login-recovery-code"));
            login.set_step(LoginStep::NewPassword);
            login.set_error("NEW_PASSWORD_MISMATCH".into());
            save(&window, &name("login-new-password"));
            login.set_error(SharedString::new());
            login.set_step(LoginStep::ResetAccount);
            save(&window, &name("login-reset-account"));
            login.set_step(LoginStep::AccountResetRequested);
            save(&window, &name("login-account-reset-requested"));
            app.set_telegram_state(TelegramState::WaitRegistration);
            login.set_step(LoginStep::Registration);
            save(&window, &name("login-sign-up"));
            app.set_telegram_state(TelegramState::WaitPhoneNumber);
            login.set_step(LoginStep::Qr);
            save(&window, &name("login-qr"));
            app.set_telegram_state(TelegramState::Starting);
            save(&window, &name("login-starting"));

            app.set_telegram_state(TelegramState::Ready);
            app.set_page(Page::Chats);
            open_keyboards(&ui);
            save(&window, &name("chats"));
            // A right click on the first chat of the list: its menu.
            let (x, y) = match theme {
                Theme::Workbench => (150.0, 91.0),
                Theme::Broadsheet => (230.0, 310.0),
                Theme::Terminal => (140.0, 98.0),
            };
            right_click(&window, x, y);
            save(&window, &name("chat-menu"));
            // Its fourth entry, Add to folder: the menu's second page.
            click(&window, x + 40.0, y + add_to_folder(theme));
            save(&window, &name("chat-menu-folders"));
            escape(&window);
            // The menu of a chat with a person: blocking, reporting, deleting.
            let (ux, uy) = match theme {
                Theme::Workbench => (150.0, 149.0),
                Theme::Broadsheet => (230.0, 376.0),
                Theme::Terminal => (140.0, 127.0),
            };
            right_click(&window, ux, uy);
            save(&window, &name("chat-menu-user"));
            escape(&window);
            // The questions asked before leaving a group and before deleting a chat.
            let chats = ui.global::<Chats>();
            chats.set_confirm_title("Keyboard Lab".into());
            chats.set_confirm_kind(ChatKind::Group);
            chats.set_confirm_revoke_choice(false);
            chats.set_question(ChatConfirm::Leave);
            save(&window, &name("chat-leave"));
            chats.set_confirm_title("Lin Xia".into());
            chats.set_confirm_kind(ChatKind::User);
            chats.set_confirm_revoke_choice(true);
            chats.set_confirm_revoke(true);
            chats.set_question(ChatConfirm::Delete);
            save(&window, &name("chat-delete"));
            chats.set_question(ChatConfirm::None);
            // Workbench's tabs: a right click on the second one, Tech Morning.
            if theme == Theme::Workbench {
                right_click(&window, 520.0, 59.0);
                save(&window, &name("tab-menu"));
                escape(&window);
            }
            message_actions(&ui, &window, &name);
            attachments(&ui, &window, theme, &name);
            screenshot_tool(&ui, &window, &overlay, &overlay_window, &name);
            open_viewer(&ui, 1);
            save(&window, &name("viewer-photo"));
            open_viewer(&ui, 2);
            save(&window, &name("viewer-video"));
            open_viewer(&ui, 3);
            save(&window, &name("viewer-wide-video"));
            ui.global::<Viewer>().set_open(false);
            open_news(&ui);
            save(&window, &name("channel"));
            app.set_page(Page::Settings);
            for (section, section_name) in
                [(SettingsSection::General, "general"), (SettingsSection::Notifications, "notifications"), (SettingsSection::Appearance, "appearance"), (SettingsSection::Language, "language"), (SettingsSection::About, "about")]
            {
                app.set_settings_section(section);
                save(&window, &name(&format!("settings-{section_name}")));
            }
            // Settings → About while an update downloads: how far, in per cent and megabytes.
            app.set_update_state(crate::UpdateState::Installing);
            app.set_update_progress(0.42);
            app.set_update_downloaded("26.8 MB".into());
            app.set_update_total("63.8 MB".into());
            save(&window, &name("settings-about-downloading"));
            app.set_update_state(crate::UpdateState::Idle);
            // Settings → General → Screenshots: the shortcut's box taking new keys.
            app.set_settings_section(SettingsSection::General);
            ui.global::<Screenshot>().set_recording(true);
            save(&window, &name("settings-screenshot-shortcut"));
            ui.global::<Screenshot>().set_recording(false);
            two_step(&ui, &window, &name);
            app.set_page(Page::Profile);
            save(&window, &name("profile"));
            app.set_page(Page::Chats);
        }
    }

    // The same in Chinese, light: every theme's chat window, and the shared pages in Workbench.
    slint::select_bundled_translation("zh_Hans").expect("Chinese is bundled");
    app.set_language("zh_Hans".into());
    app.set_appearance("light".into());
    for (theme, theme_name) in [(Theme::Workbench, "workbench"), (Theme::Broadsheet, "broadsheet"), (Theme::Terminal, "terminal")] {
        app.set_theme(theme);
        app.set_telegram_state(TelegramState::Ready);
        app.set_page(Page::Chats);
        open_keyboards(&ui);
        save(&window, &format!("zh-{theme_name}-chats"));
        if theme == Theme::Workbench {
            right_click(&window, 520.0, 59.0);
            save(&window, "zh-workbench-tab-menu");
            escape(&window);
            right_click(&window, 150.0, 91.0);
            click(&window, 190.0, 91.0 + add_to_folder(theme));
            save(&window, "zh-workbench-chat-menu-folders");
            escape(&window);
        }
        message_actions(&ui, &window, &|page: &str| format!("zh-{theme_name}-{page}"));
    }
    app.set_theme(Theme::Workbench);
    app.set_telegram_state(TelegramState::WaitPhoneNumber);
    login.set_step(LoginStep::Phone);
    save(&window, "zh-workbench-login-phone");
    login.set_step(LoginStep::RecoveryCode);
    save(&window, "zh-workbench-login-recovery-code");
    app.set_telegram_state(TelegramState::WaitRegistration);
    login.set_step(LoginStep::Registration);
    save(&window, "zh-workbench-login-sign-up");
    app.set_telegram_state(TelegramState::Ready);
    app.set_page(Page::Settings);
    app.set_settings_section(SettingsSection::General);
    save(&window, "zh-workbench-settings-general");
    app.set_settings_section(SettingsSection::Notifications);
    save(&window, "zh-workbench-settings-notifications");
    app.set_settings_section(SettingsSection::Appearance);
    save(&window, "zh-workbench-settings-appearance");
    let password = ui.global::<PasswordSettings>();
    app.set_settings_section(SettingsSection::Privacy);
    save(&window, "zh-workbench-settings-privacy");
    password.set_open(true);
    password.set_reset_pending(true);
    password.set_done(PasswordChange::Password);
    save(&window, "zh-workbench-settings-two-step");
    password.set_open(false);
    app.set_page(Page::Chats);
    let _ = login.get_countries().row_count();
}
