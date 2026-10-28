//! The platform layer (docs/architecture.md): the few things the shell does differently on each
//! operating system. Everything else stays free of platform code.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod microphone;

use slint::ComponentHandle;

use crate::{MainWindow, PlatformWords};

/// Show the window and run until FinchGram quits. On macOS closing the window only hides it, as
/// the design's "When closing the window: Minimize to tray" (the default) has it: FinchGram stays
/// in the Dock, a click on the Dock icon shows the window again, and Quit (⌘Q, the Dock's menu)
/// runs `before_quit` first. Elsewhere there is no tray icon yet to come back from, so closing the
/// window quits.
pub fn run(ui: &MainWindow, before_quit: fn()) -> Result<(), slint::PlatformError> {
    ui.show()?;
    #[cfg(target_os = "macos")]
    {
        macos::install(ui, before_quit);
        slint::run_event_loop_until_quit()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = before_quit;
        slint::run_event_loop()
    }
}

/// Bring the window back: shown, not minimized, in front.
pub fn show_window(ui: &MainWindow) {
    ui.window().set_minimized(false);
    if let Err(err) = ui.show() {
        eprintln!("platform: cannot show the window: {err}");
    }
    #[cfg(target_os = "macos")]
    macos::bring_to_front();
}

/// FinchGram's icon in the menu bar (macOS), with a menu to open the window or quit; or none. Its
/// words are the UI language's: after a change of language it is made again.
pub fn set_menu_bar_icon(ui: &MainWindow, shown: bool) {
    #[cfg(target_os = "macos")]
    {
        let words = ui.global::<PlatformWords>();
        macos::set_menu_bar_icon(shown, &words.get_open(), &words.get_quit());
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (ui, shown);
}

/// Whether FinchGram opens when the user logs in: None where the system has no such list for apps
/// (before macOS 13, and elsewhere so far).
pub fn launch_at_login() -> Option<bool> {
    #[cfg(target_os = "macos")]
    return macos::launch_at_login();
    #[cfg(not(target_os = "macos"))]
    None
}

pub fn set_launch_at_login(on: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return macos::set_launch_at_login(on);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = on;
        Err("launching at login is not built for this system".into())
    }
}

/// The order of names in a language, for the chat list sorted by name: letters as the language
/// sorts them, Chinese by pinyin, digits by their value, capitals with their small letters (macOS's
/// own comparison, in the UI language). Elsewhere, small letters by their code points.
pub struct NameOrder {
    #[cfg(target_os = "macos")]
    inner: macos::NameOrder,
}

impl NameOrder {
    /// `language` is a UI language code as lang/ names them ("en", "zh_Hans").
    pub fn new(language: &str) -> NameOrder {
        #[cfg(target_os = "macos")]
        return NameOrder { inner: macos::NameOrder::new(language) };
        #[cfg(not(target_os = "macos"))]
        {
            let _ = language;
            NameOrder {}
        }
    }

    pub fn compare(&self, a: &str, b: &str) -> std::cmp::Ordering {
        #[cfg(target_os = "macos")]
        return self.inner.compare(a, b);
        #[cfg(not(target_os = "macos"))]
        a.to_lowercase().cmp(&b.to_lowercase())
    }
}

/// Whether the window's contents may be captured (screenshots, screen recording, other apps).
/// Off while a chat whose content Telegram protects is open, as Telegram's own apps do it; macOS
/// then shows the window blank in captures.
pub fn set_window_capturable(window: &slint::Window, capturable: bool) {
    #[cfg(target_os = "macos")]
    macos::set_window_capturable(window, capturable);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, capturable);
    }
}

/// Put `text` on the clipboard.
pub fn copy_text(text: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return macos::copy_text(text);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = text;
        Err("copying is not built for this system".into())
    }
}

/// Put a picture on the clipboard: the contents of an image file (JPEG, PNG, WebP).
pub fn copy_image(contents: &[u8]) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return macos::copy_image(contents);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = contents;
        Err("copying pictures is not built for this system".into())
    }
}

