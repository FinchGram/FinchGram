//! macOS: FinchGram stays in the Dock when its window is closed. winit 0.30 owns the application's
//! delegate and has nothing for a click on the Dock icon or for Quit, so two methods are added to
//! its delegate class at run time, the way Slint adds one of its own (i-slint-backend-winit's
//! disable_macos_automatic_shortcut_localization).

use std::cell::{Cell, RefCell};
use std::ffi::CStr;

use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::sel;
use slint::ComponentHandle;

use crate::MainWindow;

/// NSApplicationTerminateReply's NSTerminateNow.
const TERMINATE_NOW: usize = 1;

thread_local! {
    static WINDOW: RefCell<Option<slint::Weak<MainWindow>>> = const { RefCell::new(None) };
    static BEFORE_QUIT: Cell<Option<fn()>> = const { Cell::new(None) };
}

pub fn stay_in_dock(ui: &MainWindow, before_quit: fn()) {
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

/// A click on the Dock icon, or FinchGram opened again, while its window is closed: show it. Slint
/// shows a window from inside its event loop, so the window is shown from there.
unsafe extern "C-unwind" fn reopen(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject, has_visible_windows: Bool) -> Bool {
    if !has_visible_windows.as_bool() {
        let _ = slint::invoke_from_event_loop(|| {
            if let Some(ui) = WINDOW.with(|window| window.borrow().as_ref().and_then(slint::Weak::upgrade)) {
                super::show_window(&ui);
            }
        });
    }
    // Handled: AppKit has nothing more to do.
    Bool::NO
}

/// Quit (⌘Q, the Dock's menu, logging out of macOS): let TDLib close first. AppKit ends the process
/// right after, so nothing after the event loop runs.
unsafe extern "C-unwind" fn should_terminate(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject) -> usize {
    if let Some(before_quit) = BEFORE_QUIT.with(Cell::take) {
        before_quit();
    }
    TERMINATE_NOW
}
