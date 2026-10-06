//! macOS: FinchGram stays in the Dock when its window is closed, can have an icon in the menu bar,
//! can be one of the user's login items, copies to the clipboard, and shows notifications, the
//! unread count on its Dock icon, and a bounce of it.
//!
//! winit 0.30 owns the application's delegate and has nothing for a click on the Dock icon, for
//! Quit or for our own menu, so methods are added to its delegate class at run time, the way Slint
//! adds one of its own (i-slint-backend-winit's disable_macos_automatic_shortcut_localization).
//!
//! Notifications go through the system's UserNotifications framework, which serves apps only:
//! FinchGram run by `cargo run` has no bundle, and so no notifications.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ffi::CStr;
use std::ptr::NonNull;
use std::sync::OnceLock;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSImage, NSMenu, NSMenuItem, NSPasteboard, NSPasteboardTypeString, NSPasteboardTypeTIFF,
    NSRequestUserAttentionType, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSArray, NSBundle, NSData, NSDate, NSError, NSSize, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationDefaultActionIdentifier,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse, UNNotificationSound,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use slint::ComponentHandle;

use crate::MainWindow;

/// NSApplicationTerminateReply's NSTerminateNow.
const TERMINATE_NOW: usize = 1;
/// SMAppServiceStatus: in the list of login items, and in it but waiting for the user's approval.
const LOGIN_ITEM_ENABLED: isize = 1;
const LOGIN_ITEM_REQUIRES_APPROVAL: isize = 2;
/// The logo's mark as a silhouette, which its rules ask for below 20 px (ui/logo/README.md).
const MARK: &[u8] = include_bytes!("../../ui/logo/svg/FinchGram-mark-16.svg");

// SMAppService (macOS 13 and later) lives in ServiceManagement.
#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

thread_local! {
    static WINDOW: RefCell<Option<slint::Weak<MainWindow>>> = const { RefCell::new(None) };
    static BEFORE_QUIT: Cell<Option<fn()>> = const { Cell::new(None) };
    static STATUS_ITEM: RefCell<Option<Retained<NSStatusItem>>> = const { RefCell::new(None) };
    /// The notification centre only keeps a weak reference to its delegate.
    static NOTIFICATION_DELEGATE: RefCell<Option<Retained<NotificationDelegate>>> = const { RefCell::new(None) };
}

/// What a click on a notification opens (`handle_notification_clicks`).
static OPEN_CHAT: OnceLock<fn(i64)> = OnceLock::new();

pub fn install(ui: &MainWindow, before_quit: fn()) {
    WINDOW.with(|window| *window.borrow_mut() = Some(ui.as_weak()));
    BEFORE_QUIT.with(|hook| hook.set(Some(before_quit)));
    let Some(class) = AnyClass::get(c"WinitApplicationDelegate") else {
        eprintln!("platform: winit's application delegate is missing: the Dock icon cannot show the window again");
        return;
    };
    // SAFETY: each function has the Objective-C method's signature, which its type encoding gives.
    unsafe {
        add_method(
            class,
            sel!(applicationShouldHandleReopen:hasVisibleWindows:),
            std::mem::transmute::<unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, Bool) -> Bool, Imp>(reopen),
            c"B@:@B",
        );
        add_method(
            class,
            sel!(applicationShouldTerminate:),
            std::mem::transmute::<unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize, Imp>(should_terminate),
            c"Q@:@",
        );
        add_method(
            class,
            sel!(finchgramOpenWindow:),
            std::mem::transmute::<unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject), Imp>(open_window),
            c"v@:@",
        );
    }
}

/// Add a method to `class`, unless it has one by that name already.
///
/// # Safety
/// `implementation` must have the signature `types` describes.
unsafe fn add_method(class: &AnyClass, name: Sel, implementation: Imp, types: &CStr) {
    if class.instance_method(name).is_none() {
        // SAFETY: the caller vouches for the implementation and its type encoding.
        unsafe { objc2::ffi::class_addMethod((class as *const AnyClass).cast_mut(), name, implementation, types.as_ptr()) };
    }
}

/// A click on the Dock icon, or FinchGram opened again, while its window is closed: show it.
unsafe extern "C-unwind" fn reopen(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject, has_visible_windows: Bool) -> Bool {
    if !has_visible_windows.as_bool() {
        show_window_soon();
    }
    // Handled: AppKit has nothing more to do.
    Bool::NO
}