/// Show the system's open panel for files (`media`: pictures and videos only), and hand `chosen`
/// the files picked, on the UI thread once the panel closes; none when it was cancelled.
pub fn choose_files(media: bool, chosen: impl FnOnce(Vec<std::path::PathBuf>) + 'static) {
    #[cfg(target_os = "macos")]
    macos::choose_files(media, chosen);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = media;
        chosen(Vec::new());
    }
}

/// The open panel for one picture, of any kind the system reads (HEIC, TIFF, GIF, … besides JPEG,
/// PNG and WebP): a chat's photo. `chosen` runs on the UI thread once the panel closes, with
/// nothing when it was cancelled.
pub fn choose_picture(chosen: impl FnOnce(Option<std::path::PathBuf>) + 'static) {
    #[cfg(target_os = "macos")]
    macos::choose_picture(chosen);
    #[cfg(not(target_os = "macos"))]
    chosen(None);
}

/// A picture the image crate cannot read (HEIC, TIFF, GIF, AVIF, …), read by the system: turned
/// the way it was taken, no larger than `largest` pixels on its longer side.
pub fn read_picture(path: &std::path::Path, largest: u32) -> Option<image::RgbaImage> {
    #[cfg(target_os = "macos")]
    return macos::read_picture(path, largest);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, largest);
        None
    }
}

/// Show a file in the system's file manager (the Finder).
pub fn reveal_file(path: &str) {
    #[cfg(target_os = "macos")]
    macos::reveal_file(path);
    #[cfg(not(target_os = "macos"))]
    let _ = path;
}

/// The files on the clipboard (copied in the Finder), if any.
pub fn pasteboard_files() -> Vec<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    return macos::pasteboard_files();
    #[cfg(not(target_os = "macos"))]
    Vec::new()
}

/// The picture on the clipboard as PNG, if there is one.
pub fn pasteboard_image() -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    return macos::pasteboard_image();
    #[cfg(not(target_os = "macos"))]
    None
}

/// Whether the system lets FinchGram capture the screen (macOS: Screen Recording, in System
/// Settings → Privacy & Security). Systems without the screenshot tool say no.
pub fn screen_capture_allowed() -> bool {
    #[cfg(target_os = "macos")]
    return macos::screen_capture_allowed();
    #[cfg(not(target_os = "macos"))]
    false
}

/// Ask the system for leave to capture the screen: macOS shows its dialog once.
pub fn request_screen_capture() {
    #[cfg(target_os = "macos")]
    macos::request_screen_capture();
}

/// Open the system's settings where screen capture is allowed.
pub fn open_screen_capture_settings() {
    #[cfg(target_os = "macos")]
    macos::open_screen_capture_settings();
}

/// Whether FinchGram may listen to the microphone, for voice input (src/speech/).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrophoneAccess {
    Allowed,
    /// The user said no, or the Mac's settings let nobody ask.
    Denied,
    /// Nobody has asked yet: the system asks the user, once.
    Undetermined,
}

pub fn microphone_access() -> MicrophoneAccess {
    #[cfg(target_os = "macos")]
    return microphone::access();
    #[cfg(not(target_os = "macos"))]
    MicrophoneAccess::Denied
}

/// Ask the system for leave to listen: macOS asks the user once. `answered` gets the answer, on a
/// thread of the system's.
pub fn ask_for_microphone(answered: impl FnOnce(bool) + Send + 'static) {
    #[cfg(target_os = "macos")]
    microphone::ask(answered);
    #[cfg(not(target_os = "macos"))]
    answered(false);
}

/// Open the system's settings where the microphone is allowed.
pub fn open_microphone_settings() {
    #[cfg(target_os = "macos")]
    microphone::open_settings();
}

/// A voice message being recorded into an M4A file (src/telegram/voice.rs); dropped without
/// `finish`, the recording stops and the file goes.
pub struct VoiceRecorder {
    #[cfg(target_os = "macos")]
    inner: microphone::Recorder,
}

impl VoiceRecorder {
    pub fn start(path: &std::path::Path) -> Result<VoiceRecorder, String> {
        #[cfg(target_os = "macos")]
        return microphone::Recorder::start(path).map(|inner| VoiceRecorder { inner });
        #[cfg(not(target_os = "macos"))]
        {
            let _ = path;
            Err("voice messages are not built for this system yet".into())
        }
    }

