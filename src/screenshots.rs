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

/// How far under a chat menu's top its third entry, Add to folder, is: the menu's padding and
/// heading, then two entries and a half.
fn add_to_folder(theme: Theme) -> f32 {
    match theme {
        Theme::Workbench => 32.0 + 2.5 * 32.0,
        Theme::Broadsheet => 32.0 + 2.5 * 34.0,
        Theme::Terminal => 28.0 + 2.5 * 26.0,
    }
}

/// The wheel turned at `x`, `y`: `down` logical pixels further down the page.
fn scroll(window: &MinimalSoftwareWindow, x: f32, y: f32, down: f32) {
    let position = slint::LogicalPosition::new(x, y + MENU_BAR as f32);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerScrolled { position, delta_x: 0.0, delta_y: -down });
    window.dispatch_event(WindowEvent::PointerExited);
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
        draft: SharedString::new(),
        marked: false,
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
    viewer.set_volume(100);
    viewer.set_muted(false);
    viewer.set_volume_shown(false);
    viewer.set_playing(false);
    viewer.set_position(0.0);
    viewer.set_open(true);
}

/// A reply's quote of message `id`.
fn quote(id: &str, sender: &str, text: &str) -> ReplyQuote {
    ReplyQuote { shown: true, id: id.into(), sender: sender.into(), content: Content::Text, text: text.into(), ..ReplyQuote::default() }
}

/// The management page as src/telegram/manage.rs fills it for the keyboard group: Zhou Ye an
/// administrator made by Ah Jie, without the right to add administrators (the design's demo).
fn set_manage(ui: &MainWindow) {
    let manage = ui.global::<ChatManage>();
    manage.set_open(true);
    manage.set_page(ManagePage::Main);
    manage.set_group(true);
    manage.set_supergroup(true);
    manage.set_title("Manage group".into());
    manage.set_can_save(false);
    manage.set_name("Keyboard Lab".into());
    manage.set_description("Custom builds, switches and keycaps. No ads; trades go to @kbd_market.".into());
    manage.set_initial("K".into());
    manage.set_has_picture(false);
    manage.set_can_info(true);
    manage.set_public(true);
    manage.set_username("kbd_lab".into());
    manage.set_check(UsernameCheck::Same);
    manage.set_check_text("This is the current username".into());
    manage.set_can_invite(true);
    manage.set_primary_link("t.me/+kM3pQ8rTz1aX".into());
    manage.set_links(model(vec![ManageLink { link: "t.me/+a1".into(), name: "Thursday voice chat".into(), meta: "5 days left · No limit · 12 joined".into() }]));
    manage.set_expiries(model(vec![chip("3600", "1 hour", false), chip("86400", "1 day", true), chip("604800", "1 week", false), chip("0", "No limit", false)]));
    manage.set_limits(model(vec![chip("1", "1 person", false), chip("10", "10 people", false), chip("100", "100 people", false), chip("0", "No limit", true)]));
    manage.set_count_line("486 members".into());
    let people = vec![
        person("aj", "Ah Jie", "Group owner", true, false, false),
        person("xz", "Xiao Zhou", "Administrator", false, true, false),
        person("me", "Zhou Ye", "Administrator", false, true, true),
        person("yk", "Yuki", "Administrator", false, true, false),
        person("mm", "Momo", "", false, false, false),
        person("ken", "Ken", "", false, false, false),
    ];
    manage.set_members(model(people.clone()));
    manage.set_admins(model(vec![
        ManagePerson { sub: "Group owner · every right".into(), ..people[0].clone() },
        ManagePerson { sub: "5 rights · set by Ah Jie".into(), ..people[1].clone() },
        ManagePerson { sub: "6 rights · set by Ah Jie".into(), ..people[2].clone() },
        ManagePerson { sub: "2 rights · set by Zhou Ye".into(), ..people[3].clone() },
    ]));
    manage.set_perms_summary("5 of 8 allowed · what members can do".into());
    manage.set_perms(model(vec![
        toggle("send", "Send messages", "", true, true),
        toggle("media", "Send photos and videos", "", true, true),
        toggle("stickers", "Send stickers and GIFs", "", true, true),
        toggle("links", "Add link previews", "", true, true),
        toggle("polls", "Send polls", "", true, true),
        toggle("invite", "Invite members", "", false, true),
        toggle("pin", "Pin messages", "", false, true),
        toggle("info", "Change group info", "", false, true),
    ]));
    manage.set_slow_rows(model(vec![
        model(vec![chip("0", "Off", true), chip("10", "10 s", false), chip("30", "30 s", false), chip("60", "1 min", false)]),
        model(vec![chip("300", "5 min", false), chip("900", "15 min", false), chip("3600", "1 hour", false)]),
    ]));
    manage.set_slow_note("Off: members can send messages one after another.".into());
    manage.set_options(model(vec![
        toggle("approve", "Approve new members", "New members join once an administrator approves them", false, true),
        toggle("topics", "Topics", "Split the group into topics, each with its own chat", false, true),
        toggle("protect", "Protected content", "Members cannot forward or save what is posted here", false, true),
        toggle("history", "History for new members", "New members see the messages from before they joined", true, true),
        toggle("hidden", "Hide members", "Members cannot see the member list; needs 100 members", false, true),
        toggle("antispam", "Aggressive anti-spam", "Telegram deletes spam on its own; needs 200 members", true, false),
    ]));
    // The rights page: Xiao Zhou, made by Ah Jie: Zhou Ye may only look.
    manage.set_who(ManagePerson { sub: "Administrator · set by Ah Jie".into(), ..people[1].clone() });
    manage.set_lock_note("You do not have the “Add administrators” right: view only. Ask the owner, Ah Jie.".into());
    manage.set_custom_title("Keycap advisor".into());
    manage.set_title_editable(false);
    manage.set_rights(model(vec![
        toggle("info", "Change group info", "", true, false),
        toggle("del", "Delete messages", "", true, false),
        toggle("ban", "Ban members", "", true, false),
        toggle("invite", "Invite members", "", true, false),
        toggle("pin", "Pin messages", "", true, false),
        toggle("topics", "Manage topics", "", false, false),
        toggle("call", "Manage video chats", "", false, false),
        toggle("anon", "Remain anonymous", "", false, false),
        toggle("promote", "Add administrators", "", false, false),
    ]));
    manage.set_can_dismiss(false);
}

fn chip(id: &str, label: &str, on: bool) -> ManageChip {
    ManageChip { id: id.into(), label: label.into(), on }
}

fn toggle(id: &str, label: &str, note: &str, on: bool, enabled: bool) -> ManageToggle {
    ManageToggle { id: id.into(), label: label.into(), note: note.into(), on, enabled, lock: if enabled { SharedString::new() } else { "You need the “Add administrators” right for that.".into() } }
}

