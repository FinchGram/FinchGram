//! The screenshot tool (the design's seventh round, "FinchGram Desktop 截图"): the scissors in the
//! composer, or its shortcut (⌘⇧A, Settings → General → Screenshots), freeze the screen under an
//! overlay per display (ui/shot.slint). The user clicks a window (lit under the pointer) or drags
//! a selection, adjusts it by its handles, draws on it with the toolbar's tools, and Done
//! hands the picture to the card before sending (telegram/attachments.rs), Copy puts it on the
//! clipboard, Save writes it to the Downloads folder.
//!
//! The screen is captured by the system (platform::capture_display) once FinchGram's own window
//! has hidden, when the setting says so (off by default: then FinchGram is in the picture like any
//! other window, and can be taken); then one borderless window per display covers it, above
//! everything, with the frozen picture. All the overlay shows comes from here: it only reports the
//! pointer and the keys. The picture itself is made in export.rs.

mod export;

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use chrono::Local;
use slint::winit_030::winit::event::{ElementState, WindowEvent};
use slint::winit_030::winit::keyboard::{Key, ModifiersState};
use slint::{ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use crate::settings::{Screenshots, Settings};
use crate::{
    AppState, MainWindow, Screenshot, Shot, ShotBox, ShotCell, ShotCursor, ShotPath, ShotText, ShotTool, ShotWindow, platform, telegram,
};

/// How long the window takes to hide before the screen is captured.
const HIDE_WAIT: Duration = Duration::from_millis(250);
/// The six colours of the annotations, red first (the default), and the halo each gets: light
/// colours a dark one, dark colours a light one, so that a line reads on anything.
const COLORS: [&str; 6] = ["#F2352B", "#FFFFFF", "#16171A", "#FFCC00", "#1F7BFF", "#1FA855"];
/// Line widths, arrow heads (length, width), text sizes and mosaic brushes: small, medium, large,
/// in points.
const LINE_WIDTH: [f32; 3] = [2.0, 4.0, 8.0];
const ARROW_HEAD: [(f32, f32); 3] = [(12.0, 10.0), (18.0, 15.0), (28.0, 24.0)];
const FONT_SIZE: [f32; 3] = [14.0, 20.0, 28.0];
const BRUSH: [f32; 3] = [12.0, 24.0, 40.0];
/// A mosaic block, in points, on a grid of the display.
const BLOCK: f32 = 8.0;
/// How near a handle the pointer counts as on it.
const HANDLE_REACH: f32 = 7.0;
/// A selection smaller than this is a click, not a selection.
const SMALLEST: f32 = 2.0;
/// A press that moves less than this before it is released is a click, which takes the window it
/// was on; further, it is a drag, which makes a selection.
const DRAG_START: f32 = 4.0;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    fn between(a: (f32, f32), b: (f32, f32)) -> Rect {
        Rect { x: a.0.min(b.0), y: a.1.min(b.1), w: (a.0 - b.0).abs(), h: (a.1 - b.1).abs() }
    }

    fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 <= self.x + self.w && p.1 >= self.y && p.1 <= self.y + self.h
    }

    /// Kept within a display of `size`, moved rather than shrunk when it can be.
    fn within(self, size: (f32, f32)) -> Rect {
        let w = self.w.min(size.0);
        let h = self.h.min(size.1);
        Rect { x: self.x.clamp(0.0, size.0 - w), y: self.y.clamp(0.0, size.1 - h), w, h }
    }
}

/// One display, frozen.
pub struct Display {
    /// Where it is, in points of the global space (the main display's top left corner is 0, 0).
    pub origin: (f32, f32),
    pub size: (f32, f32),
    /// Device pixels per point.
    pub scale: f32,
    /// The picture's size in device pixels, and the picture, RGBA.
    pub px: (u32, u32),
    pub rgba: Vec<u8>,
    image: Image,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    None,
    Rect,
    Ellipse,
    Arrow,
    Pen,
    Text,
    Mosaic,
}

#[derive(Clone, Debug)]
pub struct MosaicCell {
    pub x: f32,
    pub y: f32,
    pub color: String,
}

#[derive(Clone, Debug)]
pub enum Shape {
    Box { rect: Rect, ellipse: bool },
    Arrow { from: (f32, f32), to: (f32, f32) },
    Pen { points: Vec<(f32, f32)> },
    Text { at: (f32, f32), text: String },
    Mosaic { cells: Vec<MosaicCell>, size: f32 },
}

#[derive(Clone, Debug)]
pub struct Annotation {
    pub shape: Shape,
    pub color: usize,
    pub size: usize,
}

impl Annotation {
    pub fn line_width(&self) -> f32 {
        LINE_WIDTH[self.size.min(2)]
    }

