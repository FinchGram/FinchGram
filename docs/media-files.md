# Media files: what is kept, what is fetched when, and what is still to do

[中文](media-files.zh-Hans.md)

Status: the first two sections are how the app works on 2026-10-09 (v0.3.14 and the fix after it);
nothing under "To do" is built. The maintainer asked for videos to be fetched ahead, and noted that
photos and videos seem to be downloaded again each time: they are not, but the app fetches them
late and shows that it waits.

## What is kept

- TDLib downloads every file into a folder of its own next to its database: on macOS
  `~/Library/Application Support/FinchGram/tdlib/`, with `photos/`, `videos/`, `thumbnails/`,
  `animations/`, `profile_photos/` and so on (`files_directory` is left empty in
  setTdlibParameters, `src/telegram/mod.rs`). Its file database is on (`use_file_database`), so
  across starts TDLib knows what it has. A file is downloaded once; asked for again, it is there at
  once.
- Nothing is deleted. The app sets none of TDLib's storage options, and TDLib's own cleaner
  (`use_storage_optimizer`) is off by default. The maintainer's Mac after ten days of use held
  393 MB: 2028 photos (171 MB), 62 videos (159 MB), 662 thumbnails (8.4 MB).
- In memory (`store.rs`), decoded pictures stay for the rows and the viewer: 200 pictures (no
  larger than 2048 on the longer side, `images.rs`), 1000 previews and 2048 avatars, the oldest
  dropped first.

## How a file is asked for, and what is fetched when

`files.rs` has one way in: `download(file, priority, ready)`. A file complete on disk gives its path
at once. Otherwise one `downloadFile` goes to TDLib per file, however many wait for it, and
`updateFile` says when it is complete. What is on screen goes first (priority 16), the lists'
photos after (8), and what the user opened in the viewer before everything (32). When the app's
copy of a file's state is behind (a message loaded before its file finished), the request goes to
TDLib anyway, which answers from disk: one round trip, nothing transferred.

- A photo in a chat shows its minithumbnail (a blurred few hundred bytes carried inside the
  message) at once. When its row comes into view, `load-picture` fetches the size whose longer side
  is at least 640, and the row shows it. The largest size is a file of its own: it is fetched the
  first time the viewer opens the photo, then kept.
- A video or GIF in a chat fetches only its still, nothing of the video.
- The viewer downloads a video whole, then mpv plays it (`viewer.rs`, `start_video`); the spinner
  shows until then. One already on disk starts as soon as mpv has opened it, a fraction of a
  second. A GIF goes the same way and loops.

So nothing is downloaded twice, but every photo opens blurred before sharp, and every video waits
for its whole file the first time: that is what looks like downloading again. (Up to v0.3.14 a
video the size of the one before also stayed on its still while it played, which looked like a
download that never finished; fixed after v0.3.14.)

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
3. **Playing while downloading.** Telegram's apps start a video within a second however large it
   is, because the videos they send carry their index at the front (`supports_streaming`). mpv can
   read through a stream of the app's own (libmpv's `mpv_stream_cb_add_ro`, a `finchgram://` URL
   with the file id): its read callback reads the file TDLib is writing, waiting until the
   downloaded prefix (`updateFile`, `downloaded_prefix_size`) covers what mpv asks for, and a seek
   past it moves TDLib's download offset (`downloadFile` with `offset`). The file still ends up whole
   on disk, as now. Needs the stream callbacks in `src/player/mpv.rs`, a buffering state in the
   viewer, and the seek bar showing what has arrived.
4. **A storage limit and cleanup**, open question 2 of [architecture.md](architecture.md): TDLib's
   cleaner and its four options (`storage_max_files_size`, `storage_max_time_from_last_access`,
   `storage_max_file_count`, `storage_immunity_delay`), `optimizeStorage` and `getStorageStatistics`
   are there; Settings → Data & storage is in the design and grey in the app. Fetching ahead makes
   this more pressing: it should come with 2.
