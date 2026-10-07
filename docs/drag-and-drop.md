# Drag and drop: files dragged from the Finder reach the window

[中文](drag-and-drop.zh-Hans.md)

Status: verified (2026-10-06) with a probe build on the pinned Slint 1.18.1 and winit 0.30.13, on
macOS 26. Nothing of it is in the app yet: it was checked for sending attachments (photos, videos,
files), which is designed first and built after.

## What was asked

Can FinchGram take files dropped onto its window, as Telegram's own apps do, with Slint? The design
for sending attachments wants a drop zone over the chat.

## How it was checked

1. The sources in the Cargo registry were read: winit 0.30.13's macOS window
   (`src/platform_impl/macos/window_delegate.rs`), and Slint 1.18.1's winit backend
   (`i-slint-backend-winit`: `lib.rs`, `winitwindowadapter.rs`) and compiler built-ins.
2. A probe build: three lines in the window event hook that `telegram::start` registers
   (`src/telegram/mod.rs`), printing `HoveredFile`, `HoveredFileCancelled` and `DroppedFile`; run
   with `cargo run` on the login page (this machine has no api_id), a JPEG dragged from the Finder
   onto the window.

## What was found

- **The window gets the files.** winit registers the window for `NSFilenamesPboardType` drags and
  implements `NSDraggingDestination` itself: a drag entering gives `WindowEvent::HoveredFile(path)`,
  one per file; releasing gives `WindowEvent::DroppedFile(path)`, one per file, with the full path;
  leaving gives `HoveredFileCancelled`. The probe logged one hover and one drop, with the file's
  path.
- **Not through Slint's `DropArea`.** Slint 1.18's winit backend never maps these events, so on
  macOS a `DropArea` only takes drags that start in a `DragArea` of the app itself. The way in is
  `WinitWindowAccessor::on_winit_window_event`, which hands over every winit window event.
- **One hook per window.** A window holds one event filter; registering another replaces the first
  without a word. `telegram::start` already registers it (the window's focus for the online status,
  input, right clicks), so dropped files have to be handled there, or in something it calls. A probe
  registered in `main.rs` before `telegram::start` logged nothing, for this reason.
- **No position.** winit 0.30 implements no `draggingUpdated:`, so there is no cursor position
  while a drag hovers, and `DroppedFile` carries none. The window can know that files are over it
  and that they were dropped, not where.

## What follows for sending attachments

- One drop zone over the whole window, shown from the first `HoveredFile` until
  `HoveredFileCancelled` or the drop: not Telegram Desktop's two ("send as photos" and "send as
  files"). The choice between a photo and a file goes into the box that follows the drop, where it
  is anyway.
- Files arrive as paths, so what is dropped is sent as a file chosen in the open panel is
  (`inputFileLocal`).
- The drop goes to the open chat, wherever over the window it was.

## Not checked

- Dragging from apps other than the Finder. Some (Photos, Mail) offer promised files rather than
  paths; winit refuses a drag without `NSFilenamesPboardType`, so those would show as not allowed.
- Pasting a picture from the clipboard (⌘V): a separate path, through Slint's text input, not this
  hook.
- Several files at once: the source sends one event per file, as expected; not tried.