    pub fn font_size(&self) -> f32 {
        FONT_SIZE[self.size.min(2)]
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    /// No selection yet: the window under the pointer is offered.
    Waiting,
    /// The button is down since `start`: a drag makes a selection; a click takes `window`, the one
    /// lit when the button went down, if any.
    Selecting { start: (f32, f32), window: Option<Rect> },
    Settled,
    Moving { grab: (f32, f32), from: Rect },
    Resizing { handle: usize, from: Rect },
    Drawing { anchor: (f32, f32) },
    Texting,
}

struct Session {
    ui: slint::Weak<MainWindow>,
    displays: Vec<Display>,
    windows: Vec<ShotWindow>,
    /// The windows on the screen, front to back, in global points: FinchGram's own among them,
    /// unless it hid.
    others: Vec<platform::ScreenWindow>,
    phase: Phase,
    /// The display the selection is on (or the pointer was last seen on).
    display: usize,
    selection: Option<Rect>,
    hover: Option<Rect>,
    pointer: (usize, (f32, f32)),
    tool: Tool,
    size: usize,
    color: usize,
    annotations: Vec<Annotation>,
    redo: Vec<Annotation>,
    draft: Option<Annotation>,
    last_point: (f32, f32),
    stroke_cells: HashSet<(i32, i32)>,
    text_at: (f32, f32),
    hid_window: bool,
    boxes: Rc<VecModel<ShotBox>>,
    paths: Rc<VecModel<ShotPath>>,
    texts: Rc<VecModel<ShotText>>,
    cells: Rc<VecModel<ShotCell>>,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    static UI: RefCell<Option<slint::Weak<MainWindow>>> = const { RefCell::new(None) };
    static SETTINGS: RefCell<Option<Rc<RefCell<Settings>>>> = const { RefCell::new(None) };
    /// The system was asked for leave to capture the screen once this run.
    static ASKED: Cell<bool> = const { Cell::new(false) };
    /// The keyboard's modifiers, as winit reports them, for the shortcut.
    static MODIFIERS: Cell<ModifiersState> = const { Cell::new(ModifiersState::empty()) };
}

// ---- the main window's side ---------------------------------------------------------------------

/// Connect the scissors, the permission card and Settings → General → Screenshots. Call once.
pub fn connect(ui: &MainWindow, settings: Rc<RefCell<Settings>>) {
    UI.with(|slot| *slot.borrow_mut() = Some(ui.as_weak()));
    SETTINGS.with(|slot| *slot.borrow_mut() = Some(settings.clone()));
    let screenshot = ui.global::<Screenshot>();
    show_settings(&screenshot, &settings.borrow().screenshots);
    screenshot.on_start({
        let ui = ui.as_weak();
        move || {
            if let Some(ui) = ui.upgrade() {
                start(&ui);
            }
        }
    });
    screenshot.on_open_system_settings({
        let ui = ui.as_weak();
        move || {
            platform::open_screen_capture_settings();
            if let Some(ui) = ui.upgrade() {
                ui.global::<Screenshot>().set_permission_card(false);
            }
        }
    });
    screenshot.on_dismiss_permission({
        let ui = ui.as_weak();
        move || {
            if let Some(ui) = ui.upgrade() {
                ui.global::<Screenshot>().set_permission_card(false);
            }
        }
    });
    screenshot.on_record_shortcut({
        let ui = ui.as_weak();
        let settings = settings.clone();
        move |key, cmd, ctrl, alt, shift| {
            let Some(shortcut) = Shortcut::from_keys(&key, cmd, ctrl, alt, shift) else { return };
            settings.borrow_mut().screenshots.shortcut = shortcut.to_string();
            settings.borrow().save();
            if let Some(ui) = ui.upgrade() {
                let screenshot = ui.global::<Screenshot>();
                screenshot.set_recording(false);
                show_settings(&screenshot, &settings.borrow().screenshots);
            }
        }
    });
    screenshot.on_reset_shortcut({
        let ui = ui.as_weak();
        let settings = settings.clone();
        move || {
            settings.borrow_mut().screenshots.shortcut = Screenshots::default().shortcut;
            settings.borrow().save();
            if let Some(ui) = ui.upgrade() {
                let screenshot = ui.global::<Screenshot>();
                screenshot.set_recording(false);
                show_settings(&screenshot, &settings.borrow().screenshots);
            }
        }
    });
    screenshot.on_set_hide_own_window({
        let ui = ui.as_weak();
        move |hide| {
            settings.borrow_mut().screenshots.hide_own_window = hide;
            settings.borrow().save();
            if let Some(ui) = ui.upgrade() {
                ui.global::<Screenshot>().set_hide_own_window(hide);
            }
        }
    });
}

fn show_settings(screenshot: &Screenshot, settings: &Screenshots) {
    let shortcut = Shortcut::parse(&settings.shortcut).unwrap_or_default();
    screenshot.set_shortcut_label(shortcut.label().into());
    screenshot.set_custom_shortcut(shortcut.to_string() != Screenshots::default().shortcut);
    screenshot.set_hide_own_window(settings.hide_own_window);
    screenshot.set_background(settings.global);
}

/// The keys that take a screenshot, as "cmd+shift+a" in the settings and "⌘⇧A" on the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    cmd: bool,
    ctrl: bool,
    alt: bool,
    shift: bool,
    key: String,
}

impl Default for Shortcut {
    fn default() -> Self {
        Shortcut::parse(&Screenshots::default().shortcut).expect("the default shortcut")
    }
}

impl Shortcut {
    pub fn parse(text: &str) -> Option<Shortcut> {
        let mut shortcut = Shortcut { cmd: false, ctrl: false, alt: false, shift: false, key: String::new() };
        for part in text.split('+') {
            match part.trim().to_ascii_lowercase().as_str() {
                "cmd" => shortcut.cmd = true,
                "ctrl" => shortcut.ctrl = true,
                "alt" => shortcut.alt = true,
                "shift" => shortcut.shift = true,
                key => shortcut.key = key.to_string(),
            }
        }
        shortcut.valid().then_some(shortcut)
    }

    /// A key pressed in the settings' field: one letter or digit with ⌘ or ⌃ makes a shortcut.
    fn from_keys(key: &str, cmd: bool, ctrl: bool, alt: bool, shift: bool) -> Option<Shortcut> {
        let key = key.trim().to_lowercase();
        let shortcut = Shortcut { cmd, ctrl, alt, shift, key };
        shortcut.valid().then_some(shortcut)
    }

    fn valid(&self) -> bool {
        (self.cmd || self.ctrl) && self.key.chars().count() == 1 && self.key.chars().all(|c| c.is_ascii_alphanumeric())
    }

    /// As macOS writes keys: "⌘⇧A".
    pub fn label(&self) -> String {
        let mut label = String::new();
        if self.cmd {
            label.push('⌘');
        }
        if self.ctrl {
            label.push('⌃');
        }
        if self.alt {
            label.push('⌥');
        }
        if self.shift {
            label.push('⇧');
        }
        label.push_str(&self.key.to_uppercase());
        label
    }

    fn matches(&self, key: &str, modifiers: ModifiersState) -> bool {
        key.eq_ignore_ascii_case(&self.key)
            && modifiers.super_key() == self.cmd
            && modifiers.control_key() == self.ctrl
            && modifiers.alt_key() == self.alt
            && modifiers.shift_key() == self.shift
    }
}

impl std::fmt::Display for Shortcut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (on, name) in [(self.cmd, "cmd"), (self.ctrl, "ctrl"), (self.alt, "alt"), (self.shift, "shift")] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        write!(f, "{}", self.key)
    }
}

/// The main window's own events (src/telegram/mod.rs): the shortcut starts a screenshot.
pub fn window_event(event: &WindowEvent) {
    match event {
        WindowEvent::ModifiersChanged(modifiers) => MODIFIERS.set(modifiers.state()),
        WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed && !event.repeat => {
            let Key::Character(key) = &event.logical_key else { return };
            let shortcut = SETTINGS
                .with(|settings| settings.borrow().as_ref().and_then(|settings| Shortcut::parse(&settings.borrow().screenshots.shortcut)));
            if shortcut.is_some_and(|shortcut| shortcut.matches(key, MODIFIERS.get())) {
                let ui = UI.with(|ui| ui.borrow().as_ref().and_then(|ui| ui.upgrade()));
                if let Some(ui) = ui {
                    start(&ui);
                }
            }
        }
        _ => {}
    }
}