/// "Open FinchGram" in the menu of the icon in the menu bar. The menu item has no target, so AppKit
/// sends its action to the application's delegate.
unsafe extern "C-unwind" fn open_window(_this: *mut AnyObject, _cmd: Sel, _sender: *mut AnyObject) {
    show_window_soon();
}

/// Slint shows a window from inside its event loop, so the window is shown from there.
fn show_window_soon() {
    let _ = slint::invoke_from_event_loop(|| {
        if let Some(ui) = WINDOW.with(|window| window.borrow().as_ref().and_then(slint::Weak::upgrade)) {
            super::show_window(&ui);
        }
    });
}

/// Quit (⌘Q, the Dock's menu, the menu bar icon's menu, logging out of macOS): let TDLib close
/// first. AppKit ends the process right after, so nothing after the event loop runs.
unsafe extern "C-unwind" fn should_terminate(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject) -> usize {
    if let Some(before_quit) = BEFORE_QUIT.with(Cell::take) {
        before_quit();
    }
    TERMINATE_NOW
}

/// Make FinchGram the active app, as a click on its Dock icon does and a menu in the menu bar does
/// not.
pub fn bring_to_front() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    // activateIgnoringOtherApps: is deprecated from macOS 14 on for activate, which 12 and 13 lack.
    // SAFETY: the method takes a BOOL and returns nothing.
    unsafe {
        let _: () = msg_send![&app, activateIgnoringOtherApps: true];
    }
}

/// FinchGram's icon in the menu bar, with a menu to open its window or to quit; or none.
pub fn set_menu_bar_icon(shown: bool, open: &str, quit: &str) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    STATUS_ITEM.with(|item| {
        let mut item = item.borrow_mut();
        if let Some(old) = item.take() {
            NSStatusBar::systemStatusBar().removeStatusItem(&old);
        }
        if shown {
            *item = Some(status_item(mtm, open, quit));
        }
    });
}

fn status_item(mtm: MainThreadMarker, open: &str, quit: &str) -> Retained<NSStatusItem> {
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = item.button(mtm) {
        button.setImage(menu_bar_image().as_deref());
        button.setToolTip(Some(&NSString::from_str("FinchGram")));
    }
    let menu = NSMenu::new(mtm);
    let entry = |title: &str, action: Sel| {
        // SAFETY: the actions are the application's terminate: and its delegate's
        // finchgramOpenWindow: (added in `install`), both taking the sender.
        unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(title), Some(action), &NSString::new())
        }
    };
    menu.addItem(&entry(open, sel!(finchgramOpenWindow:)));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&entry(quit, sel!(terminate:)));
    item.setMenu(Some(&menu));
    item
}

/// The mark as a template image, which macOS colours to suit the menu bar: 16 pt high in 18 pt,
/// drawn at twice that for Retina screens.
fn menu_bar_image() -> Option<Retained<NSImage>> {
    let png = mark_png(36)?;
    let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&png))?;
    image.setSize(NSSize::new(18.0, 18.0));
    image.setTemplate(true);
    Some(image)
}

/// The mark on nothing, as a PNG `side` pixels square, the bird eight ninths of it at most.
fn mark_png(side: u32) -> Option<Vec<u8>> {
    let tree = resvg::usvg::Tree::from_data(MARK, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let room = side as f32 * 8.0 / 9.0;
    let scale = (room / size.width()).min(room / size.height());
    let (left, top) = ((side as f32 - size.width() * scale) / 2.0, (side as f32 - size.height() * scale) / 2.0);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(side, side)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, left, top), &mut pixmap.as_mut());
    // tiny-skia's pixels have their alpha premultiplied; a template image only uses the alpha.
    let pixels = image::RgbaImage::from_raw(side, side, pixmap.take())?;
    let mut png = Vec::new();
    pixels.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

/// The login item for the app itself (SMAppService, macOS 13 and later); None before.
fn main_app_service() -> Option<Retained<AnyObject>> {
    let class = AnyClass::get(c"SMAppService")?;
    // SAFETY: +[SMAppService mainAppService] takes nothing and returns an SMAppService.
    unsafe { msg_send![class, mainAppService] }
}