fn person(id: &str, name: &str, tag: &str, owner: bool, admin: bool, you: bool) -> ManagePerson {
    ManagePerson {
        id: id.into(),
        name: name.into(),
        initial: name.chars().next().map(|c| c.to_string()).unwrap_or_default().into(),
        picture: Image::default(),
        has_picture: false,
        you,
        tag: tag.into(),
        owner,
        admin,
        can_promote: false,
        promote_lock: "You need the “Add administrators” right for that.".into(),
        can_remove: !owner && !you && !admin,
        confirming: false,
        sub: SharedString::new(),
    }
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
    account.set_id("6130081346".into());
    account.set_bio("Design, and a little code. Shanghai.".into());
    // Three accounts signed in: the account menu, the toolbox and Broadsheet's rail show them.
    let rows = vec![
        AccountRow {
            slot: 1,
            index: 0,
            name: "Zhou Ye".into(),
            sub: "+86 138 0013 2046".into(),
            initial: "Z".into(),
            picture: portrait(330.0),
            has_picture: true,
            unread: 0,
            current: true,
            ready: true,
        },
        AccountRow {
            slot: 2,
            index: 1,
            name: "Lin Xia".into(),
            sub: "@linxia".into(),
            initial: "L".into(),
            picture: Image::default(),
            has_picture: false,
            unread: 3,
            current: false,
            ready: true,
        },
        AccountRow {
            slot: 3,
            index: 2,
            name: "Keycap Studio".into(),
            sub: "+86 21 5555 0199".into(),
            initial: "K".into(),
            picture: Image::default(),
            has_picture: false,
            unread: 0,
            current: false,
            ready: true,
        },
    ];
    let accounts = ui.global::<Accounts>();
    accounts.set_count(3);
    accounts.set_index(0);
    accounts.set_others_unread(3);
    accounts.set_others(model(rows.iter().filter(|row| !row.current).cloned().collect()));
    accounts.set_rows(model(rows));

    // Some chats have photos, the others their letters, as in any account.
    let keyboards = ChatRow {
        pinned: true,
        ..chat("keyboards", "Keyboard Lab", ChatKind::Group, moment(Day::Today, 14, 20), "Jie", "@Zhou Ye do you still sell the dark keycaps?", 42, 486)
    };
    let alex = with_portrait(chat("alex", "Alex Chen", ChatKind::User, moment(Day::Today, 12, 8), "", "Can we move the call to Thursday?", 1, 0), 120.0);
    let linxia = with_portrait(chat("linxia", "Lin Xia", ChatKind::User, moment(Day::Today, 13, 52), "", "Is that place open tonight?", 2, 0), 200.0);
    // Words left in its composer (drafts.rs): the list shows them as a draft.
    let linxia = ChatRow { draft: "See you at 7 then?".into(), ..linxia };
    let news = with_portrait(chat("news", "Tech Morning", ChatKind::Channel, moment(Day::Today, 7, 30), "", "New chip export rules take effect", 12, 82413), 25.0);
    let mom = chat("mom", "Mom", ChatKind::User, moment(Day::Yesterday, 18, 32), "", "Never mind, if you are busy don't reply", 0, 0);
    let finch = with_portrait(chat("finch", "FinchGram Updates", ChatKind::Channel, moment(Day::Yesterday, 20, 0), "", "FinchGram 0.1: the design arrives", 0, 41000), 260.0);
    let books = chat("books", "Wednesday Book Club", ChatKind::Group, moment(Day::ThisWeek, 20, 15), "Chen", "Next up: Invisible Cities", 0, 9);
    let saved = ChatRow { kind: ChatKind::Saved, ..chat("saved", "Saved Messages", ChatKind::Saved, moment(Day::ThisYear, 22, 42), "", "Kyoto, check-in October 3", 0, 0) };
    let personal = vec![keyboards.clone(), linxia.clone(), mom.clone(), books.clone()];
    let work = vec![alex.clone()];
    let channels = vec![news.clone(), finch.clone()];
    let all = vec![keyboards.clone(), linxia.clone(), alex.clone(), news.clone(), mom.clone(), finch.clone(), books.clone()];

    let chats = ui.global::<Chats>();
    // Saved Messages is the fixed row above the lists, in no folder.
    chats.set_saved_messages(saved);
    chats.set_saved_messages_shown(true);
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

/// The toolbox as src/toolbox.rs fills it: downloads pinned, every category shown.
fn set_toolbox(ui: &MainWindow) {
    crate::toolbox::show_for_pictures(ui, &["downloads"], "all", "");
}

fn info_row(id: &str, kind: InfoRowKind, icon: &str, label: &str, key: &str) -> InfoRow {
    InfoRow {
        id: id.into(),
        key: key.into(),
        icon: icon.into(),
        label: label.into(),
        sub: SharedString::new(),
        value: SharedString::new(),
        kind,
        on: false,
        open: false,
        danger: false,
        grey: false,
        enabled: true,
        child: false,
    }
}

fn info_section(id: &str, title: &str, rows: Vec<InfoRow>) -> InfoSection {
    InfoSection {
        id: id.into(),
        title: title.into(),
        grey: false,
        collapsed: false,
        shared: false,
        rows: model(rows),
        members: ModelRc::default(),
        tabs: ModelRc::default(),
    }
}

/// The info panel's sections for Keyboard Lab, as chat_info.rs would build them.
fn info_sections() -> Vec<InfoSection> {
    let member = |name: &str, role: &str, status: Status, hour: i32| InfoMember {
        name: name.into(),
        initial: name.chars().next().map(|c| c.to_string()).unwrap_or_default().into(),
        picture: Image::default(),
        has_picture: false,
        status,
        last_seen: moment(Day::Today, hour, 5),
        role: role.into(),
    };
    vec![
        info_section("info", "Info", vec![
            InfoRow { value: "Custom builds, switches and keycaps. No ads; trades go to @kbd_market.\nGroup call every Thursday at 21:00.".into(), ..info_row("desc", InfoRowKind::Info, "info", "Description", "") },
            InfoRow { value: "t.me/kbd_lab".into(), ..info_row("link", InfoRowKind::Info, "link", "Link", "") },
        ]),
        info_section("settings", "Settings", vec![
            InfoRow { on: true, ..info_row("notif", InfoRowKind::Toggle, "bell", "Notifications", "m") },
            InfoRow { grey: true, ..info_row("mutefor", InfoRowKind::Nav, "clock", "Mute duration & exceptions", "") },
            InfoRow { on: true, ..info_row("pin", InfoRowKind::Toggle, "push-pin", "Pin chat", "p") },
            InfoRow { value: "Keyboards".into(), ..info_row("folders", InfoRowKind::Nav, "folder-simple", "Add to folder", "f") },
            InfoRow { value: "42 unread".into(), ..info_row("read", InfoRowKind::Action, "checks", "Mark as read", "r") },
            InfoRow { grey: true, value: "Off".into(), ..info_row("autodel", InfoRowKind::Nav, "timer", "Auto-delete messages", "") },
        ]),
        InfoSection { grey: true, shared: true, tabs: model(vec!["Photos & Videos".into(), "Files".into(), "Links".into()]), ..info_section("shared", "Shared Media", Vec::new()) },
        InfoSection {
            grey: true,
            members: model(vec![
                member("Jie", "owner", Status::Online, 14),
                member("Mika", "admin", Status::LastSeen, 13),
                member("Mi", "", Status::Online, 14),
                member("Yuki", "admin", Status::LastSeen, 11),
                member("Ken", "", Status::LastSeen, 9),
            ]),
            ..info_section("members", "Members · 486", vec![
                InfoRow { grey: true, value: "3".into(), ..info_row("admins", InfoRowKind::Nav, "shield-check", "Administrators", "") },
                InfoRow { grey: true, value: "486".into(), ..info_row("allm", InfoRowKind::Nav, "users", "Show all members", "") },
            ])
        },
        info_section("invite", "Invite", vec![
            InfoRow { grey: true, value: "t.me/+Kx7bQ2mL0aZj".into(), ..info_row("invite", InfoRowKind::Info, "link-simple", "Invite link", "") },
            info_row("manage", InfoRowKind::Nav, "gear-six", "Manage group", ""),
        ]),
        info_section("actions", "Actions", vec![
            InfoRow { danger: true, ..info_row("report", InfoRowKind::Action, "flag", "Report", "x") },
            InfoRow { danger: true, grey: true, ..info_row("clear", InfoRowKind::Action, "broom", "Clear history", "") },
            InfoRow { danger: true, ..info_row("leave", InfoRowKind::Action, "sign-out", "Leave group", "l") },
        ]),
    ]
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
        MessageRow {
            reactions: model(vec![
                Reaction { emoji: "👍".into(), count: "6".into(), chosen: true },
                Reaction { emoji: "🎉".into(), count: "2".into(), chosen: false },
            ]),
            ..message("5", "Mi", "11:05", "Thursday 15:00 at the usual place. Who's coming?")
        },
        MessageRow {
            forwarded_from: "Keyboard Lab Notices".into(),
            ..message("5a", "Mi", "11:40", "October group buy: dark PBT keycaps, orders close October 8.")
        },
        MessageRow {
            reply: ReplyQuote { shown: true, gone: true, ..ReplyQuote::default() },
            ..message("5b", "Mika", "14:05", "Saw it before it was deleted, looks great.")
        },
        // A round video message and a contact card (the design's fifth round).
        MessageRow {
            content: Content::VideoNote,
            round: RoundNote { shown: true, picture: gradient(12, 12, 200.0), has_picture: true, duration: "0:14".into(), playing: false, loading: false },
            ..message("5e", "Jie", "14:08", "")
        },
        MessageRow {
            content: Content::Contact,
            contact: ContactCard {
                shown: true,
                name: "Aunt Wang".into(),
                initial: "A".into(),
                phone: "+86 139 0571 2233".into(),
                registered: true,
                added: false,
            },
            ..message("5f", "Mika", "14:09", "")
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
    let post = |id: &str, time: &str, text: &str| MessageRow { sender: "Tech Morning".into(), can_share: true, ..message(id, "Tech Morning", time, text) };
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

/// A person, a chat or a message as the search and the dialogs list it.
fn found(section: FoundKind, index: i32, title: &str, kind: ChatKind, hue: Option<f32>) -> FoundRow {
    FoundRow {
        section,
        index,
        id: title.to_lowercase().replace(' ', "-").into(),
        user_id: if matches!(kind, ChatKind::User | ChatKind::Bot) { "7".into() } else { SharedString::new() },
        title: title.into(),
        initial: title.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default().into(),
        picture: hue.map(portrait).unwrap_or_default(),
        has_picture: hue.is_some(),
        kind,
        status: match kind {
            ChatKind::Group => Status::Members,
            ChatKind::Channel => Status::Subscribers,
            ChatKind::Bot => Status::Bot,
            _ => Status::Recently,
        },
        ..FoundRow::default()
    }
}

/// The search over everything (the design's compose.js round): the chats used last, the results
/// for words, a phone number nobody has, a public group's card; the pencil's menu; the dialogs that
/// make a group (its members, its name), a channel (its name, its type public and private, its
/// subscribers), a chat, a contact (with a number that has no account).
/// Bots (src/telegram/bots.rs): the group's pinned message as the bar under the header; a bot's
/// chat with the buttons under its messages (one waiting for the bot's answer), its keyboard, its
/// menu button; its commands offered for "/"; its answer as a dialog.
fn bots(ui: &MainWindow, window: &MinimalSoftwareWindow, name: &dyn Fn(&str) -> String) {
    let conversation = ui.global::<Conversation>();
    let actions = ui.global::<Actions>();
    conversation.set_pinned_message(ReplyQuote {
        shown: true,
        id: "5".into(),
        sender: "Mi".into(),
        content: Content::Text,
        text: "Thursday 15:00 at the usual place. Who's coming?".into(),
        ..ReplyQuote::default()
    });
    save(window, &name("pinned"));
    conversation.set_pinned_message(ReplyQuote::default());

    let button = |text: &str, kind: ButtonKind, row: i32, column: i32| ChatButton { text: text.into(), kind, row, column };
    let rows = |rows: Vec<Vec<ChatButton>>| model(rows.into_iter().map(|buttons| ButtonRow { buttons: model(buttons) }).collect());
    conversation.set_chat_id("groupbuy".into());
    conversation.set_title("GroupBuy Bot".into());
    conversation.set_initial("G".into());
    conversation.set_kind(ChatKind::Bot);
    conversation.set_status(Status::Bot);
    conversation.set_members(0);
    conversation.set_online_members(0);
    conversation.set_can_write(true);
    conversation.set_can_send_photos(true);
    conversation.set_can_send_videos(true);
    conversation.set_can_send_files(true);
    conversation.set_messages(model(vec![
        day_row(Day::Today),
        message("1", "Zhou Ye", "09:40", "/start"),
        MessageRow {
            buttons: rows(vec![
                vec![button("Open orders", ButtonKind::Callback, 0, 0), button("My orders", ButtonKind::Callback, 0, 1)],
                vec![button("Delivery list", ButtonKind::Url, 1, 0)],
                vec![button("Copy my order code", ButtonKind::Copy, 2, 0), button("Share the group buy", ButtonKind::SwitchInline, 2, 1)],
            ]),
            ..message("2", "GroupBuy Bot", "09:40", "Welcome to the Keyboard Lab group buy. What would you like to do?")
        },
        message("3", "Zhou Ye", "09:41", "Dark PBT keycaps, one set"),
        MessageRow {
            buttons: rows(vec![vec![button("Pay now", ButtonKind::Buy, 0, 0), button("Cancel the order", ButtonKind::Callback, 0, 1)]]),
            ..message("4", "GroupBuy Bot", "09:41", "Order #1045: dark PBT keycaps × 1, 48 USD. Orders close on October 8.")
        },
    ]));
    conversation.set_keyboard(rows(vec![
        vec![button("📦 My orders", ButtonKind::Text, 0, 0), button("🧾 Invoice", ButtonKind::Text, 0, 1)],
        vec![button("📞 Share my number", ButtonKind::RequestPhone, 1, 0)],
    ]));
    conversation.set_keyboard_placeholder("Choose what to do…".into());
    conversation.set_keyboard_hidden(false);
    conversation.set_menu_button("/".into());
    // "Cancel the order" is waiting for the bot.
    actions.set_busy_button("4:0:1".into());
    save(window, &name("bot"));
    actions.set_busy_button(SharedString::new());

    // "/" typed, the keyboard folded away: the bot's commands.
    conversation.set_keyboard_hidden(true);
    conversation.set_draft("/".into());
    conversation.set_commands(model(vec![
        BotCommand { command: "/start".into(), description: "Start over".into() },
        BotCommand { command: "/orders".into(), description: "Your orders in this group buy".into() },
        BotCommand { command: "/status".into(), description: "Where the shipment is".into() },
        BotCommand { command: "/help".into(), description: "What this bot does".into() },
    ]));
    save(window, &name("bot-commands"));
    conversation.set_commands(ModelRc::default());
    conversation.set_draft(SharedString::new());

    // The bot's answer to a button, as the dialog it asked for.
    actions.set_alert_text("Orders close on October 8 at 23:59. Yours is in: #1045, dark PBT keycaps × 1.".into());
    actions.set_alert_open(true);
    save(window, &name("bot-alert"));
    actions.set_alert_open(false);

    conversation.set_keyboard(ModelRc::default());
    conversation.set_keyboard_placeholder(SharedString::new());
    conversation.set_keyboard_hidden(false);
    conversation.set_menu_button(SharedString::new());
    open_keyboards(ui);
}

/// The design's fifth round (src/telegram/{reactions,topics,share,admin}.rs): the reactions'
/// picker; the settings panel as an administrator and as a member; slow mode in the composer; a
/// group with topics, its list, a new topic's form, a topic open, a closed one; the share panel.
fn extras(ui: &MainWindow, window: &MinimalSoftwareWindow, name: &dyn Fn(&str) -> String) {
    let conversation = ui.global::<Conversation>();
    // Keyboard Lab is open (open_keyboards): its end shows Mi's message with reactions and the
    // picker open on it, Jie's round video message and Mika's contact card.
    let messages = conversation.get_messages();
    let kept: Vec<MessageRow> = (0..messages.row_count())
        .filter_map(|index| messages.row_data(index))
        .filter(|row| ["", "4", "5", "5e", "5f"].contains(&row.id.as_str()) && row.kind != RowKind::Day || row.kind == RowKind::Day && row.id.is_empty())
        .collect();
    conversation.set_messages(model(kept));
    conversation.set_reaction_choices(model(["👍", "❤️", "🔥", "🎉", "😂", "😮", "😢", "🙏"].map(SharedString::from).to_vec()));
    conversation.set_reaction_max(1);
    conversation.set_reaction_picker("5".into());
    save(window, &name("reactions"));
    conversation.set_reaction_picker(SharedString::new());
    save(window, &name("extras"));

    // The management page in the info panel's place (the design's "管理群组 / 频道" round): the
    // group as one of its administrators, then an administrator's rights page.
    {
        let info = ui.global::<ChatInfo>();
        info.set_open(true);
        set_manage(ui);
        save(window, &name("manage"));
        let manage = ui.global::<ChatManage>();
        manage.set_page(ManagePage::Rights);
        manage.set_title("Administrator rights".into());
        save(window, &name("manage-rights"));
        manage.set_page(ManagePage::Main);
        manage.set_open(false);
        info.set_open(false);
    }

    // Slow mode holds the composer: one message every 30 seconds, 22 still to wait.
    conversation.set_slow_mode(30);
    conversation.set_slow_limited(true);
    conversation.set_slow_wait(22);
    save(window, &name("slow-mode"));
    conversation.set_slow_wait(0);
    conversation.set_slow_limited(false);
    conversation.set_slow_mode(0);

    // A group with topics: Film Photography opens on its topics.
    let topic = |id: &str, icon: &str, colour: u32, title: &str, last: &str, hour: i32, minute: i32, unread: i32, pinned: bool, closed: bool| TopicRow {
        id: id.into(),
        icon: icon.into(),
        color: slint::Color::from_rgb_u8((colour >> 16) as u8, (colour >> 8) as u8, colour as u8),
        title: title.into(),
        last: last.into(),
        time: moment(Day::Today, hour, minute),
        unread,
        pinned,
        closed,
    };
    conversation.set_chat_id("film".into());
    conversation.set_title("Film Photography".into());
    conversation.set_initial("F".into());
    conversation.set_kind(ChatKind::Group);
    conversation.set_status(Status::Members);
    conversation.set_members(1204);
    conversation.set_online_members(0);
    conversation.set_can_write(true);
    conversation.set_messages(ModelRc::default());
    conversation.set_is_forum(true);
    conversation.set_showing_topics(true);
    conversation.set_topics(model(vec![
        topic("1", "", 0x6FB9F0, "General", "Lao Zhou: Any tickets left for the weekend screening?", 13, 40, 0, true, false),
        topic("2", "📷", 0xFFD67E, "Gear talk", "Ajie: What does a used FM2 go for now?", 14, 2, 3, false, false),
        topic("3", "🚶", 0x8EEE98, "Weekend photo walk", "Kai: Saturday 9:00, meet at Wukang Road", 11, 15, 12, false, false),
        topic("4", "🎞️", 0xCB86DB, "Scanning and editing", "Lulu: The scan settings are pinned", 9, 30, 0, false, false),
        topic("5", "📌", 0xFB6F5F, "Rules", "An administrator closed this topic", 8, 0, 0, false, true),
    ]));
    conversation.set_topic_count(5);
    save(window, &name("topics"));
    // A new topic's form.
    conversation.set_topic_icons(model(["💬", "📷", "🎞️", "🗺️", "📚", "🎧", "⚠️", "🎉"].map(SharedString::from).to_vec()));
    conversation.set_new_topic_icon(1);
    conversation.set_new_topic_name("Darkroom".into());
    conversation.set_new_topic_open(true);
    save(window, &name("topic-new"));
    conversation.set_new_topic_open(false);
    // Gear talk, open.
    conversation.set_showing_topics(false);
    conversation.set_topic_id("2".into());
    conversation.set_topic_title("Gear talk".into());
    conversation.set_topic_icon("📷".into());
    conversation.set_topic_color(slint::Color::from_rgb_u8(0xFF, 0xD6, 0x7E));
    conversation.set_topic_closed(false);
    conversation.set_messages(model(vec![
        day_row(Day::Today),
        message("t1", "Xiaoyu", "13:10", "Got an FM2, the shutter sound is lovely"),
        message("t2", "Lulu", "13:31", "Congrats! Replace the light seals first"),
        message("t3", "Ajie", "14:02", "What does a used FM2 go for now? Worth it?"),
    ]));
    save(window, &name("topic"));
    // Rules, closed.
    conversation.set_topic_id("5".into());
    conversation.set_topic_title("Rules".into());
    conversation.set_topic_icon("📌".into());
    conversation.set_topic_color(slint::Color::from_rgb_u8(0xFB, 0x6F, 0x5F));
    conversation.set_topic_closed(true);
    conversation.set_messages(model(vec![
        day_row(Day::Today),
        message("t4", "Mi", "10:00", "Say which camera and film a photo was shot on; no advertising."),
    ]));
    save(window, &name("topic-closed"));
    conversation.set_topic_closed(false);
    conversation.set_topic_id(SharedString::new());
    conversation.set_is_forum(false);
    conversation.set_topics(ModelRc::default());
    conversation.set_topic_count(0);

    // The share panel: a post of Tech Morning, two chats chosen.
    let share = ui.global::<Share>();
    let row = |id: &str, title: &str, kind: ChatKind, sub: &str, hue: Option<f32>, picked: bool| ShareRow {
        id: id.into(),
        title: title.into(),
        initial: title.chars().next().map(String::from).unwrap_or_default().into(),
        picture: hue.map(portrait).unwrap_or_default(),
        has_picture: hue.is_some(),
        kind,
        sub: sub.into(),
        picked,
    };
    share.set_kind(ShareKind::Message);
    share.set_preview("Tech Morning: New chip export rules take effect next month".into());
    share.set_query(SharedString::new());
    share.set_rows(model(vec![
        row("saved", "Saved Messages", ChatKind::Saved, "Only you can see it", None, false),
        row("keyboards", "Keyboard Lab", ChatKind::Group, "486 members", None, true),
        row("linxia", "Lin Xia", ChatKind::User, "online", Some(200.0), true),
        row("alex", "Alex Chen", ChatKind::User, "", Some(120.0), false),
        row("mom", "Mom", ChatKind::User, "", None, false),
        row("book", "Wednesday Book Club", ChatKind::Group, "9 members", None, false),
    ]));
    share.set_picked(2);
    share.set_comment(SharedString::new());
    share.set_error(SharedString::new());
    share.set_busy(false);
    share.set_open(true);
    save(window, &name("share"));
    share.set_open(false);
    open_keyboards(ui);
}

fn compose(ui: &MainWindow, window: &MinimalSoftwareWindow, theme: Theme, name: &dyn Fn(&str) -> String) {
    let search = ui.global::<Search>();
    let recent = [
        found(FoundKind::Chat, 0, "Keyboard Lab", ChatKind::Group, None),
        found(FoundKind::Chat, 1, "Lin Xia", ChatKind::User, Some(200.0)),
        found(FoundKind::Chat, 2, "Alex Chen", ChatKind::User, Some(120.0)),
        found(FoundKind::Chat, 3, "Tech Morning", ChatKind::Channel, Some(25.0)),
        found(FoundKind::Chat, 4, "Mom", ChatKind::User, None),
        found(FoundKind::Chat, 5, "Wednesday Book Club", ChatKind::Group, None),
    ];
    search.set_query(SharedString::new());
    search.set_empty(true);
    search.set_recent(model(recent.iter().map(|row| FoundRow { members: if row.kind == ChatKind::Group { 486 } else { 0 }, ..row.clone() }).collect()));
    search.set_sections(ModelRc::default());
    search.set_count(6);
    search.set_selected(1);
    search.set_none(false);
    search.set_phone_missed(false);
    search.set_error(SharedString::new());
    search.set_card_open(false);
    search.set_open(true);
    save(window, &name("search-recent"));

    let section = |kind: FoundKind, rows: Vec<FoundRow>| FoundSection { kind, count: rows.len() as i32, rows: model(rows) };
    let keyboards = FoundRow { members: 486, ..found(FoundKind::Chat, 0, "Keyboard Lab", ChatKind::Group, None) };
    let kai = FoundRow { username: "kai_hikes".into(), status: Status::Online, ..found(FoundKind::Contact, 1, "Kai", ChatKind::User, None) };
    let keebcn = FoundRow { username: "keebcn".into(), members: 12384, ..found(FoundKind::Public, 2, "Mechanical keyboards", ChatKind::Group, None) };
    let keeb_daily = FoundRow { username: "keeb_daily".into(), members: 36000, ..found(FoundKind::Public, 3, "Keyboard Daily", ChatKind::Channel, Some(300.0)) };
    let kiro = FoundRow { username: "kiro_caps".into(), ..found(FoundKind::Public, 4, "Kiro", ChatKind::User, None) };
    let message = |index: i32, chat: &str, kind: ChatKind, sender: &str, text: &str| FoundRow {
        message_id: "42".into(),
        sender: sender.into(),
        text: text.into(),
        time: moment(Day::Today, 14, 20),
        ..found(FoundKind::Message, index, chat, kind, None)
    };
    search.set_query("keeb".into());
    search.set_empty(false);
    search.set_recent(ModelRc::default());
    search.set_sections(model(vec![
        section(FoundKind::Chat, vec![keyboards, kai]),
        section(FoundKind::Public, vec![keebcn.clone(), keeb_daily, kiro]),
        section(FoundKind::Message, vec![
            message(5, "Keyboard Lab", ChatKind::Group, "Jie", "the keeb meetup is on Saturday, bring the 75%"),
            message(6, "Lin Xia", ChatKind::User, "", "saw your keeb photos, the dark keycaps look great"),
        ]),
    ]));
    search.set_count(7);
    search.set_selected(2);
    save(window, &name("search-results"));

    search.set_query("+86 139 1234 5678".into());
    search.set_sections(ModelRc::default());
    search.set_count(0);
    search.set_selected(0);
    search.set_phone_missed(true);
    save(window, &name("search-phone"));
    search.set_phone_missed(false);

    search.set_query("keeb".into());
    search.set_card(keebcn);
    search.set_card_about("Custom builds, switches and keycaps; questions from beginners are welcome too.".into());
    search.set_card_busy(false);
    search.set_card_open(true);
    save(window, &name("search-card"));
    search.set_card_open(false);
    search.set_open(false);

    // The pencil's menu: a click on the pencil.
    let (px, py) = match theme {
        Theme::Workbench => (278.0, 61.0),
        Theme::Broadsheet => (358.0, 76.0),
        Theme::Terminal => (224.0, 89.0),
    };
    click(window, px, py);
    save(window, &name("new-menu"));
    escape(window);

    let new_chat = ui.global::<NewChat>();
    let contact = |index: i32, name: &str, username: &str, status: Status, hue: Option<f32>, picked: bool| FoundRow {
        username: username.into(),
        status,
        picked,
        ..found(FoundKind::Contact, index, name, ChatKind::User, hue)
    };
    let contacts = vec![
        contact(0, "Lin Xia", "linxia_lx", Status::Online, Some(200.0), true),
        contact(1, "Alex Chen", "alexc", Status::Recently, Some(120.0), false),
        contact(2, "Mom", "", Status::LastWeek, None, false),
        contact(3, "Jie", "ajie_keys", Status::Online, None, true),
        contact(4, "Mika", "mika_t", Status::Recently, None, false),
        contact(5, "Chen", "chen_reads", Status::LastWeek, None, false),
        contact(6, "Kai", "kai_hikes", Status::Online, None, false),
    ];
    let picked: Vec<FoundRow> = contacts.iter().filter(|row| row.picked).cloned().collect();
    new_chat.set_contacts(model(contacts.clone()));
    new_chat.set_picked(model(picked));
    new_chat.set_picked_names("Lin Xia, Jie".into());
    new_chat.set_query(SharedString::new());
    new_chat.set_name(SharedString::new());
    new_chat.set_name_initial(SharedString::new());
    new_chat.set_about(SharedString::new());
    new_chat.set_has_photo(false);
    new_chat.set_public(true);
    new_chat.set_username(SharedString::new());
    new_chat.set_link_check(LinkCheck::Empty);
    new_chat.set_invite_link(SharedString::new());
    new_chat.set_first_name(SharedString::new());
    new_chat.set_last_name(SharedString::new());
    new_chat.set_phone(SharedString::new());
    new_chat.set_phone_ok(false);
    new_chat.set_busy(false);
    new_chat.set_error(SharedString::new());
    new_chat.set_link_copied(false);
    new_chat.set_can_go_back(false);
    new_chat.set_step(1);
    new_chat.set_dialog(NewDialog::Group);
    save(window, &name("new-group-members"));
    new_chat.set_step(2);
    new_chat.set_can_go_back(true);
    new_chat.set_name("Keycap Swap".into());
    new_chat.set_name_initial("K".into());
    save(window, &name("new-group-name"));

    new_chat.set_dialog(NewDialog::Channel);
    new_chat.set_step(1);
    new_chat.set_can_go_back(false);
    new_chat.set_name("Keyboard Daily".into());
    new_chat.set_about("One piece of keyboard news a day, and when a group buy opens.".into());
    save(window, &name("new-channel-info"));
    new_chat.set_step(2);
    new_chat.set_username("keeb_daily".into());
    new_chat.set_link_check(LinkCheck::Taken);
    save(window, &name("new-channel-public"));
    new_chat.set_public(false);
    new_chat.set_invite_link("https://t.me/+Hq3kT9xWm2Y4NzE0".into());
    save(window, &name("new-channel-private"));
    new_chat.set_step(3);
    new_chat.set_can_go_back(true);
    new_chat.set_picked(ModelRc::default());
    new_chat.set_contacts(model(contacts.iter().map(|row| FoundRow { picked: false, ..row.clone() }).collect()));
    save(window, &name("new-channel-invite"));

    new_chat.set_dialog(NewDialog::Direct);
    new_chat.set_step(1);
    new_chat.set_can_go_back(false);
    save(window, &name("new-chat"));

    new_chat.set_dialog(NewDialog::Contact);
    new_chat.set_first_name("Song".into());
    new_chat.set_last_name("Yi".into());
    new_chat.set_phone("+86 139 1234 5670".into());
    new_chat.set_phone_ok(true);
    new_chat.set_error("This number is not on Telegram yet.".into());
    save(window, &name("new-contact"));
    new_chat.set_error(SharedString::new());
    new_chat.set_dialog(NewDialog::None);
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

/// Settings → AI access as in the design: on, with chosen chats, two apps (one connected now) and
/// two turned away, and a log; the picker, plain and searched; off; and the dialog, for an app
/// the system named and for one it would not.
fn ai_access(ui: &MainWindow, window: &MinimalSoftwareWindow, name: &dyn Fn(&str) -> String) {
    let app = ui.global::<AppState>();
    let ai = ui.global::<AiAccess>();
    let chat = |id: &str, title: &str, kind: ChatKind, members: i32, secret: bool, chosen: bool| AiChat {
        id: id.into(),
        title: title.into(),
        kind,
        members,
        secret,
        chosen,
    };
    let program = |name: &str, path: &str, allowed: bool, live: bool, last: Option<Moment>| AiApp {
        key: path.into(),
        name: name.into(),
        path: path.into(),
        allowed,
        live,
        has_last_used: last.is_some(),
        last_used: last.unwrap_or_default(),
    };
    let entry = |kind: AiActivityKind, app: &str, hour: i32, minute: i32, chat: &str, count: i32| AiActivity {
        kind,
        app: app.into(),
        time: moment(Day::Today, hour, minute),
        chat: chat.into(),
        count,
    };
    app.set_page(Page::AiAccess);
    ai.set_on(true);
    ai.set_scope(AiScope::Chosen);
    ai.set_writing(AiWriting::Drafts);
    ai.set_chosen_count(3);
    ai.set_chosen(model(vec![
        chat("1", "Keyboard Lab", ChatKind::Group, 486, false, true),
        chat("2", "Weekend Hikers", ChatKind::Group, 23, false, true),
        chat("3", "Mika", ChatKind::User, 0, false, true),
    ]));
    ai.set_apps(model(vec![
        program("Claude", "/Applications/Claude.app", true, true, None),
        program("Zed", "/Applications/Zed.app", true, false, Some(moment(Day::ThisWeek, 21, 40))),
        program("claude", "/Users/me/.local/bin/claude", false, false, Some(moment(Day::Today, 14, 7))),
        program("", "pid 48213", false, false, Some(moment(Day::Today, 14, 8))),
    ]));
    ai.set_activity(model(vec![
        entry(AiActivityKind::Draft, "Claude", 14, 6, "Weekend Hikers", 0),
        entry(AiActivityKind::Read, "Claude", 14, 5, "Keyboard Lab", 20),
        entry(AiActivityKind::Refused, "Claude", 14, 5, "Tech Morning", 0),
        entry(AiActivityKind::Searched, "Claude", 14, 4, "", 3),
        entry(AiActivityKind::Connected, "Claude", 14, 3, "", 0),
    ]));
    ai.set_claude_code_command("claude mcp add finchgram -- /Applications/FinchGram.app/Contents/MacOS/finchgram mcp".into());
    ai.set_claude_desktop_config(
        r#"{ "mcpServers": { "finchgram": { "command": "/Applications/FinchGram.app/Contents/MacOS/finchgram", "args": ["mcp"] } } }"#.into(),
    );
    save(window, &name("ai-access"));
    // The rest of the page: the apps and how to connect one, then the log and the rules.
    scroll(window, 800.0, 400.0, 760.0);
    save(window, &name("ai-access-apps"));
    scroll(window, 800.0, 400.0, 760.0);
    save(window, &name("ai-access-log"));
    scroll(window, 800.0, 400.0, -1520.0);
    // The picker: every chat, the chosen ones ticked, a secret chat dimmed; then searched.
    ai.set_all_chats(model(vec![
        chat("1", "Keyboard Lab", ChatKind::Group, 486, false, true),
        chat("2", "Weekend Hikers", ChatKind::Group, 23, false, true),
        chat("4", "Film Photography", ChatKind::Group, 1204, false, false),
        chat("5", "Tech Morning", ChatKind::Channel, 82413, false, false),
        chat("3", "Mika", ChatKind::User, 0, false, true),
        chat("6", "Jie", ChatKind::User, 0, false, false),
        chat("7", "Lin Xia", ChatKind::User, 0, false, false),
        chat("8", "Deer", ChatKind::User, 0, true, false),
    ]));
    ai.set_picking(true);
    save(window, &name("ai-access-chats"));
    ai.set_all_chats(model(vec![
        chat("2", "Weekend Hikers", ChatKind::Group, 23, false, true),
        chat("3", "Mika", ChatKind::User, 0, false, true),
        chat("8", "Deer", ChatKind::User, 0, true, false),
    ]));
    save(window, &name("ai-access-chats-searched"));
    ai.set_picking(false);
    // Off: the switch and the rules only.
    ai.set_on(false);
    save(window, &name("ai-access-off"));
    ai.set_on(true);
    // The dialog, over the chats.
    app.set_page(Page::Chats);
    ai.set_ask_name("Cursor".into());
    ai.set_ask_initial("C".into());
    ai.set_ask_path("/Applications/Cursor.app".into());
    ai.set_ask_open(true);
    save(window, &name("ai-allow"));
    ai.set_ask_name(SharedString::new());
    ai.set_ask_initial(SharedString::new());
    ai.set_ask_path("pid 48213".into());
    save(window, &name("ai-allow-unidentified"));
    ai.set_ask_open(false);
    app.set_page(Page::Settings);
}

/// The Downloads page as it is looked at: three servers, a folder of films with two files chosen,
/// and a queue with a file on its way, one waiting, one stopped and one done.
fn fill_downloads(ui: &MainWindow) {
    let downloads = ui.global::<Downloads>();
    let server = |name: &str, host: &str, protocol: &str, status: ServerStatus| ServerRow { name: name.into(), host: host.into(), protocol: protocol.into(), status };
    let entry = |name: &str, folder: bool, icon: &str, date: &str, size: &str, selected: bool| FolderEntry {
        name: name.into(),
        folder,
        icon: icon.into(),
        date: date.into(),
        size: size.into(),
        selected,
    };
    let transfer = |id: i32, name: &str, icon: &str, state: TransferState, fraction: f32, done: &str, total: &str, speed: &str, seconds: i32, problem: Problem| TransferRow {
        id,
        name: name.into(),
        icon: icon.into(),
        state,
        fraction,
        done: done.into(),
        total: total.into(),
        speed: speed.into(),
        seconds_left: seconds,
        minutes_left: if seconds < 0 { -1 } else { (seconds + 59) / 60 },
        problem,
        detail: SharedString::new(),
        waiting: 0,
    };
    downloads.set_servers(model(vec![
        server("Home NAS", "nas.home", "WebDAV", ServerStatus::Connected),
        server("Studio", "files.studio.example", "FTPS", ServerStatus::Connected),
        server("Seedbox", "box.example.net", "SFTP", ServerStatus::NotReachable),
    ]));
    downloads.set_current(0);
    downloads.set_crumbs(model(vec!["Home NAS".into(), "Movies".into(), "2026".into()]));
    downloads.set_title("2026".into());
    downloads.set_folder_state(FolderState::Listed);
    downloads.set_problem(Problem::None);
    downloads.set_problem_detail(SharedString::new());
    downloads.set_filter(SharedString::new());
    downloads.set_has_entries(true);
    downloads.set_entries(model(vec![
        entry("Documentaries", true, "folder", "Sep 30", "", false),
        entry("Shorts", true, "folder", "Aug 2", "", false),
        entry("Dune Part Three (2026).mkv", false, "film-strip", "Sep 30", "4.4 GB", true),
        entry("Dune Part Three (2026).srt", false, "file-text", "Sep 30", "90 KB", false),
        entry("poster.jpg", false, "file-image", "Sep 29", "2.4 MB", true),
        entry("soundtrack.zip", false, "file-zip", "Sep 28", "48 MB", false),
        entry("notes.pdf", false, "file-pdf", "Mar 3", "1.1 MB", false),
    ]));
    downloads.set_selected_count(2);
    downloads.set_selected_size("4.4 GB".into());
    downloads.set_can_open(false);
    downloads.set_all_selected(false);
    downloads.set_some_selected(true);
    downloads.set_queue(model(vec![
        transfer(1, "Dune Part Three (2026).mkv", "film-strip", TransferState::Active, 0.38, "4.1 GB", "10.8 GB", "24.3 MB/s", 275, Problem::None),
        transfer(2, "poster.jpg", "file-image", TransferState::Waiting, 0.0, "", "2.4 MB", "", -1, Problem::None),
        transfer(3, "soundtrack.zip", "file-zip", TransferState::Stopped, 0.25, "12 MB", "48 MB", "", -1, Problem::Connection),
        transfer(4, "notes.pdf", "file-pdf", TransferState::Done, 1.0, "1.1 MB", "1.1 MB", "", -1, Problem::None),
    ]));
    downloads.set_active_count(1);
    downloads.set_done_count(1);
    downloads.set_queue_open(true);
    downloads.set_folder_label("~/Documents/FinchGram/webdav".into());
    downloads.set_connections(12);
    downloads.set_form_open(false);
    downloads.set_form_editing(false);
    downloads.set_form_error(FormError::None);
    downloads.set_test_state(TestState::None);
}

/// The Downloads page's scenes: browsing, the queue hidden, a file's menu, the folder loading,
/// empty and unreadable, the form for a new server and for one being edited, and no server yet.
fn downloads(ui: &MainWindow, window: &MinimalSoftwareWindow, theme: Theme, name: &dyn Fn(&str) -> String) {
    let app = ui.global::<AppState>();
    let downloads = ui.global::<Downloads>();
    fill_downloads(ui);
    app.set_page(Page::Downloads);
    save(window, &name("downloads"));
    downloads.set_queue_open(false);
    save(window, &name("downloads-collapsed"));
    downloads.set_queue_open(true);
    // A right click on the first file, Dune: its menu.
    let (x, y) = match theme {
        Theme::Workbench => (520.0, 309.0),
        Theme::Broadsheet => (520.0, 334.0),
        Theme::Terminal => (520.0, 305.0),
    };
    right_click(window, x, y);
    save(window, &name("downloads-menu"));
    escape(window);
    // Loading, in a folder whose name is a whole sentence, as PikPak folders are: the path row cuts
    // it short instead of pushing the column off the window.
    let sentence = "【更多高清电影访问 www.mkvhome.com】教父[共3部合集][国英多音轨+简繁字幕].The.Godfather.Trilogy.2160p";
    downloads.set_folder_state(FolderState::Loading);
    downloads.set_crumbs(model(vec!["Home NAS".into(), "Movies".into(), sentence.into()]));
    downloads.set_title(sentence.into());
    save(window, &name("downloads-loading"));
    downloads.set_crumbs(model(vec!["Home NAS".into(), "Movies".into(), "2026".into()]));
    downloads.set_title("2026".into());
    downloads.set_folder_state(FolderState::Listed);
    downloads.set_has_entries(false);
    downloads.set_entries(model(Vec::new()));
    downloads.set_selected_count(0);
    downloads.set_some_selected(false);
    downloads.set_title("Shorts".into());
    downloads.set_crumbs(model(vec!["Home NAS".into(), "Movies".into(), "2026".into(), "Shorts".into()]));
    save(window, &name("downloads-empty"));
    downloads.set_folder_state(FolderState::Error);
    downloads.set_problem(Problem::Unreachable);
    downloads.set_problem_detail("nas.home".into());
    save(window, &name("downloads-error"));
    downloads.set_folder_state(FolderState::Listed);
    // The form: a new server, then Home NAS being edited after a test that worked.
    downloads.set_form_open(true);
    downloads.set_form_editing(false);
    downloads.set_form_name(SharedString::new());
    downloads.set_form_protocol("webdav".into());
    downloads.set_form_address(SharedString::new());
    downloads.set_form_port(SharedString::new());
    downloads.set_form_username(SharedString::new());
    downloads.set_form_password(SharedString::new());
    save(window, &name("downloads-add"));
    // The same with an http address: the note, in red, that the password travels in the clear.
    downloads.set_form_address("http://192.168.1.10:5005/".into());
    save(window, &name("downloads-add-http"));
    // A seedbox over SFTP (the seventh round): the protocol chips, the port, the way to sign in;
    // this version says it cannot connect.
    downloads.set_form_protocol("sftp".into());
    downloads.set_form_name("Seedbox (SFTP)".into());
    downloads.set_form_address("sb-ams.example.net".into());
    downloads.set_form_username("zhouye".into());
    save(window, &name("downloads-add-sftp"));
    downloads.set_form_protocol("webdav".into());
    downloads.set_form_editing(true);
    downloads.set_form_name("Home NAS".into());
    downloads.set_form_address("https://nas.home/dav".into());
    downloads.set_form_port("5006".into());
    downloads.set_form_username("yumin".into());
    downloads.set_form_password("correct horse".into());
    downloads.set_test_state(TestState::Ok);
    downloads.set_test_folders(12);
    downloads.set_test_files(3);
    downloads.set_test_ranges(true);
    downloads.set_test_probed(true);
    save(window, &name("downloads-edit"));
    downloads.set_test_state(TestState::Failed);
    downloads.set_test_problem(Problem::Password);
    downloads.set_test_detail(SharedString::new());
    save(window, &name("downloads-edit-refused"));
    downloads.set_test_state(TestState::None);
    downloads.set_form_open(false);
    // No server yet.
    downloads.set_servers(model(Vec::new()));
    downloads.set_current(-1);
    downloads.set_queue(model(Vec::new()));
    downloads.set_active_count(0);
    save(window, &name("downloads-none"));
    // Back to the page as it was, but with nothing on its way: the chat windows' pictures that
    // follow show the download icon without its dot.
    fill_downloads(ui);
    downloads.set_active_count(0);
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
            set_toolbox(&ui);
            open_keyboards(&ui);
            save(&window, &name("chats"));
            // Searching in the chat: the line under the header, at the third of 14 matches.
            {
                let conversation = ui.global::<Conversation>();
                conversation.set_search_open(true);
                conversation.set_search_count(14);
                conversation.set_search_at(3);
                save(&window, &name("chat-search"));
                conversation.set_search_open(false);
                conversation.set_search_count(-1);
                conversation.set_search_at(0);
            }
            // The chat's info panel beside the messages (the chat-info round), for the group.
            {
                let info = ui.global::<ChatInfo>();
                info.set_quick(model(vec![info_row("q-search", InfoRowKind::Action, "magnifying-glass", "Search", "s")]));
                info.set_sections(model(info_sections()));
                info.set_cursor("notif".into());
                info.set_open(true);
                save(&window, &name("chat-info"));
                info.set_open(false);
                info.set_cursor(SharedString::new());
            }
            // The toolbox from the logo (the seventh round), with two downloads on their way:
            // every category; the transfers alone, with the protocols under Downloads; a search
            // that finds a tool still to come; the accounts (Terminal's /accounts).
            {
                let toolbox = ui.global::<Toolbox>();
                fill_downloads(&ui);
                ui.global::<Downloads>().set_active_count(2);
                set_toolbox(&ui);
                toolbox.set_open(true);
                save(&window, &name("toolbox"));
                crate::toolbox::show_for_pictures(&ui, &["downloads"], "transfers", "");
                save(&window, &name("toolbox-transfers"));
                crate::toolbox::show_for_pictures(&ui, &["downloads"], "all", "live");
                save(&window, &name("toolbox-search"));
                crate::toolbox::show_for_pictures(&ui, &["downloads"], "accounts", "");
                save(&window, &name("toolbox-accounts"));
                crate::toolbox::show_for_pictures(&ui, &["downloads"], "bar", "");
                save(&window, &name("toolbox-bar"));
                toolbox.set_open(false);
                ui.global::<Downloads>().set_active_count(0);
                set_toolbox(&ui);
            }
            // A right click on the first chat of the list: its menu.
            let (x, y) = match theme {
                Theme::Workbench => (150.0, 166.0),
                Theme::Broadsheet => (230.0, 310.0),
                Theme::Terminal => (140.0, 110.0),
            };
            right_click(&window, x, y);
            save(&window, &name("chat-menu"));
            // Its third entry, Add to folder: the menu's second page.
            click(&window, x + 40.0, y + add_to_folder(theme));
            save(&window, &name("chat-menu-folders"));
            escape(&window);
            // The menu of a chat with a person (Lin Xia, under the Personal folder's header in
            // Workbench): blocking, reporting, deleting.
            let (ux, uy) = match theme {
                Theme::Workbench => (150.0, 255.0),
                Theme::Broadsheet => (230.0, 443.0),
                Theme::Terminal => (140.0, 170.0),
            };
            right_click(&window, ux, uy);
            save(&window, &name("chat-menu-user"));
            escape(&window);
            // A right click on the Personal folder's tab (its header in Workbench's tree): the
            // folder's menu.
            let (fx, fy) = match theme {
                Theme::Workbench => (110.0, 211.0),
                Theme::Broadsheet => (170.0, 252.0),
                Theme::Terminal => (97.0, 83.0),
            };
            right_click(&window, fx, fy);
            save(&window, &name("folder-menu"));
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
            // The question before a link whose words are not its address opens.
            chats.set_confirm_title("evil.example".into());
            chats.set_confirm_link("https://evil.example/login?account=lin.xia&next=%2Fwallet".into());
            chats.set_confirm_revoke_choice(false);
            chats.set_question(ChatConfirm::Link);
            save(&window, &name("chat-link"));
            chats.set_question(ChatConfirm::None);
            // Workbench's tabs: a right click on the second one, Tech Morning.
            if theme == Theme::Workbench {
                right_click(&window, 520.0, 59.0);
                save(&window, &name("tab-menu"));
                escape(&window);
            }
            message_actions(&ui, &window, &name);
            bots(&ui, &window, &name);
            extras(&ui, &window, &name);
            compose(&ui, &window, theme, &name);
            attachments(&ui, &window, theme, &name);
            screenshot_tool(&ui, &window, &overlay, &overlay_window, &name);
            open_viewer(&ui, 1);
            save(&window, &name("viewer-photo"));
            open_viewer(&ui, 2);
            save(&window, &name("viewer-video"));
            // The volume tray out, as after ↑ ↓: at 60, then with the sound off. It comes out over
            // 160 ms: it starts now, and is out when the next picture is taken, a second later.
            {
                let viewer = ui.global::<Viewer>();
                viewer.set_volume(60);
                viewer.set_volume_shown(true);
                slint::platform::update_timers_and_animations();
                save(&window, &name("viewer-volume"));
                viewer.set_muted(true);
                save(&window, &name("viewer-volume-muted"));
            }
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
            ai_access(&ui, &window, &name);
            app.set_page(Page::Profile);
            save(&window, &name("profile"));
            downloads(&ui, &window, theme, &name);
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
            right_click(&window, 150.0, 166.0);
            click(&window, 190.0, 166.0 + add_to_folder(theme));
            save(&window, "zh-workbench-chat-menu-folders");
            escape(&window);
        }
        message_actions(&ui, &window, &|page: &str| format!("zh-{theme_name}-{page}"));
        compose(&ui, &window, theme, &|page: &str| format!("zh-{theme_name}-{page}"));
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
    ai_access(&ui, &window, &|page: &str| format!("zh-workbench-{page}"));
    fill_downloads(&ui);
    app.set_page(Page::Downloads);
    save(&window, "zh-workbench-downloads");
    app.set_page(Page::Chats);

    // The other languages, light, Workbench: the chat window and Settings → General, every script
    // set in the bundled fonts.
    for code in ["zh_Hant", "ja", "ko", "fr", "de", "es", "pt", "ru", "ar"] {
        slint::select_bundled_translation(code).expect("every language is bundled");
        app.set_language(code.into());
        app.set_telegram_state(TelegramState::Ready);
        app.set_page(Page::Chats);
        open_keyboards(&ui);
        save(&window, &format!("{code}-workbench-chats"));
        app.set_page(Page::Settings);
        app.set_settings_section(SettingsSection::General);
        save(&window, &format!("{code}-workbench-settings-general"));
        app.set_page(Page::Chats);
    }
    let _ = login.get_countries().row_count();
}
