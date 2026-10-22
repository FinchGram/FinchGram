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
- There is no async runtime for the app's own work: TDLib does Telegram's networking. The one
  exception, since 2026-10-16, is the downloads engine's HTTP client (`src/downloads/http.rs`):
  hyper on a tokio runtime of its own, on two threads named "http", because HTTP/2 (which PikPak
  and the like need for speed: one connection whose streams keep it busy) only exists in Rust as
  async code. The engine blocks on that runtime from the transfer's own thread, so nothing outside
  `src/downloads/` is async, and the UI thread never touches it.
- The MCP server (`src/mcp/`) has a thread listening on its socket and one per connection. A
  connection's thread reads a line and answers it; a tool's work goes to the UI thread
  (`upgrade_in_event_loop`), where the store, TDLib and the page are, and its outcome comes back
  on a channel. So does the question whether a program may connect: the thread waits while the
  dialog asks the user.
- A panic on the UI thread ends the app: Slint calls into the app through `extern "C"` functions,
  which no panic unwinds through, so Rust aborts there, and macOS's crash report shows abort() and
  nothing of the cause. The panic's message and place go into `panic.log` next to settings.toml
  first (`src/main.rs`), since the app started from the Dock has no terminal to print them to. The
  store (`store.rs`) is one RefCell: `store::with` inside `store::with` is such a panic, so a helper
  that needs the store while its caller holds it takes `&Store` (contacts::matching).

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
- Light or dark is each theme's own choice (`[appearance]` in settings.toml): Workbench and
  Broadsheet follow the system until told otherwise, Terminal starts dark. A theme switched to
  comes back as it was left.
- What the design shows but FinchGram does not have yet is greyed out, never left to look working.

## In the shell's code

