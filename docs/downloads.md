# Downloads: a WebDAV client in the media center

[中文](downloads.zh-Hans.md)

Status: built on 2026-10-16 from the design's round for it ("FinchGram Desktop 下载"), in every
theme; released in v0.4.0.

## What it is

A page where files come from servers over WebDAV, FTP, FTPS or SFTP: a NAS (Synology, QNAP), a
seedbox, PikPak, Alist or OpenList (which put cloud drives behind WebDAV), Nextcloud, `rclone serve
webdav`, or a folder shared with `vget serve`. The user adds a server, browses it in the app, picks files or whole folders, and
they arrive in the downloads folder over several connections at once, the way the maintainer's
command-line tool vget fetches them. Photos and videos open in the media viewer when they arrive.

FinchGram only reads. Nothing is uploaded, synced, renamed or deleted on the server, and it is not
a sync client: the user picks what comes.

## The page

View → Downloads (⌥⌘L); the toolbox's Downloads (⌘J, docs/architecture.md), pinned to Workbench's
activity bar and Broadsheet's rail by default, the button carrying the number of files on their
way; and "[↓ 2 downloading]" in Terminal's status bar. The page needs no Telegram account: it opens
before logging in.

Three columns:

- **Servers**, on the left: each with its name, a badge with the protocol it speaks, its host and
  its status, checked when the page opens: Connected,
  Password rejected, Not reachable. A right click gives Edit, Remove and Copy address. Add server
  is the last row. Its form asks for the protocol first (WebDAV, FTP, FTPS or SFTP, see below),
  then the name, the address with the port beside it (the protocol's default stands in the empty
  field: 80 or 443 for WebDAV, 21, 990, 22; a port typed in the address goes into the field), the
  account, and for SFTP how to sign in, with a password, the key waiting for another version.
- **The folder**, in the middle: where we are as a path, "Home NAS /Movies/2026", each folder of it
  a step back, then the folder's name, a box
  that filters the listing by name, a refresh button, and the "Downloads" button that shows or hides
  the queue, with the number of files on their way. The listing is a table: a checkbox for each
  file, an icon by kind, the name, when it was modified, its size; folders first, then names in the
  order of the UI language. A folder opens on a click. Chosen files are counted at the foot with
  their size: Clear, Download, and Open when the one chosen file is a photo or video. A right click
  on a file gives Download, Open and Copy link; on a folder, Open, Download folder and Copy link.
- **Downloads**, on the right, 380 px: the queue. Each file has its name, a progress bar, "4.1 GB
  of 10.8 GB · 24.3 MB/s · 5 min left", and an X that stops it. A stopped file says why and how
  much is kept, with Retry; a finished one has Show in Finder. A right click gives Cancel, Retry,
  Show in Finder and Remove from list; once a file is done, Clear finished stands at the head beside
  Hide and takes every finished file off the list at once. The foot says how many connections a file gets and where
  downloads go, with Change, which opens the folder chooser. Hide folds the column; the choice is
  remembered, and the column comes out by itself when a download starts. With nothing in the queue
  the column is not there at all.

Open fetches the photo or video and then shows it in the media viewer, the same one as for
Telegram's media, with the player's speed and volume. Any other file is shown in the Finder when
it arrives. Download folder lists the folder's tree first, eight folders at a time with the total
growing on the page as files are found, then fetches the files into the same tree under the
downloads folder, six at a time (a server that takes half a second per request would otherwise
make a folder of small files crawl), the files sharing the transfer's streams, first requests
included, so that a folder never has more requests out than one file would; a file already there
with the same size is skipped.

## How a file comes

The engine, `src/downloads/transfer.rs`, follows vget:

1. The first request asks for the first chunk, `Range: bytes=0-8388607` (or the first chunk still
   missing of a stopped download). A 206 answer gives the file's size and says the server does
   ranges, and its body is that chunk, streamed in while the other streams start on the rest; a
   200 answer means the server does no ranges, and its body is the whole file. One round trip
   per file, where a probe for two bytes used to cost a second more on PikPak. Every 206, this one
   and the chunks', must say in its Content-Range that it carries the bytes asked for, with a body
   of that length and the size the first answer gave; an answer for other bytes is a failure like
   any other, and none of it is written.
