# Downloads: a WebDAV client in the media center

[中文](downloads.zh-Hans.md)

Status: built on 2026-10-16 from the design's round for it ("FinchGram Desktop 下载"), in every
theme; released in v0.4.0.

## What it is

A page where files come from WebDAV servers: a NAS (Synology, QNAP), a seedbox, PikPak, Alist or
OpenList (which put cloud drives behind WebDAV), Nextcloud, `rclone serve webdav`, or a folder shared
with `vget serve`. The user adds a server, browses it in the app, picks files or whole folders, and
they arrive in the downloads folder over several connections at once, the way the maintainer's
command-line tool vget fetches them. Photos and videos open in the media viewer when they arrive.

FinchGram only reads. Nothing is uploaded, synced, renamed or deleted on the server, and it is not
a sync client: the user picks what comes.

## The page

View → Downloads (⌥⌘L), the download icon above the gear in Workbench's activity bar and
Broadsheet's rail (with a dot while a file is on its way), and "[↓ 2]" in Terminal's foot, the
number being the files on their way. The page needs no Telegram account: it opens before logging in.

Three columns:

- **WebDAV servers**, on the left: each with its name, its host and its status, checked when the
  page opens: Connected, Password rejected, Not reachable. A right click gives Edit, Remove and Copy
  address. Add server is the last row.
- **The folder**, in the middle: the server's name and the path as crumbs, the folder's name, a box
  that filters the listing by name, a refresh button, and the "Downloads" button that shows or hides
  the queue, with the number of files on their way. The listing is a table: a checkbox for each
  file, an icon by kind, the name, when it was modified, its size; folders first, then names in the
  order of the UI language. A folder opens on a click. Chosen files are counted at the foot with
  their size: Clear, Download, and Open when the one chosen file is a photo or video. A right click
  on a file gives Download, Open and Copy link; on a folder, Open, Download folder and Copy link.
- **Downloads**, on the right, 380 px: the queue. Each file has its name, a progress bar, "4.1 GB
  of 10.8 GB · 24.3 MB/s · 5 min left", and an X that stops it. A stopped file says why and how
  much is kept, with Retry; a finished one has Show in Finder. A right click gives Cancel, Retry,
  Show in Finder and Remove from list. The foot says how many connections a file gets and where
  downloads go, with Change, which opens the folder chooser. Hide folds the column; the choice is
  remembered, and the column comes out by itself when a download starts. With nothing in the queue
  the column is not there at all.

Open fetches the photo or video and then shows it in the media viewer, the same one as for
Telegram's media, with the player's speed and volume. Any other file is shown in the Finder when
it arrives. Download folder lists the folder's tree first and then fetches every file into the same
tree under the downloads folder; a file already there with the same size is skipped.

## How a file comes

The engine, `src/downloads/transfer.rs`, follows vget:

1. A probe asks for the first two bytes with `Range: bytes=0-1`. A 206 answer gives the file's size
   and says the server does ranges; a 200 answer means it does not.
2. The file is cut into chunks of 8 MB. A pool of workers, one connection each (12 by default,
   `connections` in settings.toml, 1 to 32), takes the next chunk until none is left, each written at
   its offset into `<name>.part`, which has the file's full size from the start.
3. `<name>.part.json` records which chunks are done. Retry, or a later download of the same file,
   goes on from there: "12 MB of 48 MB kept".
4. A chunk that fails is tried again from the last byte written, up to ten times, after a pause that
   grows from half a second to eight; a 401, 403 or 404 stops at once. A server that answers 200 to a
   range gets the file over one connection.
5. Done, `.part` takes the file's name, or "name (2).ext" when that name is taken.

Two files are on their way at a time; the others wait their turn. The speed shown is over the last
three seconds. Cancel keeps the `.part` files for a retry; Remove from list takes a stopped or
finished file off the list and deletes nothing.

WebDAV itself is `src/downloads/webdav.rs`: PROPFIND with depth 1 for a folder's listing (and depth 0
to check a server), the 207 Multi-Status answer parsed with roxmltree, Basic authentication, addresses
percent-encoded by segment. HTTP is ureq with rustls.

## Where things are kept

- `settings.toml`: `[downloads]` has `folder` (empty for the default, `~/Downloads/FinchGram`),
  `connections` and `queue_shown`; `[[webdav]]` has each server's `name`, `url` and `username`.
- The password goes into the macOS Keychain, not into a file: a generic password with the service
  "FinchGram WebDAV" and the account "username@url", in the login keychain only (not iCloud
  Keychain). It is stored when the form is saved, read the first time the server is used in a
  session, and deleted when the server is removed, or when its address or user name changes. On the
  wire it is Basic authentication: base64, so plain over http on the LAN, and encrypted only by TLS
  over https.
- Keychain items belong to the app's signing identity. Releases are signed with the project's own
  certificate, which stays the same across updates, so the installed app reads them without a
  prompt; a local ad-hoc build is a different identity and macOS may ask once.
- Deleting FinchGram does not delete any of this, as with any Mac app: the Application Support
  folder stays, and so do the Keychain items (Keychain Access → search "FinchGram" → delete).

## Not there yet

- A self-signed certificate, the default on a NAS, is refused: rustls trusts the Mozilla roots only.
  Use http on the LAN, or a real certificate (most NAS can get one from Let's Encrypt). A "trust this
  certificate" choice is not built.
- Basic authentication only: no Digest, no tokens, no proxy settings.
- A file is fetched whole before it opens; nothing streams.
- No sorting of the table beyond folders first and names in order, no sizes of folders.
- The `.part` files of a stopped download that was removed from the list stay in the downloads
  folder until the same file is downloaded again.

## How it is built

- `src/downloads/mod.rs`: the page's state on the UI thread, one worker thread ("webdav") for
  listings, checks and the form's test, and one thread per file on its way, with the engine's own
  chunk workers inside it. Answers come back through `slint::invoke_from_event_loop`; there is no
  async runtime, as everywhere in the app.
- `src/platform/`: the folder chooser (NSOpenPanel) and the Keychain (objc2-security).
- `ui/downloads.slint` (the global the page and Rust share), `ui/pages/downloads.slint` (the page,
  its menus and its words, drawn with Look's tokens so one page serves the three themes),
  `src/telegram/viewer.rs` (`view_file`: a downloaded photo or video in the viewer).
- `cargo test downloads`: the listing parser, the addresses, and the engine against a file server
  on the loopback: chunks over several connections, going on from an earlier try, one stream without
  ranges, a failed chunk tried again, a refused password, a cancel.