/// Whether FinchGram is one of the login items (or waits for the user's approval to be one).
pub fn launch_at_login() -> Option<bool> {
    let service = main_app_service()?;
    // SAFETY: -[SMAppService status] takes nothing and returns an NSInteger.
    let status: isize = unsafe { msg_send![&service, status] };
    Some(matches!(status, LOGIN_ITEM_ENABLED | LOGIN_ITEM_REQUIRES_APPROVAL))
}

pub fn set_launch_at_login(on: bool) -> Result<(), String> {
    let service = main_app_service().ok_or("launching at login needs macOS 13 or later")?;
    // SAFETY: both methods take an NSError out-parameter and return a BOOL.
    let result: Result<(), Retained<NSError>> = unsafe {
        if on { msg_send![&service, registerAndReturnError: _] } else { msg_send![&service, unregisterAndReturnError: _] }
    };
    result.map_err(|error| error.localizedDescription().to_string())
}

/// Put `text` on the clipboard.
pub fn copy_text(text: &str) -> Result<(), String> {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    // SAFETY: an AppKit constant, there for the life of the process.
    let kind = unsafe { NSPasteboardTypeString };
    if pasteboard.setString_forType(&NSString::from_str(text), kind) { Ok(()) } else { Err("the clipboard refused the text".into()) }
}

/// Put a picture on the clipboard: an image file's contents (JPEG, PNG, WebP), as TIFF, which
/// every app that pastes pictures reads.
pub fn copy_image(contents: &[u8]) -> Result<(), String> {
    let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(contents)).ok_or("not a picture macOS can read")?;
    let tiff = image.TIFFRepresentation().ok_or("the picture cannot be converted")?;
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    // SAFETY: an AppKit constant, there for the life of the process.
    let kind = unsafe { NSPasteboardTypeTIFF };
    if pasteboard.setData_forType(Some(&tiff), kind) { Ok(()) } else { Err("the clipboard refused the picture".into()) }
}

// ---- notifications --------------------------------------------------------------------------

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and NotificationDelegate does not implement
    // Drop.
    #[unsafe(super(NSObject))]
    #[name = "FinchGramNotificationDelegate"]
    struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        /// A notification while FinchGram is in front: shown all the same. Those of the chat the
        /// user is looking at never get here (src/telegram/notifications.rs).
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion_handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion_handler.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        /// A click on a notification: the window, with the notification's chat. The system may
        /// call this on a thread of its own.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive_response(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion_handler: &block2::DynBlock<dyn Fn()>,
        ) {
            // SAFETY: a constant of the framework, there for the life of the process.
            let clicked = response.actionIdentifier().isEqualToString(unsafe { UNNotificationDefaultActionIdentifier });
            let chat_id = response.notification().request().content().threadIdentifier().to_string().parse::<i64>();
            if clicked && let Ok(chat_id) = chat_id {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = WINDOW.with(|window| window.borrow().as_ref().and_then(slint::Weak::upgrade)) {
                        super::show_window(&ui);
                    }
                    if let Some(open_chat) = OPEN_CHAT.get() {
                        open_chat(chat_id);
                    }
                });
            }
            completion_handler.call(());
        }
    }
);

impl NotificationDelegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        // SAFETY: NSObject's init, which takes nothing.
        unsafe { msg_send![super(this), init] }
    }
}

/// The system's notification centre. Only an app has one: asked for without a bundle, it would
/// raise an exception.
fn notification_center() -> Option<Retained<UNUserNotificationCenter>> {
    NSBundle::mainBundle().bundleIdentifier()?;
    Some(UNUserNotificationCenter::currentNotificationCenter())
}

pub fn handle_notification_clicks(open_chat: fn(i64)) {
    let _ = OPEN_CHAT.set(open_chat);
    let Some(center) = notification_center() else { return };
    let delegate = NotificationDelegate::new();
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    NOTIFICATION_DELEGATE.with(|kept| *kept.borrow_mut() = Some(delegate));
}

pub fn ask_to_notify() {
    let Some(center) = notification_center() else { return };
    let answer = RcBlock::new(|allowed: Bool, error: *mut NSError| {
        if !allowed.as_bool() {
            // SAFETY: the system passes a valid error or none, for the length of the call.
            let reason = unsafe { error.as_ref() }.map(|error| format!(": {}", error.localizedDescription())).unwrap_or_default();
            eprintln!("platform: notifications are not allowed{reason}");
        }
    });
    let options = UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge;
    center.requestAuthorizationWithOptions_completionHandler(options, &answer);
}

