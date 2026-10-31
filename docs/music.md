# Music: a player for your own servers

[中文](music.zh-Hans.md)

Status: built on 2026-10-31 from the maintainer's decisions of 2026-10-30 and 2026-10-31 and the
canvas "FinchGram 音乐" (its third draft), the shared page in every theme, Workbench first; not
released yet.

## What it is

A music player, a tool of the toolbox of its own, like Downloads. It is not a client named after
any server: its sources are servers of the user's own that speak the Subsonic API, added inside the
tool. Navidrome, gonic, LMS, Ampache, Nextcloud Music, Koel, Airsonic-Advanced and Supysonic all
answer it, and Bandcamp gives a Subsonic endpoint for the music a user bought. Each source is a
library of its own, looked at one at a time; libraries are never merged (the maintainer's word).

What it is not, in this version: Jellyfin and Emby are tools of their own in the toolbox, still
grey, not sources here. Internet radio was drawn and then dropped as too niche. Telegram's own music
messages and folders on the Mac are not sources yet. Nothing of Telegram's is involved: the page
opens before logging in, like Downloads.

## The page

View → Music (⌥⌘M); the toolbox's Music (⌘J), which the user can pin to Workbench's activity bar,
Broadsheet's rail or Terminal's status bar; `/music` in Terminal. Three columns, in the shape of
the Downloads page:

- **Library and Sources**, on the left. The library's sections: Albums, Artists, Songs, Genres,
  Playlists, Favourites. Under them the sources, each with its name, a Subsonic badge, its host and
  its status, checked when the page opens (Connected, Password rejected, Not reachable, Not a
  Subsonic server); the source looked at is highlighted, a click looks at another, a right click
  edits. Add source is the last row. Its form asks for a name, the address with the port beside it
  (a port typed into the address moves into the field; Navidrome's 4533 stands in the empty field
  as a hint, nothing is assumed), the account, and has Test connection, which pings the server and
  says what it is ("Navidrome 0.64.2 · OpenSubsonic, 7 extensions") or why not.
- **What is looked at**, in the middle. Albums as a grid of covers (a tile with the album's
  initials where the server has no cover), with a filter box, the order (Recently added, By name,
  By artist, Most played, Recently played, Random; the server's `getAlbumList2` orders) and a
  refresh button; a hundred at a time, More albums fetches the next hundred. An album opens with
  its cover, artist, year, genre, length and format, Play, Shuffle, Add to queue and a star, and its
  songs numbered; a click on a song plays it and the rest of the album follows. Artists, Genres and
  Playlists are lists of names: an artist opens their albums, a genre its albums, a playlist its
  songs. Songs lists every song of the library, two hundred at a time; Favourites the starred ones.
  Lists of songs have Play all and Shuffle. A star on a song or an album goes to the server
  (`star`, `unstar`).
- **The queue**, on the right, which hides (the player bar's queue button, or Hide): what came
  from an album or a list under "Up next", what the user added after it under "Added by you", the
  song playing highlighted; a click plays from there, × takes a song out, Clear empties it. Its
  foot says how much the cache holds.

The player bar stands at the foot of the window while a song plays, on every page and in every
theme's chat window: the cover, the title and artist, a star, shuffle, previous, play or pause,
next, repeat (off, all, one), the position with a bar to seek in, a spinner while mpv waits for
bytes, the queue button, the volume, and a button that opens the Music page (or, on the page
itself, stops the music).

## How songs play

The music has an mpv of its own (`src/player/music.rs`), with no video, beside the viewer's and
the voice messages': songs follow each other without a gap, the next one queued behind the current
one in mpv's playlist and opened ahead of time (`gapless-audio`, `prefetch-playlist`).

The vendored FFmpeg has no network code, by the project's decision (docs/architecture.md), so mpv
does not fetch a song itself. `src/music/fetch.rs` does, with the app's own HTTP client (ureq, with
the Keychain's roots as well as the usual ones, for a server behind a certificate of the user's
own): the server's `stream` URL is read with Range requests into a file of the cache, and mpv reads
that file as it arrives through the `finchgram://` stream callbacks (`src/player/stream.rs`, the
same way a video arrives from TDLib). A seek past what has arrived moves the fetch there, and the
fetch comes back for what it skipped. The whole file stays in the cache under its own name (".part"
while on its way), so the next time the song plays from the disk.

The server's own transcoding settings for the client named "FinchGram" decide what comes over
the wire; FinchGram asks for nothing in particular. The server is told when a song starts
(`scrobble` with `submission=false`, its "now playing") and once it was listened to past half of
it or four minutes, whichever comes first (`submission=true`): the play counts, and Last.fm or
ListenBrainz where the server scrobbles.

## Where things are kept

- The sources, with their passwords, in the app's database (`music_sources`, layout 8), each
  password sealed to its source (kind, address, port and account), as a download server's is to
  its server.
- The volume, shuffle, repeat, whether the queue column shows, the source looked at last and the
  cache's limit in settings.toml, `[music]`.
- Songs and covers in `~/Documents/FinchGram/music_cache/<source>/`, the source a hash of its
  address and account, songs by their ids, covers under `covers/`. The cache is trimmed to
  `cache_limit_mb` (2048 by default, 0 for no limit), the oldest songs first, when the page opens
  and when a song ends; the covers are not counted.

## The API

Subsonic REST API 1.16.1 in JSON (`src/music/subsonic.rs`), with OpenSubsonic's extensions read
from `getOpenSubsonicExtensions` where a server has them. Every request carries a salt and the MD5
of the password and the salt, never the password itself (token authentication); a server that
cannot take a token (error 41, LDAP accounts) gets it hex-encoded instead, once asked. The calls
used: `ping`, `getAlbumList2`, `getAlbum`, `getArtists`, `getArtist`, `getGenres`, `getPlaylists`,
`getPlaylist`, `getStarred2`, `search3` (an empty query lists every song, a page at a time, as
OpenSubsonic reads it), `star`, `unstar`, `scrobble`, `getCoverArt`, `stream`.

## Not in this version

Lyrics (the servers' `getLyricsBySongId` where they have it), a search box over the whole library,
playlist editing, ReplayGain, crossfade and an equaliser, the Mac's Now Playing and media keys,
pausing the music while a voice message plays, albums kept for offline on purpose, drag to reorder
the queue, Telegram's audio messages and local folders as sources, Jellyfin and Emby as sources,
internet radio. Broadsheet and Terminal draw the page in Look's colours and shapes, as the other
shared pages are drawn, without a round of their own yet.

## Testing

- `cargo test -- music`: the API client against answers recorded from demo.navidrome.org
  (`src/music/fixtures/`), the fetcher against an in-process HTTP server with and without ranges,
  the queue's order and repeat, the form, the cache's trimming.
- `cargo test live_subsonic -- --ignored` with `FINCHGRAM_SUBSONIC_URL`, `_USER` and `_PASSWORD`
  set: a real server listed, searched and streamed from (demo.navidrome.org with demo/demo
  answers; a Navidrome from Homebrew or gonic, LMS and Ampache in Docker over the same music
  folder make the compatibility matrix).
- `cargo test screenshots -- --ignored`: the page's scenes in every theme (`music`,
  `music-album`, `music-artists`, `music-favourites`, `music-add`, `music-add-tested`,
  `music-error`, `music-none`, `music-collapsed`) and the chat window with the player bar
  (`chats-playing`).