    /// How loud the microphone is now, 0 … 1.
    pub fn level(&self) -> f32 {
        #[cfg(target_os = "macos")]
        return self.inner.level();
        #[cfg(not(target_os = "macos"))]
        0.0
    }

    /// The recording stops and its file is complete; how long it is, in seconds.
    pub fn finish(self) -> f64 {
        #[cfg(target_os = "macos")]
        return self.inner.finish();
        #[cfg(not(target_os = "macos"))]
        0.0
    }
}

/// The microphone, listening for as long as this is kept (voice input, src/speech/).
pub struct Microphone {
    #[cfg(target_os = "macos")]
    _inner: microphone::Microphone,
}

impl Microphone {
    /// Listen to the default input: `deliver` gets the sound, one channel at the rate it is given
    /// with, on a thread of the system's.
    pub fn start(deliver: impl Fn(&[f32], f64) + Send + Sync + 'static) -> Result<Microphone, String> {
        #[cfg(target_os = "macos")]
        return microphone::Microphone::start(deliver).map(|inner| Microphone { _inner: inner });
        #[cfg(not(target_os = "macos"))]
        {
            let _ = deliver;
            Err("voice input is not built for this system yet".into())
        }
    }
}

/// Capture one display (counted from 1, as the system counts them) into a picture of `format`
/// ("bmp" or "png") at `path`; false when the system refused.
pub fn capture_display(number: usize, path: &std::path::Path, format: &str) -> bool {
    #[cfg(target_os = "macos")]
    return macos::capture_display(number, path, format);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (number, path, format);
        false
    }
}

/// A window on the screen, as the system lists them: where it is, in points of the global
/// coordinate space (the main display's top left corner is its origin), front to back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenWindow {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// The windows on the screen, front to back, for the screenshot tool to offer: FinchGram's own
/// among them, unless it hid for the capture.
pub fn windows_on_screen() -> Vec<ScreenWindow> {
    #[cfg(target_os = "macos")]
    return macos::windows_on_screen();
    #[cfg(not(target_os = "macos"))]
    Vec::new()
}

