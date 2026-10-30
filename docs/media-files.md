# Media files: what is kept, what is fetched when, and what is still to do

[中文](media-files.zh-Hans.md)

Status: the first two sections are how the app works on 2026-10-29, when playing while downloading
(once the third item under "To do") was built; the rest of "To do" is not. The maintainer asked for
videos to be fetched ahead, and noted that photos and videos seem to be downloaded again each time:
they are not, but the app fetches them late and shows that it waits.

## What is kept

- TDLib downloads every file into a folder of its own under `~/Documents/FinchGram/media_cache/`:
  `photos/`, `videos/`, `thumbnails/`, `animations/`, `profile_photos/` and so on (the
  `files_directory` of setTdlibParameters, `src/telegram/mod.rs`); the database itself is in
  `~/Library/Application Support/FinchGram/tdlib/`. Its file database is on (`use_file_database`), so
  across starts TDLib knows what it has. A file is downloaded once; asked for again, it is there at
  once.
- Nothing is deleted. The app sets none of TDLib's storage options, and TDLib's own cleaner
  (`use_storage_optimizer`) is off by default. The maintainer's Mac after ten days of use held
  393 MB: 2028 photos (171 MB), 62 videos (159 MB), 662 thumbnails (8.4 MB).
- In memory (`store.rs`), decoded pictures stay for the rows and the viewer: 200 pictures (no
  larger than 2048 on the longer side, `images.rs`), 1000 previews and 2048 avatars, the oldest
  dropped first.

## How a file is asked for, and what is fetched when

`files.rs` has two ways in: `download(file, priority, ready)`, and `stream(file, priority, …)` for
the viewer (below). A file complete on disk gives its path at once. Otherwise one `downloadFile`
goes to TDLib per file, however many wait for it (one more when a later one wants it sooner: TDLib
moves it up), and `updateFile` says when it is complete; when a file that was on its way is
complete, a line in the log says how many MB came in how many seconds. What is on screen goes
first (priority 16), the lists' photos after (8), and what the user opened in the viewer before
everything (32). When the app's copy of a file's state is behind (a message loaded before its file
finished), the request goes to TDLib anyway, which answers from disk: one round trip, nothing
transferred.

- A photo in a chat shows its minithumbnail (a blurred few hundred bytes carried inside the
  message) at once. When its row comes into view, `load-picture` fetches the size whose longer side
  is at least 640, and the row shows it. The largest size is a file of its own: it is fetched the
  first time the viewer opens the photo, then kept.
- A video or GIF in a chat fetches only its still, nothing of the video.
- The viewer plays a video as it arrives (`viewer.rs`, `start_video`): mpv reads it through a
  stream of the app's own (`files::stream`, `src/player/stream.rs`: libmpv's
  `mpv_stream_cb_add_ro`, under a `finchgram://` URL) from the file TDLib is writing, waits for
  bytes that have not come (the spinner, while mpv says it is paused for its cache), and has the
  download moved when it seeks past them (`downloadFile` with `offset`; TDLib keeps what it has
  and goes on from there); the seek bar shows what has arrived. A video whose index is at its
  front (`supports_streaming`, as Telegram's apps send them) starts within a second or two; one
  with the index at the end has mpv fetch the end first, then the start. The file ends up whole on
  the disk as before, and one already there plays from it. A GIF goes the same way and loops.

So nothing is downloaded twice, but every photo opens blurred before sharp, and until 2026-10-29
every video waited for its whole file the first time: that is what looked like downloading again.
(Up to v0.3.14 a video the size of the one before also stayed on its still while it played, which
looked like a download that never finished; fixed after v0.3.14.)

## To do

In the order they would come. None changes how files are kept.

1. **The full photo with the tile.** When a photo's row is in view, fetch its largest size too,
   after the tile and at a low priority, so that the viewer opens it sharp. A few hundred kilobytes
   per photo; a few lines in `conversation.rs` (`load_picture`).
2. **Videos and GIFs of the open chat ahead of time**, under a size limit, at a low priority, as
   their rows come into view or they arrive. For the limit, follow the auto-download settings
   Telegram keeps for the account (TDLib's `getAutoDownloadSettingsPresets`, the `high` preset:
   what the user's phone does on Wi‑Fi, with `max_video_file_size` and `preload_large_videos`), so
   that no page of our own is needed until the design has one.
3. **A storage limit and cleanup**, open question 2 of [architecture.md](architecture.md): TDLib's
   cleaner and its four options (`storage_max_files_size`, `storage_max_time_from_last_access`,
   `storage_max_file_count`, `storage_immunity_delay`), `optimizeStorage` and `getStorageStatistics`
   are there; Settings → Data & storage is in the design and grey in the app. Fetching ahead makes
   this more pressing: it should come with 2.
