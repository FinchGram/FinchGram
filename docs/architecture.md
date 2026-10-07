# Architecture: the app is a shell, TDLib does Telegram

[中文](architecture.zh-Hans.md)

Status: agreed direction (2026-09-29), after Coova Studio's architecture. The shell,
finchgram-tdlib and libmpv run on macOS; logging in, the chat list, text messages, photos and videos,
sending photos, videos and files, what can be done with a message (its menu: replying, editing,
copying, forwarding, reporting, deleting, choosing several) and notifications of new messages work,
in the design's three themes.
The decisions still open are listed at the end.

## Why

- A Telegram client is two things: the protocol with its local state (MTProto, encryption, the
  database of chats and messages, files, keeping in step with the servers), and the pages. TDLib,
  Telegram's own library, does the first completely. FinchGram's work is the second, and the media
  center.
- TDLib is C++. Kept in a program of its own, it stays out of the app's code: the app builds with
  plain `cargo build`, with no C++ toolchain and no foreign function interface, and a crash in TDLib
  does not take the window down.
- FinchGram runs on macOS first. Windows and Linux will follow; the rules below keep the door open.

## The idea

```
FinchGram (the shell: Rust + Slint)
│  pages · the app's state · settings · updates · notifications
│  talks to Telegram only through src/telegram/
▼
finchgram-tdlib   a separate program next to the executable, as ffmpeg is for Coova Studio:
                  TDLib (pinned) and a small host that passes TDLib's JSON over standard input
                  and output
```

## Terms

- **Shell**: the app itself: the pages, the app's state, settings, updates. It knows what a chat
  looks like and what the user can do with it, not how Telegram's protocol works.
- **finchgram-tdlib**: TDLib built from pinned sources by this repository's vendor build, with our
  host (`vendor/tdlib/host/main.cpp`). The only thing that talks to Telegram.
- **Adapter**: `src/telegram/`, the only code that talks to finchgram-tdlib.
- **Platform layer**: the few parts of the shell that differ from one OS to another: notifications,
  the tray, the dock badge, secure storage, opening links, showing files in the file manager,
  updates.

## Principles

1. The shell never speaks Telegram's protocol and never links TDLib. Everything goes through
   finchgram-tdlib and the adapter.
2. TDLib keeps its own JSON interface, as FFmpeg keeps its command line: requests, answers and
   updates are TDLib's objects exactly as its td_api.tl defines them, one per line. There is no
   protocol of our own on top.
3. One program runs one TDLib client, that is one account. It ends when its standard input closes,
   after TDLib has closed and written its database out; after a log out it ends by itself.
4. A crash does not take the window down. When finchgram-tdlib ends unexpectedly, the requests still
   waiting fail and the adapter starts it again: TDLib's database is on disk, so it carries on. One
   that ends within 30 seconds of starting is broken, and the app says so instead of starting it
   over and over.
5. The version is part of the contract. `src/telegram/api.rs` is written against td_api.tl of one
   TDLib version, the one pinned in `vendor/tdlib/build.sh`. The adapter's first request asks
   finchgram-tdlib for its version; anything else is a packaging error. The types are written by
   hand, only for what FinchGram uses, and fields TDLib adds are ignored.
6. Files are passed by path. TDLib downloads into its files directory and reports the path; the
   shell reads from there. Nothing big crosses the pipe.
7. The development conventions hold ([conventions.md](conventions.md)): everything ships with the
   app, built from pinned sources by our own CI, and nothing is taken from the user's machine.
8. Platform code lives only in the platform layer. Everything else is shared Rust.
9. Telegram's terms for API clients are part of the design (core.telegram.org/api/terms): our own
   api_id; no "Telegram" in the name and not its logo; the app says that it uses the Telegram API;
   official sponsored messages in channels are shown; nothing interferes with read receipts, typing,
   online status or self-destructing messages; nothing obtained from Telegram trains or feeds AI.

## How the shell talks to finchgram-tdlib

- The program is looked for in one place only: next to the executable (`Contents/MacOS/` in the
  .app; `target/<profile>/` for `cargo run`, where build.rs copies it from `vendor/tdlib/bin/`).
- Standard input: requests, one JSON object per line. Each carries an `"@extra"` number; TDLib
  copies it into the answer, which goes to the callback given with the request.
- Standard output: answers and updates, one per line. A reader thread parses them and wakes the UI
  thread only when its inbox was empty, so a burst of updates arrives as one batch. Updates nothing
  follows yet are dropped there.
- Standard error: TDLib's log (errors only, until the shell asks for more).
- `finchgram-tdlib --version` prints TDLib's version and commit; the vendor build checks it.

## Threads

- The UI thread runs Slint's event loop and owns the app's state: what TDLib has said about chats,
  users and messages, and the Slint models the pages show. The state changes there, from updates,
  in batches, with no locks.