/// Before the first overlay of the screenshot tool is made: what the system has to be told about
/// overlay windows, once; `window` is any window of FinchGram's.
pub fn prepare_overlays(window: &slint::Window) {
    #[cfg(target_os = "macos")]
    macos::prepare_overlays(window);
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

/// Put the screenshot tool's overlay above everything else on its display, and give it the
/// keyboard.
pub fn raise_overlay(window: &slint::Window) {
    #[cfg(target_os = "macos")]
    macos::raise_overlay(window);
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

/// A notification of a new message, as the system shows it (on macOS in Notification Center).
/// Systems without notifications in FinchGram yet show none, and have no badge either.
pub struct Notification {
    /// Its name, by which it is taken back.
    pub id: String,
    /// The account it is for (src/telegram/mod.rs's slot), and the chat it is from: the system
    /// keeps a chat's notifications together, and a click on one opens the chat, in its account.
    pub account: u32,
    pub chat_id: i64,
    pub title: String,
    /// Who wrote it, in a group; else empty.
    pub subtitle: String,
    pub body: String,
    /// It plays the system's sound.
    pub sound: bool,
}

/// Clicks on FinchGram's notifications show the window and hand their account and chat to
/// `open_chat`. Call before the event loop runs, so that the click that started FinchGram counts
/// too.
pub fn handle_notification_clicks(open_chat: fn(u32, i64)) {
    #[cfg(target_os = "macos")]
    macos::handle_notification_clicks(open_chat);
    #[cfg(not(target_os = "macos"))]
    let _ = open_chat;
}

/// Ask the user whether FinchGram may show notifications. The system asks only once and remembers
/// the answer, which the user can change in its settings.
pub fn ask_to_notify() {
    #[cfg(target_os = "macos")]
    macos::ask_to_notify();
}

pub fn show_notification(notification: &Notification) {
    #[cfg(target_os = "macos")]
    macos::show_notification(notification);
    #[cfg(not(target_os = "macos"))]
    let _ = notification;
}

/// Take back the notifications with these ids, where they are still shown.
pub fn remove_notifications(ids: &[String]) {
    #[cfg(target_os = "macos")]
    macos::remove_notifications(ids);
    #[cfg(not(target_os = "macos"))]
    let _ = ids;
}

/// Take back every notification shown so far whose id begins with `prefix` (one account's) but
/// those with these ids.
pub fn keep_only_notifications(prefix: &str, ids: Vec<String>) {
    #[cfg(target_os = "macos")]
    macos::keep_only_notifications(prefix, ids);
    #[cfg(not(target_os = "macos"))]
    let _ = (prefix, ids);
}


/// The number on FinchGram's icon (the Dock's on macOS); none for 0.
pub fn set_badge(count: i32) {
    #[cfg(target_os = "macos")]
    macos::set_badge(count);
    #[cfg(not(target_os = "macos"))]
    let _ = count;
}

/// Draw the eye to FinchGram's icon: on macOS the Dock icon bounces once. Nothing happens while
/// FinchGram is in front.
pub fn flash_icon() {
    #[cfg(target_os = "macos")]
    macos::flash_icon();
}

/// What the account's list of sessions calls this device (TDLib's device_model, which must not be
/// empty). TDLib finds the system's version by itself.
pub fn device_model() -> &'static str {
    if cfg!(target_os = "macos") { "Mac" } else { "PC" }
}

/// Select Slint's backend, with the window the design draws: on macOS the title bar is
/// transparent and the pages run underneath it, the window buttons over each theme's own title
/// bar.
pub fn select_backend() -> Result<(), slint::PlatformError> {
    slint::BackendSelector::new()
        .with_winit_window_attributes_hook(|attributes| {
            #[cfg(target_os = "macos")]
            let attributes = {
                use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
                attributes.with_titlebar_transparent(true).with_title_hidden(true).with_fullsize_content_view(true)
            };
            attributes
        })
        .select()
}

/// Open a web, mail or Telegram link with the system (the browser, the mail app, or whatever
/// handles tg: links).
/// Anything else, a file or another kind of link, is refused.
pub fn open_link(url: &str) {
    let allowed = ["https://", "http://", "tg://", "mailto:"].iter().any(|scheme| url.starts_with(scheme));
    if !allowed {
        eprintln!("platform: not opening {url:?}: only web and Telegram links are opened");
        return;
    }
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("/usr/bin/open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(url).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(err) = result {
        eprintln!("platform: cannot open {url}: {err}");
    }
}

/// Show the system's open panel for one folder, and hand `chosen` the folder picked, on the UI
/// thread once the panel closes; none when it was cancelled.
pub fn choose_folder(chosen: impl FnOnce(Option<std::path::PathBuf>) + 'static) {
    #[cfg(target_os = "macos")]
    macos::choose_folder(chosen);
    #[cfg(not(target_os = "macos"))]
    chosen(None);
}

/// Keep a password in the system's keychain (the Keychain on macOS), as `service`'s password for
/// `account`; one that is there already is replaced. The system may ask the user the first time
/// FinchGram reads it back.
pub fn keychain_store(service: &str, account: &str, password: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return macos::keychain_store(service, account, password);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (service, account, password);
        Err("the keychain is not built for this system".into())
    }
}

/// The password kept for `account` under `service`; none when there is none.
pub fn keychain_read(service: &str, account: &str) -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    return macos::keychain_read(service, account);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (service, account);
        Err("the keychain is not built for this system".into())
    }
}

/// The service every Keychain item of FinchGram's is under; the account names what the item is
/// ("tdlib", "database", …).
pub const KEYCHAIN_SERVICE: &str = "FinchGram";

/// Mark the file at `path` as a download from the internet, from `source` when known: macOS's
/// quarantine (the Finder says where a file came from, and Gatekeeper looks at what is opened),
/// as a browser marks what it saves. Nothing happens on other systems, or when the system
/// refuses, which is said on stderr.
pub fn mark_downloaded(path: &std::path::Path, source: Option<&str>) {
    #[cfg(target_os = "macos")]
    macos::mark_downloaded(path, source);
    #[cfg(not(target_os = "macos"))]
    let _ = (path, source);
}

