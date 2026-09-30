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

impl Platform for Screenshots {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
}

/// Draw the window as it is now into `name`.png. Without a native menu bar, Slint draws the menu
/// bar at the top of the window: the window is that much taller, and the picture leaves it out.
fn save(window: &MinimalSoftwareWindow, name: &str) {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("screenshots");
    std::fs::create_dir_all(&directory).expect("target/screenshots");
    let mut pixels = vec![slint::Rgb8Pixel::default(); (WIDTH * (HEIGHT + MENU_BAR)) as usize];
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

/// A right click at `x`, `y` in the picture, where the pointer then stays.
fn right_click(window: &MinimalSoftwareWindow, x: f32, y: f32) {
    let position = slint::LogicalPosition::new(x, y + MENU_BAR as f32);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Right });
    window.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Right });
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
    }
}

fn message(id: &str, sender: &str, time: &str, text: &str) -> MessageRow {
    MessageRow {
        kind: RowKind::Message,
        id: id.into(),
        outgoing: sender == "Zhou Ye",
        sender: sender.into(),
        sender_initial: sender.chars().next().map(String::from).unwrap_or_default().into(),
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
    }
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
        time: moment(Day::Today, 14, 19),
        caption: "The lubed switch comparison is in the group album, have a look.".into(),
    };
    let wide = Media { id: "8".into(), duration: 95, name: "screen-recording.mp4".into(), ..media("8", 1280, 544, 200.0, true) };
    viewer.set_items(model(album().into_iter().chain(std::iter::once(wide)).map(item).collect()));
    viewer.set_index(index);
    viewer.set_zoom(0);
    viewer.set_playing(false);
    viewer.set_position(0.0);
    viewer.set_open(true);
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
    account.set_username("zhouye".into());
    account.set_phone("+86 138 0013 2046".into());
    account.set_bio("Design, and a little code. Shanghai.".into());

    let keyboards = ChatRow {
        pinned: true,
        ..chat("keyboards", "Keyboard Lab", ChatKind::Group, moment(Day::Today, 14, 20), "Jie", "@Zhou Ye do you still sell the dark keycaps?", 42, 486)
    };
    let alex = chat("alex", "Alex Chen", ChatKind::User, moment(Day::Today, 12, 8), "", "Can we move the call to Thursday?", 1, 0);
    let linxia = chat("linxia", "Lin Xia", ChatKind::User, moment(Day::Today, 13, 52), "", "Is that place open tonight?", 2, 0);
    let news = chat("news", "Tech Morning", ChatKind::Channel, moment(Day::Today, 7, 30), "", "New chip export rules take effect", 12, 82413);
    let mom = chat("mom", "Mom", ChatKind::User, moment(Day::Yesterday, 18, 32), "", "Never mind, if you are busy don't reply", 0, 0);
    let finch = chat("finch", "FinchGram Updates", ChatKind::Channel, moment(Day::Yesterday, 20, 0), "", "FinchGram 0.1: the design arrives", 0, 41000);
    let books = chat("books", "Wednesday Book Club", ChatKind::Group, moment(Day::ThisWeek, 20, 15), "Chen", "Next up: Invisible Cities", 0, 9);
    let saved = ChatRow { kind: ChatKind::Saved, ..chat("saved", "Saved Messages", ChatKind::Saved, moment(Day::ThisYear, 22, 42), "", "Kyoto, check-in October 3", 0, 0) };
    let personal = vec![keyboards.clone(), linxia.clone(), mom.clone(), books.clone(), saved.clone()];
    let work = vec![alex.clone()];
    let channels = vec![news.clone(), finch.clone()];
    let all = vec![keyboards.clone(), linxia.clone(), alex.clone(), news.clone(), mom.clone(), finch.clone(), books.clone(), saved.clone()];

    let chats = ui.global::<Chats>();
    let folder = |id: i32, name: &str, unread: i32| Folder { id, name: name.into(), unread };
    chats.set_folders(model(vec![folder(0, "All chats", 57), folder(1, "Personal", 44), folder(2, "Work", 1), folder(3, "Channels", 12)]));
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
    chats.set_bots(model(vec![bot("groupbuy", "Group-buy Helper", "keeb_gb_bot", false)]));
    chats.set_loaded(true);

    let conversation = ui.global::<Conversation>();
    let tab = |chat: &ChatRow| Tab { id: chat.id.clone(), title: chat.title.clone(), kind: chat.kind };
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
    conversation.set_messages(model(vec![
        day_row(Day::Today),
        message("1", "Mi", "09:12", "Morning! This week's group-buy keycaps arrived; the delivery list is in the pinned post."),
        message("2", "Jie", "10:30", "I'm lubing my 65% today, photos later."),
        message("3", "Zhou Ye", "10:31", "Nice, @ me when it's done."),
        MessageRow { content: Content::Sticker, detail: "👍".into(), sticker: sticker(), ..message("3a", "Zhou Ye", "10:31", "") },
        message("4", "Mika", "11:02", "I can make the meetup on Thursday, but only after 3pm."),
        message("5", "Mi", "11:05", "Thursday 15:00 at the usual place. Who's coming?"),
        MessageRow {
            content: Content::Photo,
            media: model(album()),
            ..message("6", "Jie", "14:19", "The lubed switch comparison is in the group album, have a look.")
        },
        message("7", "Jie", "14:20", "@Zhou Ye do you still sell the dark keycaps?"),
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
        post("1", "07:00", "New chip export rules take effect\n\nApprovals for advanced equipment are now split into three tiers by type and end use, with a clear upper limit on how long a review may take. Most orders already in transit are unaffected."),
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
                Theme::Broadsheet => (230.0, 265.0),
                Theme::Terminal => (140.0, 98.0),
            };
            right_click(&window, x, y);
            save(&window, &name("chat-menu"));
            escape(&window);
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
                [(SettingsSection::General, "general"), (SettingsSection::Appearance, "appearance"), (SettingsSection::Language, "language"), (SettingsSection::About, "about")]
            {
                app.set_settings_section(section);
                save(&window, &name(&format!("settings-{section_name}")));
            }
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