- Each finchgram-tdlib has a writer thread and a reader thread.
- Work that takes time (decoding an image, reading a file) runs on a thread of its own and reports
  back through the event loop, as in Coova Studio. Images are decoded into a `SharedPixelBuffer` off
  the UI thread and become a `slint::Image` on it.
- There is no async runtime: TDLib does the networking.

## The pages: three themes

The design has three themes: Workbench (the default), Broadsheet and Terminal. Each has a chat
window of its own (`ui/workbench/`, `ui/broadsheet/`, `ui/terminal/`); logging in, settings and the
profile are pages the three share (`ui/pages/`), in the theme's colours, type and shapes
(`ui/look.slint`).

- A theme is only pages. All three read the same globals (`ui/state.slint`, `ui/telegram.slint`)
  and call the same callbacks, which the same Rust code fills and answers. A feature is written once
  in Rust and drawn three times; the design's rule is Workbench first, then the other two, so that
  the themes stay in step.
- Switching is immediate (Settings → Appearance, or the View menu), with no restart: the window
  swaps its pages and nothing else changes, not even the open chat.
- What the design shows but FinchGram does not have yet is greyed out, never left to look working.

## In the shell's code

- `src/telegram/`: the adapter. `process.rs` runs the program, `api.rs` has TDLib's types that we
  use, `mod.rs` sends requests, hands out answers, starts the program again and passes updates on.
  `store.rs` keeps what TDLib has said (chats, users, groups, folders, the messages of the open
  chats) and brings the pages' models up to date after each batch, touching only the rows that
  changed. The open chat's rows are its newest 100 messages, more as the view goes up: the pages
  lay every row out and measure it again at each change among them, so their number is what each
  new message costs. `login.rs`, `chats.rs`,
  `conversation.rs`, `actions.rs` (what can be done with a message), `account.rs` and
  `password.rs` (two-step verification) do what the pages ask for; `files.rs` downloads files,
  `avatars.rs` fetches the photos of chats and people for the rows that show them (a letter square
  until then, or for good when there is no photo), `viewer.rs` fills the media viewer, and
  `rich_text.rs` turns a message's formatted text (bold, links, …) into Slint's styled text.
