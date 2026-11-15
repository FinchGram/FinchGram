# Building FinchGram from source

[中文](development.zh-Hans.md)

The app is a shell. Telegram itself is done by TDLib, running as a separate program next to the
executable, `finchgram-tdlib`, which this repository builds from pinned sources; the shell talks to
it only through `src/telegram/`, in TDLib's own JSON. Video plays through libmpv, built the same
way. Nothing is taken from the user's machine, and a build downloads nothing but these pinned,
checksummed builds. Everything is public: the source, the vendor builds (`vendor/`) and the
releases. See [architecture.md](architecture.md) and
[conventions.md](conventions.md).

Development needs macOS on Apple silicon for now: finchgram-tdlib is only built for it. Linux and
Windows will follow.

finchgram-tdlib, libmpv, Skia and the UI fonts are not in git: `vendor/tdlib/`, `vendor/mpv/` and
`vendor/skia/` only hold the scripts that build them, and the fonts come from Google Fonts. After
cloning, get them once:

```sh
scripts/fetch-tdlib.sh    # downloads the pinned finchgram-tdlib release into vendor/tdlib/bin/ and verifies the SHA-256
scripts/fetch-mpv.sh      # downloads the pinned libmpv release into vendor/mpv/bin/ and verifies the SHA-256
scripts/fetch-skia.sh     # downloads the pinned Skia release into vendor/skia/bin/, verifies the SHA-256, and tells cargo where it is
scripts/fetch-fonts.sh    # downloads the pinned UI fonts into vendor/fonts/ and verifies their SHA-256
FINCHGRAM_API_ID=… FINCHGRAM_API_HASH=… cargo run
```

- To change finchgram-tdlib itself, build it here (a few minutes; needs Xcode or the Command Line
  Tools, and `brew install cmake ninja gperf`, build tools only):

  ```sh
  vendor/tdlib/build.sh && vendor/tdlib/build.sh install
  ```

- `FINCHGRAM_API_ID` and `FINCHGRAM_API_HASH`: your own, from
  [my.telegram.org](https://my.telegram.org) → API development tools. They are read when compiling
  and never belong in the repository. Without them the app starts and says it has no API ID.
- `FINCHGRAM_TEST_DC=1 cargo run` uses Telegram's test servers, which have their own accounts; the
  app keeps a separate database for them.
- `cargo test screenshots -- --ignored` draws every page in every theme, light and dark, with made-up
  chats, into `target/screenshots/`: a way to look at the UI without an account.

`build.rs` copies `vendor/tdlib/bin/finchgram-tdlib` next to the compiled executable, so `cargo run`
uses exactly the same program as the packaged app; it links libmpv from `vendor/mpv/bin/` and copies it
there too, and the fonts in `vendor/fonts/` are compiled into the executable. Skia, which draws the
window, is linked from the archive in `vendor/skia/bin/`. Without any of them the build fails with a
message saying so. Rust 1.92 or newer.

TDLib's database and downloaded files are in `~/Library/Application Support/FinchGram/tdlib/`, the
settings in `~/Library/Application Support/FinchGram/settings.toml`.

See also [the repository's layout](layout.md), [translating the interface](translations.md), [releases](releases.md) and [the conventions](conventions.md).