/// Take a screenshot for the chat in front: unless one is being taken, the chat takes no photos,
/// or the system has not given leave (asked for once; then the card says where to give it).
pub fn start(ui: &MainWindow) {
    if SESSION.with(|session| session.borrow().is_some()) || !telegram::can_send_photos() {
        return;
    }
    if !platform::screen_capture_allowed() {
        if ASKED.replace(true) {
            ui.global::<Screenshot>().set_permission_card(true);
        } else {
            platform::request_screen_capture();
        }
        return;
    }
    let monitors = monitors(ui);
    if monitors.is_empty() {
        return;
    }
    let hide = ui.global::<Screenshot>().get_hide_own_window();
    if hide {
        ui.window().hide().ok();
    }
    let weak = ui.as_weak();
    slint::Timer::single_shot(if hide { HIDE_WAIT } else { Duration::ZERO }, move || capture(weak, monitors, hide));
}

/// A display as winit knows it: where (device pixels), how large (device pixels), its scale.
struct Monitor {
    position: (i32, i32),
    size: (u32, u32),
    scale: f32,
}

fn monitors(ui: &MainWindow) -> Vec<Monitor> {
    use slint::winit_030::WinitWindowAccessor;
    ui.window()
        .with_winit_window(|window| {
            window
                .available_monitors()
                .map(|monitor| Monitor {
                    position: (monitor.position().x, monitor.position().y),
                    size: (monitor.size().width, monitor.size().height),
                    scale: monitor.scale_factor() as f32,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Capture every display on another thread (the system takes a moment for each), then open the
/// overlays.
fn capture(ui: slint::Weak<MainWindow>, monitors: Vec<Monitor>, hid: bool) {
    let folder = scratch_dir();
    let _ = std::fs::create_dir_all(&folder);
    let main = ui.clone();
    let spawned = std::thread::Builder::new().name("screenshot".into()).spawn(move || {
        let mut captured = Vec::new();
        for index in 0..monitors.len() {
            let path = folder.join(format!("display-{index}.png"));
            let picture = platform::capture_display(index + 1, &path).then(|| image::open(&path).ok()).flatten();
            let _ = std::fs::remove_file(&path);
            match picture {
                Some(picture) => {
                    let rgba = picture.into_rgba8();
                    captured.push(Some((rgba.width(), rgba.height(), rgba.into_raw())));
                }
                None => captured.push(None),
            }
        }
        let _ = slint::invoke_from_event_loop(move || open(ui, monitors, captured, hid));
    });
    if let Err(err) = spawned {
        eprintln!("screenshot: cannot start a thread: {err}");
        if let (true, Some(main)) = (hid, main.upgrade()) {
            platform::show_window(&main);
        }
    }
}

type Captured = Option<(u32, u32, Vec<u8>)>;

/// The pictures in the monitors' order: the system numbers displays its own way, so each monitor
/// takes the picture of its size, and only then the one of its number.
fn matched(monitors: &[Monitor], mut captured: Vec<Captured>) -> Vec<Captured> {
    let mut out: Vec<Captured> = Vec::with_capacity(monitors.len());
    for (index, monitor) in monitors.iter().enumerate() {
        let same_size = captured.iter().position(|picture| picture.as_ref().is_some_and(|(w, h, _)| (*w, *h) == monitor.size));
        let pick = same_size.or_else(|| (index < captured.len() && captured[index].is_some()).then_some(index));
        out.push(pick.and_then(|at| captured[at].take()));
    }
    out
}

/// The overlays: one window per display captured, above everything, with the frozen picture.
fn open(ui: slint::Weak<MainWindow>, monitors: Vec<Monitor>, captured: Vec<Captured>, hid: bool) {
    let Some(main) = ui.upgrade() else { return };
    let captured = matched(&monitors, captured);
    let mut displays = Vec::new();
    let mut monitors = monitors;
    let mut kept = Vec::new();
    for (monitor, picture) in monitors.drain(..).zip(captured) {
        let Some((width, height, rgba)) = picture else { continue };
        kept.push(monitor);
        let monitor = kept.last().expect("just pushed");
        let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba, width, height);
        displays.push(Display {
            origin: (monitor.position.0 as f32 / monitor.scale, monitor.position.1 as f32 / monitor.scale),
            size: (monitor.size.0 as f32 / monitor.scale, monitor.size.1 as f32 / monitor.scale),
            scale: monitor.scale,
            px: (width, height),
            rgba,
            image: Image::from_rgba8(buffer),
        });
    }
    let monitors = kept;
    if displays.is_empty() {
        eprintln!("screenshot: the system gave no picture of the screen");
        if hid {
            platform::show_window(&main);
        }
        return;
    }
    let others = platform::windows_on_screen();
    let theme = main.global::<AppState>().get_theme();
    let appearance = main.global::<AppState>().get_appearance();
    let mut windows = Vec::new();
    let boxes = Rc::new(VecModel::default());
    let paths = Rc::new(VecModel::default());
    let texts = Rc::new(VecModel::default());
    let cells = Rc::new(VecModel::default());
    for (index, (display, monitor)) in displays.iter().zip(&monitors).enumerate() {
        let Ok(window) = ShotWindow::new() else { break };
        window.global::<AppState>().set_theme(theme);
        window.global::<AppState>().set_appearance(appearance.clone());
        let shot = window.global::<Shot>();
        shot.set_frame(display.image.clone());
        shot.set_scale(display.scale);
        shot.set_display(index as i32);
        shot.set_boxes(ModelRc::from(boxes.clone()));
        shot.set_paths(ModelRc::from(paths.clone()));
        shot.set_texts(ModelRc::from(texts.clone()));
        shot.set_cells(ModelRc::from(cells.clone()));
        shot.on_pressed(move |x, y, right, shift| with_session(|session| session.pressed(index, (x, y), right, shift)));
        shot.on_moved(move |x, y, shift| with_session(|session| session.moved(index, (x, y), shift)));
        shot.on_released(move |x, y| with_session(|session| session.released(index, (x, y))));
        shot.on_double_clicked(|| with_session(|session| session.done()));
        shot.on_key(|key, cmd, shift| with_session(|session| session.key(&key, cmd, shift)));
        shot.on_choose_tool(|tool| with_session(|session| session.choose_tool(tool)));
        shot.on_choose_size(|size| with_session(|session| session.size = size.clamp(0, 2) as usize));
        shot.on_choose_color(|color| with_session(|session| session.color = color.clamp(0, 5) as usize));
        shot.on_undo(|| with_session(|session| session.undo()));
        shot.on_redo(|| with_session(|session| session.redo()));
        shot.on_copy(|| with_session(|session| session.copy()));
        shot.on_save(|| with_session(|session| session.save()));
        shot.on_cancel(|| with_session(|session| session.cancel()));
        shot.on_done(|| with_session(|session| session.done()));
        shot.on_text_done(|text| with_session(|session| session.text_done(&text)));
        window.window().set_position(slint::PhysicalPosition::new(monitor.position.0, monitor.position.1));
        window.window().set_size(slint::PhysicalSize::new(monitor.size.0, monitor.size.1));
        if window.show().is_err() {
            break;
        }
        platform::raise_overlay(window.window());
        windows.push(window);
    }
    // One overlay per display, or none: the pointer's display and its window must agree.
    if windows.len() != displays.len() {
        eprintln!("screenshot: cannot open a window over every display");
        for window in &windows {
            window.hide().ok();
        }
        if hid {
            platform::show_window(&main);
        }
        return;
    }
    SESSION.with(|session| {
        *session.borrow_mut() = Some(Session {
            ui,
            displays,
            windows,
            others,
            phase: Phase::Waiting,
            display: 0,
            selection: None,
            hover: None,
            pointer: (0, (0.0, 0.0)),
            tool: Tool::None,
            size: 1,
            color: 0,
            annotations: Vec::new(),
            redo: Vec::new(),
            draft: None,
            last_point: (0.0, 0.0),
            stroke_cells: HashSet::new(),
            text_at: (0.0, 0.0),
            hid_window: hid,
            boxes,
            paths,
            texts,
            cells,
        });
    });
    with_session(|_| {});
}

/// Run `change` on the session, then show what it did; a session that ended inside `change` is
/// dropped afterwards.
fn with_session(change: impl FnOnce(&mut Session)) {
    let ended = SESSION.with(|session| {
        let mut session = session.borrow_mut();
        let Some(session) = session.as_mut() else { return false };
        change(session);
        session.refresh();
        session.windows.is_empty()
    });
    if ended {
        SESSION.with(|session| session.borrow_mut().take());
    }
}

impl Session {
    fn display(&self) -> &Display {
        &self.displays[self.display.min(self.displays.len() - 1)]
    }

    fn pressed(&mut self, display: usize, p: (f32, f32), right: bool, shift: bool) {
        if self.phase == Phase::Texting {
            let draft = self.windows.get(self.display).map(|window| window.global::<Shot>().get_text_draft().to_string()).unwrap_or_default();
            self.text_done(&draft);
        }
        let inside = self.selection.is_some_and(|selection| display == self.display && selection.contains(p));
        if right {
            if !inside {
                self.cancel();
            }
            return;
        }
        if let (Some(selection), true) = (self.selection, display == self.display) {
            if let Some(handle) = self.handle_at(p) {
                self.phase = Phase::Resizing { handle, from: selection };
                return;
            }
            if selection.contains(p) {
                match self.tool {
                    Tool::None => self.phase = Phase::Moving { grab: p, from: selection },
                    Tool::Text => {
                        self.text_at = p;
                        self.phase = Phase::Texting;
                    }
                    _ => {
                        self.begin_draw(p);
                        self.phase = Phase::Drawing { anchor: p };
                        self.update_draw(p, shift);
                    }
                }
                return;
            }
        }
        // A new selection, on this display: whatever was drawn goes with the old one. The button
        // down over a lit window takes nothing yet: a drag from there makes a selection of its
        // own, and only a click (released) takes the window.
        self.display = display;
        self.annotations.clear();
        self.redo.clear();
        self.draft = None;
        self.tool = Tool::None;
        self.selection = None;
        let window = self.hover.filter(|_| self.pointer.0 == display);
        self.phase = Phase::Selecting { start: p, window };
    }

    fn moved(&mut self, display: usize, p: (f32, f32), shift: bool) {
        self.pointer = (display, p);
        match self.phase {
            Phase::Waiting | Phase::Settled if self.selection.is_none() => {
                self.hover = self.window_under(display, p);
                self.display = display;
            }
            Phase::Selecting { start, .. } if display == self.display => {
                // Not a drag yet: the window stays lit, and a click still takes it.
                if self.selection.is_none() && (p.0 - start.0).abs() < DRAG_START && (p.1 - start.1).abs() < DRAG_START {
                    return;
                }
                self.hover = None;
                let size = self.display().size;
                let mut rect = Rect::between(start, p);
                rect.x = rect.x.max(0.0);
                rect.y = rect.y.max(0.0);
                rect.w = rect.w.min(size.0 - rect.x);
                rect.h = rect.h.min(size.1 - rect.y);
                self.selection = Some(rect);
            }
            Phase::Moving { grab, from } if display == self.display => {
                let moved = Rect { x: from.x + p.0 - grab.0, y: from.y + p.1 - grab.1, ..from };
                self.selection = Some(moved.within(self.display().size));
            }
            Phase::Resizing { handle, from } if display == self.display => {
                self.selection = Some(resized(from, handle, p).within(self.display().size));
            }
            Phase::Drawing { .. } if display == self.display => self.update_draw(p, shift),
            _ => {}
        }
    }

    fn released(&mut self, _display: usize, _p: (f32, f32)) {
        match self.phase {
            Phase::Selecting { window, .. } => {
                // A click takes the window that was lit when the button went down; a drag, what
                // it drew, if that is anything.
                let clicked = self.selection.is_none();
                let drawn = self.selection.filter(|drawn| drawn.w >= SMALLEST && drawn.h >= SMALLEST);
                self.selection = if clicked { window } else { drawn };
                if self.selection.is_some() {
                    self.hover = None;
                    self.phase = Phase::Settled;
                } else {
                    self.phase = Phase::Waiting;
                    self.hover = self.window_under(self.pointer.0, self.pointer.1);
                }
            }
            Phase::Moving { .. } | Phase::Resizing { .. } => self.phase = Phase::Settled,
            Phase::Drawing { .. } => {
                if let Some(draft) = self.draft.take().filter(drawn) {
                    self.annotations.push(draft);
                    self.redo.clear();
                }
                self.phase = Phase::Settled;
            }
            _ => {}
        }
    }

    fn key(&mut self, key: &str, cmd: bool, shift: bool) {
        use slint::platform::Key;
        let is = |which: Key| key == SharedString::from(which).as_str();
        if is(Key::Escape) {
            if self.phase == Phase::Texting {
                self.text_done("");
            } else {
                self.cancel();
            }
        } else if is(Key::Return) {
            self.done();
        } else if cmd && key.eq_ignore_ascii_case("z") {
            if shift { self.redo() } else { self.undo() }
        } else if cmd && key.eq_ignore_ascii_case("c") {
            self.copy();
        } else if cmd && key.eq_ignore_ascii_case("s") {
            self.save();
        } else if let (Some(selection), Phase::Settled) = (self.selection, self.phase) {
            let step = if shift { 10.0 } else { 1.0 };
            let (dx, dy) = if is(Key::LeftArrow) {
                (-step, 0.0)
            } else if is(Key::RightArrow) {
                (step, 0.0)
            } else if is(Key::UpArrow) {
                (0.0, -step)
            } else if is(Key::DownArrow) {
                (0.0, step)
            } else {
                return;
            };
            let moved = Rect { x: selection.x + dx, y: selection.y + dy, ..selection };
            self.selection = Some(moved.within(self.display().size));
        }
    }

    fn choose_tool(&mut self, tool: ShotTool) {
        self.tool = match tool {
            ShotTool::Rect => Tool::Rect,
            ShotTool::Ellipse => Tool::Ellipse,
            ShotTool::Arrow => Tool::Arrow,
            ShotTool::Pen => Tool::Pen,
            ShotTool::Text => Tool::Text,
            ShotTool::Mosaic => Tool::Mosaic,
            ShotTool::None => Tool::None,
        };
    }

    fn undo(&mut self) {
        if let Some(last) = self.annotations.pop() {
            self.redo.push(last);
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.annotations.push(next);
        }
    }

    // ---- drawing -----------------------------------------------------------------------------

    fn begin_draw(&mut self, p: (f32, f32)) {
        let shape = match self.tool {
            Tool::Rect | Tool::Ellipse => Shape::Box { rect: Rect { x: p.0, y: p.1, w: 0.0, h: 0.0 }, ellipse: self.tool == Tool::Ellipse },
            Tool::Arrow => Shape::Arrow { from: p, to: p },
            Tool::Pen => Shape::Pen { points: vec![p] },
            _ => Shape::Mosaic { cells: Vec::new(), size: BLOCK },
        };
        self.draft = Some(Annotation { shape, color: self.color, size: self.size });
        self.stroke_cells.clear();
        self.last_point = p;
    }

    fn update_draw(&mut self, p: (f32, f32), shift: bool) {
        let Phase::Drawing { anchor } = self.phase else { return };
        let brush = BRUSH[self.size.min(2)];
        let new_cells = if self.tool == Tool::Mosaic { self.cells_along(self.last_point, p, brush / 2.0) } else { Vec::new() };
        let Some(draft) = self.draft.as_mut() else { return };
        match &mut draft.shape {
            Shape::Box { rect, .. } => {
                let mut to = p;
                if shift {
                    let side = (p.0 - anchor.0).abs().max((p.1 - anchor.1).abs());
                    to = (anchor.0 + side * (p.0 - anchor.0).signum(), anchor.1 + side * (p.1 - anchor.1).signum());
                }
                *rect = Rect::between(anchor, to);
            }
            Shape::Arrow { to, .. } => {
                *to = if shift { snapped(anchor, p) } else { p };
            }
            Shape::Pen { points } => {
                if points.last().is_none_or(|last| (last.0 - p.0).abs() + (last.1 - p.1).abs() >= 1.0) {
                    points.push(p);
                }
            }
            Shape::Mosaic { cells, .. } => cells.extend(new_cells),
            Shape::Text { .. } => {}
        }
        self.last_point = p;
    }

    /// The mosaic blocks the brush of radius `r` paints between `a` and `b`, each once, in the
    /// average colour of the picture under it.
    fn cells_along(&mut self, a: (f32, f32), b: (f32, f32), r: f32) -> Vec<MosaicCell> {
        let (x0, x1) = ((a.0.min(b.0) - r) / BLOCK, (a.0.max(b.0) + r) / BLOCK);
        let (y0, y1) = ((a.1.min(b.1) - r) / BLOCK, (a.1.max(b.1) + r) / BLOCK);
        let mut cells = Vec::new();
        for by in (y0.floor() as i32)..=(y1.ceil() as i32) {
            for bx in (x0.floor() as i32)..=(x1.ceil() as i32) {
                if self.stroke_cells.contains(&(bx, by)) {
                    continue;
                }
                let centre = (bx as f32 * BLOCK + BLOCK / 2.0, by as f32 * BLOCK + BLOCK / 2.0);
                if distance_to_segment(centre, a, b) > r {
                    continue;
                }
                let Some(color) = self.block_colour(bx, by) else { continue };
                self.stroke_cells.insert((bx, by));
                cells.push(MosaicCell { x: bx as f32 * BLOCK, y: by as f32 * BLOCK, color });
            }
        }
        cells
    }

    /// The average colour of the picture under a block, as "#rrggbb"; none off the picture.
    fn block_colour(&self, bx: i32, by: i32) -> Option<String> {
        let display = self.display();
        let (w, h) = display.px;
        let side = (BLOCK * display.scale).round().max(1.0) as i64;
        let (x0, y0) = ((bx as f32 * BLOCK * display.scale).round() as i64, (by as f32 * BLOCK * display.scale).round() as i64);
        let (mut sum, mut count) = ([0u64; 3], 0u64);
        for y in y0.max(0)..(y0 + side).min(h as i64) {
            for x in x0.max(0)..(x0 + side).min(w as i64) {
                let at = ((y as u32 * w + x as u32) * 4) as usize;
                sum[0] += display.rgba[at] as u64;
                sum[1] += display.rgba[at + 1] as u64;
                sum[2] += display.rgba[at + 2] as u64;
                count += 1;
            }
        }
        (count > 0).then(|| format!("#{:02x}{:02x}{:02x}", sum[0] / count, sum[1] / count, sum[2] / count))
    }

    fn text_done(&mut self, text: &str) {
        if self.phase != Phase::Texting {
            return;
        }
        let text = text.trim();
        if !text.is_empty() {
            self.annotations.push(Annotation { shape: Shape::Text { at: self.text_at, text: text.to_string() }, color: self.color, size: self.size });
            self.redo.clear();
        }
        if let Some(window) = self.windows.get(self.display) {
            window.global::<Shot>().set_text_draft(SharedString::new());
        }
        self.phase = Phase::Settled;
    }

    // ---- finishing ---------------------------------------------------------------------------

    fn picture(&self) -> Option<Vec<u8>> {
        let selection = self.selection?;
        export::compose(self.display(), selection, &self.annotations)
    }

    fn done(&mut self) {
        if self.phase == Phase::Texting {
            let draft = self.windows.get(self.display).map(|window| window.global::<Shot>().get_text_draft().to_string()).unwrap_or_default();
            self.text_done(&draft);
        }
        let Some(png) = self.picture() else { return };
        let path = scratch_dir().join(file_name());
        if let Err(err) = std::fs::create_dir_all(scratch_dir()).and_then(|()| std::fs::write(&path, &png)) {
            eprintln!("screenshot: cannot write the picture: {err}");
            return;
        }
        self.finish();
        telegram::add_screenshot(path);
    }

    fn copy(&mut self) {
        let Some(png) = self.picture() else { return };
        self.finish();
        match platform::copy_image(&png) {
            Ok(()) => telegram::screenshot_copied(),
            Err(err) => eprintln!("screenshot: cannot copy the picture: {err}"),
        }
    }

    fn save(&mut self) {
        let Some(png) = self.picture() else { return };
        let name = file_name();
        let path = scratch_dir().join(&name);
        if let Err(err) = std::fs::create_dir_all(scratch_dir()).and_then(|()| std::fs::write(&path, &png)) {
            eprintln!("screenshot: cannot write the picture: {err}");
            return;
        }
        self.finish();
        match telegram::save_to_downloads(&path, &name) {
            Ok(saved) => telegram::screenshot_saved(&saved),
            Err(err) => eprintln!("screenshot: cannot save the picture: {err}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    fn cancel(&mut self) {
        self.finish();
    }

    /// Close the overlays and bring FinchGram's window back.
    fn finish(&mut self) {
        for window in self.windows.drain(..) {
            window.hide().ok();
        }
        if let (true, Some(ui)) = (self.hid_window, self.ui.upgrade()) {
            platform::show_window(&ui);
        }
    }

    // ---- what the overlays show --------------------------------------------------------------

    fn handle_at(&self, p: (f32, f32)) -> Option<usize> {
        let selection = self.selection?;
        handles(selection).iter().position(|handle| (handle.0 - p.0).abs() <= HANDLE_REACH && (handle.1 - p.1).abs() <= HANDLE_REACH)
    }

    /// The window under the pointer, as a rectangle of this display, if any.
    fn window_under(&self, display: usize, p: (f32, f32)) -> Option<Rect> {
        let screen = self.displays.get(display)?;
        let global = (screen.origin.0 + p.0, screen.origin.1 + p.1);
        let window = self.others.iter().find(|window| {
            global.0 >= window.x && global.0 <= window.x + window.width && global.1 >= window.y && global.1 <= window.y + window.height
        })?;
        let rect = Rect { x: window.x - screen.origin.0, y: window.y - screen.origin.1, w: window.width, h: window.height };
        let x = rect.x.max(0.0);
        let y = rect.y.max(0.0);
        let w = (rect.x + rect.w).min(screen.size.0) - x;
        let h = (rect.y + rect.h).min(screen.size.1) - y;
        (w > 0.0 && h > 0.0).then_some(Rect { x, y, w, h })
    }

    fn cursor(&self) -> ShotCursor {
        let (display, p) = self.pointer;
        match self.phase {
            Phase::Moving { .. } => return ShotCursor::Move,
            Phase::Resizing { handle, .. } => return handle_cursor(handle),
            Phase::Drawing { .. } if self.tool == Tool::Mosaic => return ShotCursor::Brush,
            Phase::Drawing { .. } | Phase::Selecting { .. } => return ShotCursor::Cross,
            _ => {}
        }
        if display != self.display {
            return ShotCursor::Cross;
        }
        if let Some(handle) = self.handle_at(p) {
            return handle_cursor(handle);
        }
        match (self.selection, self.tool) {
            (Some(selection), Tool::None) if selection.contains(p) => ShotCursor::Move,
            (Some(selection), Tool::Mosaic) if selection.contains(p) => ShotCursor::Brush,
            _ => ShotCursor::Cross,
        }
    }

    fn refresh(&mut self) {
        let settled = self.selection.is_some() && !matches!(self.phase, Phase::Waiting | Phase::Selecting { .. });
        let adjusting = matches!(self.phase, Phase::Selecting { .. } | Phase::Moving { .. } | Phase::Resizing { .. });
        self.sync_models();
        let (pointer_display, pointer) = self.pointer;
        let cursor = self.cursor();
        for (index, window) in self.windows.iter().enumerate() {
            let shot = window.global::<Shot>();
            let here = index == self.display;
            let selection = self.selection.filter(|_| here);
            let display = &self.displays[index.min(self.displays.len() - 1)];
            shot.set_has_selection(selection.is_some());
            if let Some(selection) = selection {
                shot.set_sel_x(selection.x);
                shot.set_sel_y(selection.y);
                shot.set_sel_w(selection.w);
                shot.set_sel_h(selection.h);
            }
            shot.set_settled(settled && here);
            shot.set_adjusting(adjusting);
            let hover = self.hover.filter(|_| index == pointer_display && self.selection.is_none());
            shot.set_hover_window(hover.is_some());
            if let Some(hover) = hover {
                shot.set_hover_x(hover.x);
                shot.set_hover_y(hover.y);
                shot.set_hover_w(hover.w);
                shot.set_hover_h(hover.h);
            }
            let labelled = selection.or(hover);
            shot.set_label(
                labelled
                    .map(|rect| format!("{} × {}", (rect.w * display.scale).round() as i32, (rect.h * display.scale).round() as i32))
                    .unwrap_or_default()
                    .into(),
            );
            let magnifying = index == pointer_display && matches!(self.phase, Phase::Selecting { .. }) && self.selection.is_some();
            shot.set_show_magnifier(magnifying);
            if index == pointer_display {
                shot.set_pointer_x(pointer.0);
                shot.set_pointer_y(pointer.1);
                if magnifying {
                    let (px, py) = ((pointer.0 * display.scale).round() as i64, (pointer.1 * display.scale).round() as i64);
                    shot.set_mag_pos(format!("x {px}  y {py}").into());
                    let hex = pixel_hex(display, px, py);
                    shot.set_mag_color(slint::Color::from_argb_encoded(0xff00_0000 | u32::from_str_radix(&hex[1..], 16).unwrap_or(0)));
                    shot.set_mag_hex(hex.into());
                }
            }
            shot.set_cursor(if index == pointer_display { cursor } else { ShotCursor::Cross });
            shot.set_brush(BRUSH[self.size.min(2)]);
            shot.set_tool(match self.tool {
                Tool::None => ShotTool::None,
                Tool::Rect => ShotTool::Rect,
                Tool::Ellipse => ShotTool::Ellipse,
                Tool::Arrow => ShotTool::Arrow,
                Tool::Pen => ShotTool::Pen,
                Tool::Text => ShotTool::Text,
                Tool::Mosaic => ShotTool::Mosaic,
            });
            shot.set_size(self.size as i32);
            shot.set_color_index(self.color as i32);
            shot.set_can_undo(!self.annotations.is_empty());
            shot.set_can_redo(!self.redo.is_empty());
            let editing = here && self.phase == Phase::Texting;
            shot.set_editing(editing);
            if editing {
                shot.set_text_x(self.text_at.0);
                shot.set_text_y(self.text_at.1);
                shot.set_text_size(FONT_SIZE[self.size.min(2)]);
                shot.set_text_color(color_of(self.color));
                shot.set_text_halo(halo_of(self.color));
            }
        }
    }

    /// The annotations, and the one being drawn, as the overlay draws them.
    fn sync_models(&mut self) {
        let mut boxes = Vec::new();
        let mut paths = Vec::new();
        let mut texts = Vec::new();
        let mut cells = Vec::new();
        for annotation in self.annotations.iter().chain(self.draft.iter()) {
            let color = color_of(annotation.color);
            let halo = halo_of(annotation.color);
            let width = annotation.line_width();
            match &annotation.shape {
                Shape::Box { rect, ellipse } => {
                    boxes.push(ShotBox { x: rect.x, y: rect.y, w: rect.w, h: rect.h, color, halo, width, ellipse: *ellipse })
                }
                Shape::Arrow { from, to } => {
                    let (shaft, head) = arrow_paths(*from, *to, annotation.size);
                    paths.push(ShotPath { commands: shaft.into(), color, halo, width, fill: false });
                    paths.push(ShotPath { commands: head.into(), color, halo, width: 1.0, fill: true });
                }
                Shape::Pen { points } => paths.push(ShotPath { commands: polyline(points).into(), color, halo, width, fill: false }),
                Shape::Text { at, text } => {
                    texts.push(ShotText { x: at.0, y: at.1, text: text.clone().into(), color, halo, size: annotation.font_size() })
                }
                Shape::Mosaic { cells: blocks, size } => {
                    for block in blocks {
                        cells.push(ShotCell { x: block.x, y: block.y, size: *size, color: hex_color(&block.color) });
                    }
                }
            }
        }
        self.boxes.set_vec(boxes);
        self.paths.set_vec(paths);
        self.texts.set_vec(texts);
        // A mosaic stroke only grows: its new blocks are added, and every block is drawn afresh
        // only when something was undone or a new selection began.
        let shown = self.cells.row_count();
        if cells.len() >= shown && shown > 0 {
            for cell in cells.into_iter().skip(shown) {
                self.cells.push(cell);
            }
        } else {
            self.cells.set_vec(cells);
        }
    }
}

// ---- geometry --------------------------------------------------------------------------------

/// The eight handles of a selection: the corners and the middles of the sides, in the order the
/// overlay draws them.
fn handles(r: Rect) -> [(f32, f32); 8] {
    [
        (r.x, r.y),
        (r.x + r.w / 2.0, r.y),
        (r.x + r.w, r.y),
        (r.x, r.y + r.h / 2.0),
        (r.x + r.w, r.y + r.h / 2.0),
        (r.x, r.y + r.h),
        (r.x + r.w / 2.0, r.y + r.h),
        (r.x + r.w, r.y + r.h),
    ]
}

fn handle_cursor(handle: usize) -> ShotCursor {
    match handle {
        0 | 7 => ShotCursor::Nwse,
        2 | 5 => ShotCursor::Nesw,
        1 | 6 => ShotCursor::Ns,
        _ => ShotCursor::Ew,
    }
}

/// `from` with its handle dragged to `p`: the opposite side stays.
fn resized(from: Rect, handle: usize, p: (f32, f32)) -> Rect {
    let (mut left, mut top, mut right, mut bottom) = (from.x, from.y, from.x + from.w, from.y + from.h);
    match handle {
        0 | 3 | 5 => left = p.0,
        2 | 4 | 7 => right = p.0,
        _ => {}
    }
    match handle {
        0..=2 => top = p.1,
        5..=7 => bottom = p.1,
        _ => {}
    }
    let rect = Rect::between((left, top), (right, bottom));
    Rect { w: rect.w.max(SMALLEST), h: rect.h.max(SMALLEST), ..rect }
}

/// An arrow's end snapped to the nearest 45° from its start.
fn snapped(from: (f32, f32), to: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = (dx * dx + dy * dy).sqrt();
    if length == 0.0 {
        return to;
    }
    let angle = (dy.atan2(dx) / std::f32::consts::FRAC_PI_4).round() * std::f32::consts::FRAC_PI_4;
    (from.0 + length * angle.cos(), from.1 + length * angle.sin())
}

/// An arrow as two SVG paths: its shaft (stroked), and its head (a filled triangle with its tip
/// at the end).
pub fn arrow_paths(from: (f32, f32), to: (f32, f32), size: usize) -> (String, String) {
    let (head_length, head_width) = ARROW_HEAD[size.min(2)];
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = (dx * dx + dy * dy).sqrt().max(0.001);
    let (ux, uy) = (dx / length, dy / length);
    let base = (to.0 - ux * head_length, to.1 - uy * head_length);
    let (nx, ny) = (-uy * head_width / 2.0, ux * head_width / 2.0);
    let shaft_end = if length > head_length { base } else { from };
    let shaft = format!("M {} {} L {} {}", from.0, from.1, shaft_end.0, shaft_end.1);
    let head = format!("M {} {} L {} {} L {} {} Z", to.0, to.1, base.0 + nx, base.1 + ny, base.0 - nx, base.1 - ny);
    (shaft, head)
}

pub fn polyline(points: &[(f32, f32)]) -> String {
    let mut path = String::new();
    for (index, point) in points.iter().enumerate() {
        path.push_str(if index == 0 { "M " } else { " L " });
        path.push_str(&format!("{} {}", point.0, point.1));
    }
    if points.len() == 1 {
        path.push_str(&format!(" L {} {}", points[0].0 + 0.01, points[0].1));
    }
    path
}

fn distance_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length2 = dx * dx + dy * dy;
    let t = if length2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length2).clamp(0.0, 1.0) };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

/// A box, an arrow with some length, a stroke, a mosaic with blocks: something to keep.
fn drawn(annotation: &Annotation) -> bool {
    match &annotation.shape {
        Shape::Box { rect, .. } => rect.w >= SMALLEST && rect.h >= SMALLEST,
        Shape::Arrow { from, to } => (from.0 - to.0).abs() + (from.1 - to.1).abs() >= SMALLEST,
        Shape::Pen { points } => !points.is_empty(),
        Shape::Text { text, .. } => !text.is_empty(),
        Shape::Mosaic { cells, .. } => !cells.is_empty(),
    }
}

// ---- colours ---------------------------------------------------------------------------------

pub fn color_hex(index: usize) -> String {
    COLORS[index.min(5)].to_string()
}

pub fn halo_hex(index: usize) -> String {
    if matches!(index, 1 | 3) { "rgba(0,0,0,0.6)".to_string() } else { "rgba(255,255,255,0.85)".to_string() }
}

fn hex_color(hex: &str) -> slint::Color {
    slint::Color::from_argb_encoded(0xff00_0000 | u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0))
}

fn color_of(index: usize) -> slint::Color {
    hex_color(COLORS[index.min(5)])
}

fn halo_of(index: usize) -> slint::Color {
    if matches!(index, 1 | 3) { slint::Color::from_argb_u8(153, 0, 0, 0) } else { slint::Color::from_argb_u8(217, 255, 255, 255) }
}

fn pixel_hex(display: &Display, px: i64, py: i64) -> String {
    let (w, h) = display.px;
    if px < 0 || py < 0 || px >= w as i64 || py >= h as i64 {
        return "#000000".to_string();
    }
    let at = ((py as u32 * w + px as u32) * 4) as usize;
    format!("#{:02X}{:02X}{:02X}", display.rgba[at], display.rgba[at + 1], display.rgba[at + 2])
}

// ---- files -----------------------------------------------------------------------------------

/// "Screenshot 2026-10-07 at 11.02.15.png", as macOS names them.
fn file_name() -> String {
    Local::now().format("Screenshot %Y-%m-%d at %H.%M.%S.png").to_string()
}

fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join("finchgram-screenshots")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_are_read_written_and_shown() {
        let shortcut = Shortcut::parse("cmd+shift+a").unwrap();
        assert_eq!(shortcut.label(), "⌘⇧A");
        assert_eq!(shortcut.to_string(), "cmd+shift+a");
        assert_eq!(Shortcut::from_keys("S", false, true, false, true).unwrap().label(), "⌃⇧S");
        assert!(Shortcut::from_keys("a", false, false, false, true).is_none());
        assert!(Shortcut::parse("cmd+").is_none());
    }

    /// A session over one display of 1000 × 800 points, with one window on the screen and no
    /// overlays (nothing is shown).
    fn session() -> Session {
        let (width, height) = (1000, 800);
        Session {
            ui: slint::Weak::default(),
            displays: vec![Display {
                origin: (0.0, 0.0),
                size: (width as f32, height as f32),
                scale: 1.0,
                px: (width, height),
                rgba: vec![0; (width * height * 4) as usize],
                image: Image::default(),
            }],
            windows: Vec::new(),
            others: vec![platform::ScreenWindow { x: 100.0, y: 100.0, width: 400.0, height: 300.0 }],
            phase: Phase::Waiting,
            display: 0,
            selection: None,
            hover: None,
            pointer: (0, (0.0, 0.0)),
            tool: Tool::None,
            size: 1,
            color: 0,
            annotations: Vec::new(),
            redo: Vec::new(),
            draft: None,
            last_point: (0.0, 0.0),
            stroke_cells: HashSet::new(),
            text_at: (0.0, 0.0),
            hid_window: false,
            boxes: Rc::new(VecModel::default()),
            paths: Rc::new(VecModel::default()),
            texts: Rc::new(VecModel::default()),
            cells: Rc::new(VecModel::default()),
        }
    }

    #[test]
    fn a_click_takes_the_lit_window() {
        let window = Rect { x: 100.0, y: 100.0, w: 400.0, h: 300.0 };
        let mut session = session();
        session.moved(0, (200.0, 200.0), false);
        assert_eq!(session.hover, Some(window));
        // The button down takes nothing yet, and the window stays lit through a steady hand.
        session.pressed(0, (200.0, 200.0), false, false);
        session.moved(0, (201.0, 202.0), false);
        assert_eq!(session.selection, None);
        assert_eq!(session.hover, Some(window));
        session.released(0, (201.0, 202.0));
        assert_eq!(session.selection, Some(window));
        assert_eq!(session.phase, Phase::Settled);
    }

    #[test]
    fn a_drag_makes_a_selection_even_from_a_lit_window() {
        let mut session = session();
        session.moved(0, (200.0, 200.0), false);
        session.pressed(0, (200.0, 200.0), false, false);
        session.moved(0, (260.0, 250.0), false);
        assert_eq!(session.hover, None);
        session.released(0, (260.0, 250.0));
        assert_eq!(session.selection, Some(Rect { x: 200.0, y: 200.0, w: 60.0, h: 50.0 }));
        assert_eq!(session.phase, Phase::Settled);
    }

    #[test]
    fn a_click_on_nothing_keeps_waiting() {
        let mut session = session();
        session.moved(0, (800.0, 700.0), false);
        assert_eq!(session.hover, None);
        session.pressed(0, (800.0, 700.0), false, false);
        session.released(0, (800.0, 700.0));
        assert_eq!(session.selection, None);
        assert_eq!(session.phase, Phase::Waiting);
        // Back over the window, it is lit again and a click takes it.
        session.moved(0, (150.0, 150.0), false);
        session.pressed(0, (150.0, 150.0), false, false);
        session.released(0, (150.0, 150.0));
        assert_eq!(session.selection, Some(Rect { x: 100.0, y: 100.0, w: 400.0, h: 300.0 }));
    }

    #[test]
    fn a_resized_selection_keeps_its_far_side() {
        let from = Rect { x: 10.0, y: 10.0, w: 100.0, h: 50.0 };
        assert_eq!(resized(from, 7, (150.0, 90.0)), Rect { x: 10.0, y: 10.0, w: 140.0, h: 80.0 });
        assert_eq!(resized(from, 0, (0.0, 0.0)), Rect { x: 0.0, y: 0.0, w: 110.0, h: 60.0 });
        // Dragged past the far side: the rectangle flips rather than vanishing.
        assert_eq!(resized(from, 4, (0.0, 30.0)), Rect { x: 0.0, y: 10.0, w: 10.0, h: 50.0 });
    }

    #[test]
    fn an_arrow_has_a_shaft_and_a_head_at_its_end() {
        let (shaft, head) = arrow_paths((0.0, 0.0), (100.0, 0.0), 1);
        assert_eq!(shaft, "M 0 0 L 82 0");
        assert_eq!(head, "M 100 0 L 82 7.5 L 82 -7.5 Z");
        // Snapped to the horizontal, as long as the pointer is far.
        let (x, y) = snapped((0.0, 0.0), (100.0, 10.0));
        assert!((x - 100.499).abs() < 0.01 && y.abs() < 0.001, "{x} {y}");
    }
}