2. The file is cut into chunks of 8 MB, vget's size. A pool of workers, one stream each (12 by
   default, vget's number; `connections` in settings.toml, 1 to 64), takes the next chunk until
   none is left, each written at its offset into `<name>.part`, which has the file's full size
   from the start, by a writer thread. Nothing else is asked of the server: larger chunks, an idle
   stream taking over the tail of a slow one, and a crawling stream replaced were all tried on
   2026-10-16, and every request more was one more that PikPak answered with 503. `<name>.part` and
   `<name>.part.json` are opened without following a symbolic link, and must be regular files: a
   link put under those names in the download folder is refused, not written through.
3. `<name>.part.json` records which chunks are done, the address they came from and the server's
   Last-Modified for the file. Retry, or a later download of the same file, goes on from there:
   "12 MB of 48 MB kept"; a `.part` of another file of the same size, or of this file changed on
   the server since, is not gone on from.
4. A chunk that fails is tried again from the last byte written, up to ten times, after a pause that
   grows from half a second to eight, whatever the failure: a dropped connection, a 404 from a CDN
   node that does not have the file yet (PikPak does that), a 200 where a 206 was due; so is the
   first request. Only the disk stops a file at once: a file is refused before it starts when the
   disk has not its size and a quarter of a gigabyte to spare, and one of unknown size stops when
   the disk is that nearly full. A server that answers 429, 502, 503 or 504 is
   busy, not broken: PikPak does that while it fetches a file from cold storage, and when it is
   pressed. Then every stream holds back for two seconds before its next request, and the file
   waits up to five minutes for the server before it is given up on; such answers are not counted
   as tries, and the page says "Server busy (HTTP 503), trying again…" only once the server has been
   busy for five seconds (a 503 or two in passing is nothing to tell of). A server that answers 200
   to a range gets the file over one connection.
5. Done, `.part` takes the file's name, or "name (2).ext" when that name is taken.

Two files are on their way at a time; the others wait their turn, and one whose destination is in
use by a file on its way (the same name from elsewhere; a folder and a file within it) waits for
that one, so that two never write one `.part`. While any is, the app tells macOS
it is busy (an NSProcessInfo activity): App Nap does not slow it down when the window is in the
background, and the Mac does not go to sleep on its own. The speed shown is over the last ten
seconds; what a CDN gives varies from minute to minute by a factor of two, and the figure follows
it. Which protocol a server speaks is in the log, not on the page.

What the engine does goes line by line into `downloads.log` next to settings.toml: each probe and
chunk with the protocol, how long the server took to answer and how fast the bytes came, every
retry and why, each listing. It starts afresh past 8 MB, and shows addresses without the account
or query that may be in them (a CDN's signed link). When a server is slow, that file says
where the time goes. Cancel keeps the `.part` files for a retry; Remove from list takes a stopped or
finished file off the list, and a stopped one's `.part` files with it (removing is giving up); Clear
finished takes every finished file off the list and deletes nothing.

WebDAV itself is `src/downloads/webdav.rs`: PROPFIND with depth 1 for a folder's listing (and depth 0
to check a server), the 207 Multi-Status answer parsed with roxmltree, Basic authentication, addresses
percent-encoded by segment. An entry whose name is not a name (empty, "." or "..", or with a "/" in
it) is left out of a listing, and the engine refuses it again before writing: the names become paths
under the download folder, and a server does not get to say where a file lands. A listing's body
must come within thirty seconds between its pieces and ten minutes in all. A folder download lists
at most 100,000 files and goes at most 64 folders deep, each folder once.

HTTP is hyper (`src/downloads/http.rs`), HTTP/2 where the server offers it and HTTP/1.1 elsewhere, on
a tokio runtime of the engine's own, with the flow-control windows set like Go's transport (4 MB per
stream, 32 MB per connection; hyper's defaults would cap a stream at a few hundred kilobytes per
second across an ocean). Redirects are followed, the credentials left behind when the origin changes
(the scheme, the host or the port), and a redirect from https to http is refused rather than followed.
TLS is rustls with the platform's trust store: a certificate the Mac trusts, including one you
trusted yourself in Keychain Access, passes.