- Notifications (`notifications.rs`, Settings → Notifications & sounds) are TDLib's: switched on
  with its `notification_group_count_max` option, TDLib decides what is worth one. It follows the
  chats' mutes and each kind of chat's setting (Telegram's own, for the account), waits a moment while
  another of the account's devices is in use, and takes a notification back once its message is
  read anywhere. The shell shows what TDLib adds (not what the user is looking at: the open chat
  while the window is in front) and takes back what TDLib removes; a click opens the chat.
- What Telegram's own apps let others see, FinchGram does too (the API terms): a message is read
  once it is seen (`conversation.rs`, viewMessages), the chat sees that the user is typing while
  they write (sendChatAction), and the account is online while the window is in front and in use
  (`online.rs`, TDLib's `online` option). Telegram goes by the last to hold back the notifications
  of the user's other devices, and TDLib to time FinchGram's. A photo or video sent to self-destruct
  (to be seen once, or for a time) is shown blurred until the user opens it in the media viewer,
  which tells TDLib (`viewer.rs`, openMessageContent); it then expires as Telegram's own apps let it.
  The viewer offers saving only where TDLib says the content may be saved (the chat may restrict it).
- Sending photos, videos and files (`attachments.rs`, the design's sixth round): the paperclip's
  menu opens the system's open panel; files dragged onto the window (`mod.rs`, winit's HoveredFile
  and DroppedFile, [drag-and-drop.md](drag-and-drop.md)) and pictures or files pasted with ⌘V come
  the same way. A card shows them before they go, with a caption and the choices Telegram's own
  apps give: as a photo or as a file, together as an album of up to ten, a self-destruct timer in a
  private chat, without sound. A picture goes as a photo and a video as a video only when its file
  is one Telegram takes as such (JPEG, PNG, WebP; MP4, M4V, MOV); anything else goes as a file, as it
  is. A video's length, size and still come from a libmpv of its own on another thread
  (`player/probe.rs`). The sending is TDLib's sendMessage and sendMessageAlbum; the messages show how
  far they have got from updateFile (`store.rs`), and a file, ours or theirs, as the design's card,
  which downloads it or shows it in the Finder. What may be sent follows the chat's permissions and
  ours in it (`store.rs`, send_rights). Caption length and file size follow Telegram's limits
  (TDLib's `message_caption_length_max` and `is_premium` options).
- A message's menu opens on a right click anywhere on the message. Its formatted words (Slint's
  StyledText) keep every click to themselves, so the right click is seen in the window's own events
  (`mod.rs`, through winit), counted in a global, and the row under the pointer asks for its menu.
  What the menu offers is what TDLib says can be done with the message (getMessageProperties).
- A chat's menu (a right click on it in the lists, `chats.rs`) mutes or pins it, marks it as read,
  and puts it into one of the account's folders or takes it out. TDLib has no request for the
  last: the folder is fetched whole (getChatFolder), its chosen chats are changed and it is sent
  back (editChatFolder). A folder that takes chats by kind (contacts, groups, …) would keep a chat
  taken out, so there the chat is excluded by name as well.
- `src/player/`: video through libmpv, drawn into the window (the media viewer's player).
- `src/platform/`: the platform layer. So far: what the account's list of sessions calls this
  device, the transparent title bar on macOS, opening links, the clipboard (words and pictures);
  and on macOS (Settings → General)
  staying in the Dock when the window is closed, as the design's "When closing the window: Minimize
  to tray" has it (the Dock icon shows the window again, and Quit lets TDLib close first), the icon
  in the menu bar, and launching at login (SMAppService, macOS 13 and later); notifications through
  the system's UserNotifications framework (only for the app as a bundle, so not from `cargo run`),
  the unread count on the Dock icon, and its bounce (Settings → Notifications & sounds).
- `src/update.rs`: the self-updater ([conventions.md](conventions.md), section 3).
- `src/settings.rs`, `src/i18n.rs`, `src/fonts.rs` (the UI fonts, compiled into the executable).
- `src/images.rs`: pictures (photos, video stills, the photos of chats and people, the tiny previews
  in messages), decoded off the UI thread, and a cache of them.
- `src/screenshots.rs`: every page in every theme, light and dark, drawn to a PNG with made-up data
  by Slint's software renderer (`cargo test screenshots -- --ignored`).
- `src/telegram/timing.rs`: how long the pages take to follow the store, with many made-up chats and
  messages, drawn the same way (`cargo test --release refresh_timing -- --ignored --nocapture`).
- `ui/`: `app.slint` (the window: its menus and which page shows), `state.slint` and
  `telegram.slint` (the globals Rust and the pages share), `look.slint`, `format.slint` (dates,
  counts and kinds of message in the UI language), `widgets.slint` and `chat.slint` (shared parts),
  `viewer.slint` (the media viewer, over the whole window), `pages/`, and the three themes' folders.

## Platforms

| | macOS | Windows | Linux |
|---|---|---|---|
| When | now | planned | later |
| finchgram-tdlib | built by our CI (Apple silicon) | planned | planned |
| Updates | our updater | our updater | our updater or packages |

## Not now

- **Several accounts.** One finchgram-tdlib per account, each with a database directory of its own,
  and the shell switching between them. Needs a design first.
- **Calls.** TDLib only does the signalling; the media part (tgcalls) is a large project of its own.
- **Plugins by other people.** They first need a trust model: who signs them, what the user
  approves, how they are sandboxed.

## Open questions

1. **Database encryption**: TDLib can encrypt its database with a key, which would live in the
   system's secure storage (Keychain). Until then the database is protected by the user's account
   only.
2. **Where downloaded files go**: next to the database for now. A size limit and a cache folder
   (`~/Library/Caches`) need a design.
3. **Media playback** (the media center): voice messages (Opus), video, GIFs and animated stickers
   (WebM, Lottie). Decided on 2026-09-30: mpv. It comes as libmpv, built from pinned sources by
   `vendor/mpv/build.sh` as finchgram-tdlib is: mpv with FFmpeg, libplacebo and libass (FreeType,
   FriBidi, HarfBuzz), all static in one library that depends only on macOS; the app links it and
   carries it in its bundle's Frameworks folder. Each time the window renders, mpv draws the frame into
   an OpenGL texture of ours inside Slint's own context (`src/player/`), and decodes with VideoToolbox
   where it can; the media viewer shows the texture. The price: libmpv decodes inside the app's
   process, so a crafted video that breaks a decoder takes the window with it. FFmpeg is
   built without network code, encoders or devices, and follows upstream; should that not be enough,
   playback moves into a sandboxed helper process that hands its picture over (IOSurface on macOS).
   Lottie stickers need something else (rlottie), and WebM stickers a VP9 decoder that keeps their
   transparency (libvpx; FFmpeg's own leaves it out). Until then a moving sticker shows its still
   thumbnail; still stickers show as they are.
4. **Rendering**: Slint's default renderer (FemtoVG) for now. The design's typefaces are bundled,
   Chinese included (Noto Sans SC); Japanese, Korean and colour emoji in messages come from the
   system's fonts until the UI speaks those languages. Skia would need building from source: its
   Rust bindings download a prebuilt library by default, which conventions.md does not allow.