- `src/telegram/`: the adapter. `process.rs` runs the program, `api.rs` has TDLib's types that we
  use, `mod.rs` sends requests, hands out answers, starts the program again and passes updates on.
  `store.rs` keeps what TDLib has said (chats, users, groups, folders, the messages of the open
  chats) and brings the pages' models up to date after each batch, touching only the rows that
  changed. The open chat's rows are its newest 100 messages, more as the view goes up: the pages
  lay every row out and measure it again at each change among them, so their number is what each
  new message costs. `login.rs`, `chats.rs`,
  `conversation.rs`, `actions.rs` (what can be done with a message), `bots.rs` (what bots put in a
  chat beyond messages, and the pinned message bar), `reactions.rs`, `topics.rs`, `drafts.rs`,
  `share.rs`, `admin.rs` (the fifth round, below; `errors.rs` puts Telegram's refusals into the
  pages' words), `account.rs` and
  `password.rs` (two-step verification) do what the pages ask for; `files.rs` downloads files,
  `avatars.rs` fetches the photos of chats and people for the rows that show them (a letter square
  until then, or for good when there is no photo), `viewer.rs` fills the media viewer and keeps its volume (the
  speaker's tray, ↑ ↓ and the wheel; `video_volume` and `video_muted` in settings.toml, for the next
  video and the next start), and `rich_text.rs` turns a message's formatted text (bold, links, …) into Slint's styled text.
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
- The screenshot tool (`screenshot/`, the design's seventh round): the scissors in the composer,
  or ⌘⇧A (Settings → General → Screenshots, where the keys can be changed), hide the window if the
  setting says so (off by default), have the system take every display (`platform/`, macOS's own
  screencapture, once Screen Recording is allowed; a card says where to allow it), and cover each
  display with a borderless window of its own (`ui/shot.slint`) showing the frozen screen, dimmed
  all over. A drag makes a selection, which shows undimmed; a click takes the window under the
  pointer whole (the system's list of them, FinchGram's own among them unless it hid). The user
  adjusts the selection by its handles and the arrow keys, and draws on it: rectangle, ellipse,
  arrow, pen, text and mosaic, in three sizes and six colours, with undo. Rust owns the state and
  the annotations; the overlay only draws them and reports the pointer and the keys. Done draws
  the annotations into the picture with resvg (`screenshot/export.rs`, in the bundled fonts, the
  same in every theme) and opens the send card with it as "Send Screenshot"; Copy puts it on the
  clipboard; Save writes it to the Downloads folder, named as macOS names screenshots.
- A message's menu opens on a right click anywhere on the message. Its formatted words (Slint's
  StyledText) keep every click to themselves, so the right click is seen in the window's own events
  (`mod.rs`, through winit), counted in a global, and the row under the pointer asks for its menu.
  What the menu offers is what TDLib says can be done with the message (getMessageProperties).
- A chat's menu (a right click on it in the lists, `chats.rs`) mutes or pins it, copies its id
  (TDLib's chat id, the number bots and scripts know the chat by; the account's own is on the
  profile page), and puts it into one of the account's folders or takes it out. TDLib has no
  request for the last: the folder is fetched whole (getChatFolder), its chosen chats are changed
  and it is sent back (editChatFolder). A folder that takes chats by kind (contacts, groups, …)
  would keep a chat taken out, so there the chat is excluded by name as well. The menu also does
  what Telegram's own apps do with a chat: block or unblock a user (setMessageSenderBlockList),
  report the chat (reportChat, the steps a message's report goes through), leave a group or
  channel (leaveChat), delete a chat with a user or a bot (deleteChatHistory, for the other side
  too when TDLib says it can be); blocking, leaving and deleting ask first, and a chat left or
  deleted closes its tab.
  A folder's menu (a right click on its tab, or on its header in Workbench's tree) marks every
  chat of it as read: TDLib goes through the list (readChatList), and the read receipts are real.
  Its Sort by orders every folder's chats under the pinned ones, which keep Telegram's order: by
  the newest message (Telegram's own order), by name (as the UI language orders names: macOS's
  own comparison in that locale, so Chinese by pinyin and digits by their value,
  `platform::NameOrder`), or by unread messages (muted chats count as read, as their grey badges
  say). One choice for all folders, `chat_order` in settings.toml; `store.rs` sorts (chats_in).
- Saved Messages, the chat with oneself, is a fixed row above the chat list in the three themes,
  there before the account has used it: TDLib makes the chat (createPrivateChat) when the row is
  clicked. It is not repeated in the folders below, and the search over everything matches the
  names the lists show (Saved Messages, Deleted Account), not TDLib's titles.
- Searching in the open chat (`search.rs`: the header's magnifying glass, Terminal's "[search]",
  ⌘F) asks TDLib for the messages with the words (searchChatMessages, a hundred at a time, newest
  first; Telegram's servers answer, so the search is theirs, by whole words) and goes to them one
  by one, ⏎ up the chat and ⇧⏎ down, each lit up as a reply's original is. A match far above the
  messages loaded is not paged to: the page around it takes their place (`conversation.rs`,
  `History::has_newer`), older pages come as before and newer ones as the view comes down, until
  the chat's end; a message sent from there, or the chat opened again, starts from the end anew.
  A reply's original more than two pages up is reached the same way.
- The search over everything (`global_search.rs`, the design's compose.js round: the title bar's
  box, ⌘K, Terminal's /search) is a window over the chats. With nothing typed it shows the chats
  used last; with words, the chats here whose names have them and the contacts (`contacts.rs`:
  TDLib's getContacts, matched by name, username, or phone number when the words are one), then
  what Telegram's servers find after a pause: public users, groups and channels by username and
  title (searchPublicChats, less those joined and the contacts), a person by their whole phone
  number (searchUserByPhoneNumber, which finds only those who let others find them so; a number
  nobody has offers to add them as a contact), and messages (searchMessages). A public group or
  channel shows a card first, with its description (getSupergroupFullInfo), and joins from there
  (joinChat); a person opens as a chat (createPrivateChat), a message opens its chat at the
  message. ↑↓ choose, ⏎ opens, Esc closes.
- The pencil's menu (`new_chat.rs`; Terminal's /group, /channel, /dm, /contact): a new group takes
  its members, then its name and photo, and is a basic group with its members
  (createNewBasicGroupChat) or a supergroup without any or with more than a basic group holds
  (createNewSupergroupChat, the members added after); a new channel is made at its first step, as
  Telegram's own apps make it, so that the second can check and set its public link
  (checkChatUsername after Telegram's own rules, setSupergroupUsername) or show its invite link
  (the primary one, else replacePrimaryChatInviteLink, with a copy button), and the third invites
  subscribers (addChatMembers); a new chat is a contact's; a new contact is added by phone number
  (importContacts), a number with no account said so. A photo chosen goes up once the chat exists
  (setChatPhoto, cut square and made 640 pixels wide first). What Telegram refuses is shown in the
  dialog, which stays open; the service messages a new chat shows are the servers' own.
- Bots (`bots.rs`; no design yet, so the pages draw these plainly in their own colours): the
  buttons under a bot's message (replyMarkupInlineKeyboard) share their row's width under the
  bubble, and do what their kind says as Telegram's own apps do it. A callback asks the bot
  (getCallbackQueryAnswer; the button turns meanwhile) and shows its words as a notice, or as a
  dialog when the bot asks for one, or opens its link; a link (↗) asks first, with its site, as any
  link whose words are not its address does; a login button asks Telegram what the site wants
  (getLoginUrlInfo) and the user before logging in (getLoginUrl); a web app opens in the browser
  (openWebApp or getWebAppUrl for the address: FinchGram has no web view); switch-inline writes
  "@bot words" into this chat or one picked from the forward picker; copy copies; a user button
  opens their chat. The buttons follow the message's edits (updateMessageEdited carries the new
  markup) and go when it loses them. A bot's keyboard (replyMarkupShowKeyboard) sits above the
  composer, folded away with the keyboard button or after one use when the bot asked for that, and
  goes when the bot takes it back (updateChatReplyMarkup); its keys send their words, the one for
  the phone number asks first and sends a contact card, a location, a poll or a chat to choose are
  said to be beyond FinchGram yet, as paying is. "/" alone or "/words" in the composer (Terminal:
  on its command line, among its own commands) offers the commands of the chat's bot, or of the
  bots in the group (botInfo from getUserFullInfo, the groups' full info), and a bot's menu
  button stands left of the composer's tools: its web app, or its commands. A chat's newest pinned
  message (getChatPinnedMessage, asked again at updateMessageIsPinned) is a bar under the header
  that goes to it. A chat that protects its content (has_protected_content: no forwarding, no
  saving, and Telegram's apps keep it out of screenshots) is kept out of screen captures while it
  is in front: the window's sharing type is set to none (`platform::set_window_capturable`), so
  that screenshots and screen recordings show it blank, as macOS does for such windows.
- The design's fifth round (ui/extras.slint holds what the themes share: the chips, the picker,
  the round video message, the contact card, the topics, the settings panel's group part, the
  share panel, and their words):
  - Reactions (`reactions.rs`): the reactions to a message are chips under it, ours lit; the
    smiley by the message opens a picker with what the chat allows, in Telegram's order for the
    message (getMessageAvailableReactions), and a double click gives the account's default
    reaction (updateDefaultReactionType). addMessageReaction and removeMessageReaction; TDLib keeps
    to how many one person may give here (chatAvailableReactions.max_reaction_count: one, three
    with Premium, fewer where the chat says), and the picker says so. A custom emoji reaction is
    drawn as the emoji it stands for (getCustomEmojiStickers, asked once), a paid one as a star.
    Channel posts take reactions too.
  - Topics (`topics.rs`): a group with topics (supergroup.is_forum) opens on its list of topics
    (getForumTopics): icon, name, last message, unread count, pinned, closed. A topic opened
    shows its messages alone (getForumTopicHistory; `History::topic_id` keeps messages of other
    topics out of the rows) with "All topics / the topic" under the header; what we send there
    goes with the topic (messageTopicForum). A closed topic takes no messages, and a line says so
    in place of the composer. A new topic takes a name and one of Telegram's icons
    (getForumTopicDefaultIcons, createForumTopic). updateForumTopicInfo and updateForumTopic are
    followed, and a changed topic's row is fetched whole (getForumTopic), since its unread count
    is not in the updates. A topic's icon is its custom emoji's emoji, "#" for the general one.
  - Round video messages (messageVideoNote): a 180 px circle with the still, the play button and
    the length; a click plays it in place through the same player as the viewer (`viewer.rs`:
    the frame is Viewer.frame while Conversation.round-playing names the message), with the ring
    in the accent while it plays; TDLib is told it was watched (openMessageContent). Recording
    voice or video messages is not in FinchGram: there is no capture and no Opus encoder yet, so
    the composer's microphone button is greyed.
  - Contact cards (messageContact; `contacts.rs`): who, their number, whether they are on
    Telegram; Message opens a chat with them (Invite, for someone not on Telegram, puts
    Telegram's invitation on the clipboard: a desktop sends no text messages), Add contact
    (addContact, or importContacts by the number), Share (the share panel).
  - Sharing (`share.rs`): a post's share link, or a card's Share, opens the panel: what is
    shared, a search over the chats we can write in (channels and groups with topics left out),
    several chosen at once, words of our own sent first, "Send to N chats". Posts and messages
    are forwarded (forwardMessages), a contact goes as a card (inputMessageContact). A chat that
    protects its content (has_protected_content) cannot be shared from, and the page says so;
    what Telegram refuses (CHAT_FORWARDS_RESTRICTED, FLOOD_WAIT_N) stays in the panel.
  - Drafts (`drafts.rs`): the composer is one; when another chat comes to the front, its words
    are saved for the chat that leaves (setChatDraftMessage) and the chat that comes puts its own
    back; writing saves after a pause too. updateChatDraftMessage brings what was written on
    another device. The chat list shows a draft in place of the last message (Workbench's tree
    marks the chat "Draft"; Terminal's list "✎").
  - Slow mode and the group's administration (`admin.rs`): where a group has slow mode, the
    composer counts the wait down after a message (supergroupFullInfo.slow_mode_delay_expires_in,
    and SLOWMODE_WAIT_N when Telegram refuses one) and takes none until it is over;
    administrators are not held. The chat's settings panel (the header's sidebar icon) shows a
    member the group's rules (slow mode, what administrators turned off) and an owner or
    administrator the controls: slow mode (setChatSlowModeDelay), what members may do
    (setChatPermissions), approving new members (toggleSupergroupJoinByRequest), restricting
    saving and forwarding (toggleChatHasProtectedContent), the history for new members
    (toggleSupergroupIsAllHistoryAvailable), hiding the member list (100 members or more,
    toggleSupergroupHasHiddenMembers) and aggressive anti-spam (200 or more,
    toggleSupergroupHasAggressiveAntiSpamEnabled). What Telegram refuses (CHAT_ADMIN_REQUIRED,
    …) is shown in the panel; the rest of the design's panel (files, scheduled messages,
    per-chat privacy) is not in FinchGram yet.
  - Not in this round: payments (a Buy button says so), polls, locations and choosing a chat for
    a bot; the mobile themes (FinchGram has no mobile app).
- The MCP server (`src/mcp/`, [mcp.md](mcp.md)): AI assistants' apps (Claude Desktop, Claude
  Code, …) read the user's chats and act in them through the running app, with the Model Context
  Protocol. The app listens on a Unix socket in its own folder; `finchgram mcp` (the same
  executable) is the stdio bridge an assistant's app runs. `protocol.rs` is JSON-RPC and MCP's
  methods, `tools.rs` the tools as the assistant sees them, and `src/telegram/tools.rs` what each
  does with the store and TDLib, on the UI thread, within what Settings → AI access allows (off by
  default). `access.rs` is that page and the dialog that asks the user to allow an app the first
  time it connects (ui/ai-access.slint, ui/pages/ai-access.slint, in every theme): the switch,
  which chats, what an assistant may write, the apps allowed or turned away, the log. Reading
  marks nothing as read; secret chats, self-destructing media and the media of chats that
  restrict saving are never handed out.
- The Downloads page (`src/downloads/`, [downloads.md](downloads.md)): a WebDAV client for a NAS,
  a seedbox, PikPak, Alist/OpenList or a folder shared with `vget serve`, read-only. `webdav.rs`
  lists folders (PROPFIND), `transfer.rs` fetches a file in chunks of 8 MB over up to 12 connections
  into a `.part` file, going on from where it stopped, as vget does; `mod.rs` is the page's state on
  the UI thread, a "webdav" thread for listings and one thread per file on its way, two at a time.
  The servers with their passwords go into the app's database (below), the folder into settings.toml; a downloaded
  photo or video opens in the media viewer (`viewer.rs`, `view_file`). The page needs no account.
- `src/player/`: video through libmpv, drawn into the window (the media viewer's player).
- `src/platform/`: the platform layer. So far: what the account's list of sessions calls this
  device, the transparent title bar on macOS, opening links, the clipboard (words and pictures);
  and on macOS (Settings → General)
  staying in the Dock when the window is closed, as the design's "When closing the window: Minimize
  to tray" has it (the Dock icon shows the window again, and Quit lets TDLib close first), the icon
  in the menu bar, and launching at login (SMAppService, macOS 13 and later); notifications through
  the system's UserNotifications framework (only for the app as a bundle, so not from `cargo run`),
  the unread count on the Dock icon, and its bounce (Settings → Notifications & sounds); the folder
  chooser for the Downloads page, and the Keychain, where the keys of the two databases are.
- `src/update.rs`: the self-updater ([conventions.md](conventions.md), section 3).
- `src/settings.rs`, `src/i18n.rs`, `src/fonts.rs` (the UI fonts, compiled into the executable).
- `src/images.rs`: pictures (photos, video stills, the photos of chats and people, the tiny previews
  in messages), decoded off the UI thread, and a cache of them.
- `src/screenshots.rs`: every page in every theme, light and dark, drawn to a PNG with made-up data
  by Slint's software renderer (`cargo test screenshots -- --ignored`).
- `src/telegram/timing.rs`: how long the pages take to follow the store, with many made-up chats and
  messages, drawn the same way (`cargo test --release refresh_timing -- --ignored --nocapture`).
- `ui/`: `app.slint` (the window: its menus and which page shows), `state.slint` and
  `telegram.slint` and `downloads.slint` (the globals Rust and the pages share), `look.slint`, `format.slint` (dates,
  counts and kinds of message in the UI language), `widgets.slint` and `chat.slint` (shared parts),
  `viewer.slint` (the media viewer, over the whole window), `pages/`, and the three themes' folders.

## The app's database

What FinchGram keeps that is neither a preference nor Telegram's goes into a database of its own:
`finchgram.db` next to settings.toml (`src/database.rs`), SQLite encrypted as a whole with
SQLCipher, built from the rusqlite crate's bundled sources with Apple's CommonCrypto on macOS. Its
key is random, made the first time and kept in the Keychain under "FinchGram" (the account
"database"), where only this app reads it; TDLib's database has a key of its own kept the same way
(the account "tdlib"), since 0.4.18. The rule: preferences go into settings.toml, which stays small
and readable by hand; records go into the database; Telegram's own state is TDLib's. Today the
database holds the WebDAV servers with their passwords (docs/downloads.md) and the programs
allowed or turned away at AI access (docs/mcp.md); what comes after (a download queue that outlives
a restart, logs, per-chat settings) gets a table when it is built. No table carries data over
from an earlier layout, and nothing stamps the layout's version: the maintainer is the one user,
and re-entering a record beats keeping code for it. With other users that turns around: a change
of layout then comes with its migration, told apart by SQLite's own `user_version`.

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
- **Voice input.** Dictation into the composer: the user's own OpenAI key first, later a plan,
  servers the user runs, the operating systems' own recognizers and a router between them. The
  five steps and the facts behind them are in [voice-input.md](voice-input.md). Decided on
  2026-10-21; needs a design first.

## Open questions

1. **Database encryption**: settled in 0.4.18. TDLib's database and the app's own are encrypted
   with random keys kept in the Keychain (The app's database, above).
2. **Where downloaded files go**: settled on 2026-10-20: `~/Documents/FinchGram/media_cache`, where
   the user can see the folder and clear it (what is cleared is fetched again when looked at); no
   size limit, by the maintainer's choice. What is fetched when, and what is to come (fetching
   ahead, playing while downloading), is in [media-files.md](media-files.md).
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
   with Noto Sans SC, JP, KR and Arabic for the UI's Chinese, Japanese, Korean and Arabic (the UI
   language's own first among the fallbacks, `fonts.rs`); emoji come from the system's colour
   emoji font, which `fonts.rs` puts right after the body font so that the CJK fonts' black-and-
   white emoji do not win (a font that has its own glyph for a heart or an arrow still draws it, as
   Source Sans 3 does ❤); other scripts in messages come from the system's fonts. Skia would need
   building from source: its
   Rust bindings download a prebuilt library by default, which conventions.md does not allow.
   Moving to Skia, built by our own CI, was planned on 2026-10-11 and paused the next day: the plan,
   why, and what it means for tiny-skia and resvg, is in [skia.md](skia.md).