What PikPak does, measured on 2026-10-16: its WebDAV host (HTTP/2, in Singapore) answers every file
request with a redirect to a CDN node, `dl-z01a-00xx.mypikpak.net`, which speaks HTTP/1.1 only and
gives each connection between 1 and 10 MB/s, a different node each time. So the speed is a matter
of the tail, not of the protocol: with vget's 12 streams and 8 MB chunks the same 1 GB file came
at the same speed as with vget in the same minute (35 to 40 MB/s on average, the tail included).
More streams, larger chunks and a taken tail bought 60 on average, and 503s with it; the maintainer
chose vget's parameters and no 503s.

## FTP, FTPS and SFTP

Since 0.6.0 a server speaks one of four protocols, chosen when it is added; `src/downloads/remote.rs`
puts one face on the three clients, and the page and the queue do not care which:

- **WebDAV** as above: `webdav.rs`, with the engine of `transfer.rs`.
- **FTP and FTPS** (`ftp.rs`, over suppaftp, blocking): the address is a host and, after a /, a
  folder under the one the account lands in ("sb.example.net/films"); the port is 21, or 990 for
  FTPS, unless given. FTPS is TLS from the first byte on 990 and AUTH TLS on any other port,
  trusting what the Mac trusts, as the HTTP client does. A listing is MLSD where the server has
  it, LIST parsed otherwise. A file comes over one stream, since FTP gives a start but never an
  end: no chunks, no several connections; a stopped file goes on from the end of its `.part` with
  REST. Plain FTP sends the password in the clear, and the form says so in red.
- **SFTP** (`sftp.rs`, over russh and russh-sftp: SSH in Rust, on the downloads' tokio runtime):
  the address is a host and a folder as for FTP, the port 22 unless given; the account signs in
  with its password (a key waits for another version). The server's key is trusted the first time
  it is met and kept in the database (`ssh_hosts`, by host and port); a server showing another key
  later is refused and the page says so, as ssh itself does. A file comes in 8 MB chunks over up
  to as many sessions as the connections setting, each its own SSH connection, written at their
  places in the `.part` and marked in the `.part.json` of the WebDAV engine, so a stopped file
  goes on where it was. On a seedbox, SFTP is also the way that does not count against the monthly
  upload quota that WebDAV and HTTP downloads do (2026-10-23, ultra.cc's own docs).

Test connection lists the root whatever the protocol; "ranges" is SFTP's always and FTP's never.
Copy link gives `sftp://me@host:2222/films/film.mkv` or `ftps://…` for those servers.

## Where things are kept

- `settings.toml`: `[downloads]` has `folder` (empty for the default, `~/Documents/FinchGram/webdav`;
  each server's files go into a folder named after it under there), `connections` and
  `queue_shown`. That is all of it: the file is for preferences.
- The servers, with their passwords, are in the app's database, `finchgram.db` next to it: a
  plain SQLite file any tool opens, the `servers` table with the protocol, the address, the port
  and the account of each; only the password column is sealed (AES-256-GCM) under a random key
  kept in the macOS Keychain under "FinchGram" (the account "database"), in the login keychain
  only, where only this app reads it (docs/architecture.md, The app's database): Keychain items
  belong to the app's signing identity, and releases are signed with North Brookville LLC's Apple
  Developer ID (docs/conventions.md), which macOS trusts across updates. The SSH servers' keys are
  in the `ssh_hosts` table, in the clear. On the wire a WebDAV password is Basic authentication:
  base64, so plain over http on the LAN, and encrypted only by TLS over https; plain FTP sends it
  in the clear too, FTPS and SFTP never do.
  Versions 0.4.11 to 0.4.13 gave the item an access list open to any application as a way around
  it; the prompt stayed, so 0.4.14 took that out again. An item those versions stored keeps the
  list it was given; saving the server again in the form stores a fresh one under the usual list,
  this application only.
- Deleting FinchGram does not delete any of this, as with any Mac app: the Application Support
  folder stays, and so do the Keychain items (Keychain Access → search "FinchGram" → delete).

## Not there yet

- A self-signed certificate, the default on a NAS, is refused unless the Mac trusts it: use http on
  the LAN, get a real certificate (most NAS can get one from Let's Encrypt), or open the NAS's
  certificate in Keychain Access and set it to Always Trust. A "trust this certificate" choice in
  the app is not built.
- Basic authentication only: no Digest, no tokens, no proxy settings.
- A file is fetched whole before it opens; nothing streams.
- No sorting of the table beyond folders first and names in order, no sizes of folders.

## How it is built

- `src/downloads/mod.rs`: the page's state on the UI thread, one worker thread ("servers") for
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