/// 32 random bytes from the system, as base64: a key for a database.
pub fn random_key() -> Option<String> {
    use base64::Engine as _;
    let mut bytes = [0u8; 32];
    let mut random = std::fs::File::open("/dev/urandom").ok()?;
    std::io::Read::read_exact(&mut random, &mut bytes).ok()?;
    Some(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// How many bytes the volume holding `path` has free for this user; none when the system would
/// not say, and then nothing is refused for want of room.
pub fn free_space(path: &std::path::Path) -> Option<u64> {
    #[cfg(target_os = "macos")]
    return macos::free_space(path);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        None
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn the_root_volume_has_some_room() {
        let free = super::free_space(std::path::Path::new("/"));
        if cfg!(target_os = "macos") {
            let free = free.expect("the system says how much room there is");
            assert!(free > 0 && free < 1 << 50, "{free}");
        }
    }

    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn a_download_is_marked_as_one_with_where_it_came_from() {
        let folder = std::env::temp_dir().join(format!("finchgram-quarantine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a folder");
        let file = folder.join("film.mkv");
        std::fs::write(&file, b"not a film").expect("a file");
        mark_downloaded(&file, Some("https://nas.home/dav/films/film.mkv"));
        // "0281;6adc7624;;UUID": the flags in hex, the time, the agent's name (empty for a bare
        // program, the test's; "FinchGram" for the bundle), the event's id. Bit 0 of the flags is
        // what makes the file a download to Gatekeeper.
        let flags = |file: &std::path::Path| -> u32 {
            let shown = std::process::Command::new("/usr/bin/xattr").arg("-p").arg("com.apple.quarantine").arg(file).output().expect("xattr");
            let value = String::from_utf8_lossy(&shown.stdout).trim().to_string();
            assert!(shown.status.success() && value.split(';').count() == 4, "{value}");
            u32::from_str_radix(value.split(';').next().unwrap_or_default(), 16).expect("the flags")
        };
        assert_eq!(flags(&file) & 1, 1);
        // Without an address, as a file saved from a chat is: marked all the same.
        let other = folder.join("photo.jpg");
        std::fs::write(&other, b"not a photo").expect("a file");
        mark_downloaded(&other, None);
        assert_eq!(flags(&other) & 1, 1);
        // A file that is not there: said on stderr, nothing more.
        mark_downloaded(&folder.join("missing.bin"), None);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn names_follow_the_language_not_the_code_points() {
        let english = NameOrder::new("en");
        assert_eq!(english.compare("alice", "Bob"), Ordering::Less);
        assert_eq!(english.compare("Chat 9", "Chat 10"), Ordering::Less);
        assert_eq!(english.compare("Émile", "Eve"), Ordering::Less);
        // Pinyin: 阿 (a) before 波 (bo), 李 (li) before 张 (zhang), whatever their code points.
        let chinese = NameOrder::new("zh_Hans");
        assert_eq!(chinese.compare("阿里", "波波"), Ordering::Less);
        assert_eq!(chinese.compare("张三", "李四"), Ordering::Greater);
    }
}

/// The system has been told that the app is busy on the user's behalf, until this is dropped: on
/// macOS an NSProcessInfo activity, which keeps App Nap from slowing the app down when its window
/// is not in front, and the Mac from going to sleep on its own meanwhile.
pub struct Activity {
    #[cfg(target_os = "macos")]
    token: Option<macos::ActivityToken>,
}

/// Tell the system the app is busy, for `reason` (a short English phrase, for its tools).
pub fn begin_activity(reason: &str) -> Activity {
    #[cfg(target_os = "macos")]
    return Activity { token: Some(macos::begin_activity(reason)) };
    #[cfg(not(target_os = "macos"))]
    {
        let _ = reason;
        Activity {}
    }
}

impl Drop for Activity {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some(token) = self.token.take() {
            macos::end_activity(token);
        }
    }
}