pub fn show_notification(notification: &super::Notification) {
    let Some(center) = notification_center() else { return };
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&notification.title));
    content.setSubtitle(&NSString::from_str(&notification.subtitle));
    content.setBody(&NSString::from_str(&notification.body));
    content.setThreadIdentifier(&NSString::from_str(&notification.chat_id.to_string()));
    if notification.sound {
        content.setSound(Some(&UNNotificationSound::defaultSound()));
    }
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&notification.id), &content, None);
    let shown = RcBlock::new(|error: *mut NSError| {
        // SAFETY: the system passes a valid error or none, for the length of the call.
        if let Some(error) = unsafe { error.as_ref() } {
            eprintln!("platform: cannot show a notification: {}", error.localizedDescription());
        }
    });
    center.addNotificationRequest_withCompletionHandler(&request, Some(&shown));
}

pub fn remove_notifications(ids: &[String]) {
    let Some(center) = notification_center() else { return };
    let ids: Vec<Retained<NSString>> = ids.iter().map(|id| NSString::from_str(id)).collect();
    center.removeDeliveredNotificationsWithIdentifiers(&NSArray::from_retained_slice(&ids));
}

pub fn keep_only_notifications(ids: Vec<String>) {
    let Some(center) = notification_center() else { return };
    let keep: HashSet<String> = ids.into_iter().collect();
    // The system answers later: notifications shown meanwhile are new, and stay.
    let asked = NSDate::now();
    let remover = center.clone();
    let delivered = RcBlock::new(move |notifications: NonNull<NSArray<UNNotification>>| {
        // SAFETY: the system passes a valid array, for the length of the call.
        let notifications = unsafe { notifications.as_ref() };
        let stale: Vec<Retained<NSString>> = notifications
            .iter()
            .filter(|notification| notification.date().timeIntervalSinceDate(&asked) < 0.0)
            .map(|notification| notification.request().identifier())
            .filter(|id| !keep.contains(&id.to_string()))
            .collect();
        if !stale.is_empty() {
            remover.removeDeliveredNotificationsWithIdentifiers(&NSArray::from_retained_slice(&stale));
        }
    });
    center.getDeliveredNotificationsWithCompletionHandler(&delivered);
}

pub fn remove_all_notifications() {
    if let Some(center) = notification_center() {
        center.removeAllDeliveredNotifications();
    }
}

pub fn set_badge(count: i32) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let label = (count > 0).then(|| NSString::from_str(&count.to_string()));
    NSApplication::sharedApplication(mtm).dockTile().setBadgeLabel(label.as_deref());
}

pub fn flash_icon() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    NSApplication::sharedApplication(mtm).requestUserAttention(NSRequestUserAttentionType::InformationalRequest);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_is_drawn_on_nothing() {
        let png = mark_png(36).expect("the mark as a PNG");
        let picture = image::load_from_memory(&png).expect("a PNG").to_rgba8();
        assert_eq!(picture.dimensions(), (36, 36));
        let alpha = |x: u32, y: u32| picture.get_pixel(x, y)[3];
        assert_eq!(alpha(0, 0), 0, "the corners are empty");
        let covered = picture.pixels().filter(|pixel| pixel[3] > 128).count();
        assert!((150..1000).contains(&covered), "the bird covers part of the square, not {covered} pixels");
    }

    /// The delegate is only made inside an app; here its class is checked against the framework's
    /// protocol (objc2 compares the methods' types when it registers a class in a debug build).
    #[test]
    fn the_notification_delegate_answers_the_notification_centre() {
        let delegate = NotificationDelegate::new();
        let delegate: &ProtocolObject<dyn UNUserNotificationCenterDelegate> = ProtocolObject::from_ref(&*delegate);
        assert!(delegate.respondsToSelector(sel!(userNotificationCenter:willPresentNotification:withCompletionHandler:)));
        assert!(delegate.respondsToSelector(sel!(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:)));
        // A test executable is no app: no notification centre, and no exception for asking.
        assert!(notification_center().is_none());
    }
}
