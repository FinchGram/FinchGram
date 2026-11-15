# The repository's layout

[中文](layout.zh-Hans.md)

Where things are. [architecture.md](architecture.md) says how they fit together.

```
.github/workflows/
  mpv.yml                # builds libmpv on a clean runner; publishes mpv-* tags as releases
  check.yml              # clippy and the tests on every push to main
  release.yml            # builds and publishes the app for the Release v* commits
  skia.yml               # builds Skia on a clean runner; publishes skia-* tags as releases
  tdlib.yml              # builds finchgram-tdlib on a clean runner; publishes tdlib-* tags as releases
Cargo.toml
build.rs                 # copies vendor/tdlib/bin/ next to the executable
crates/
  finchgram-release-sign/ # the Ed25519 signing tool behind releases, a package of its own
  finchgram-ui/          # the UI: compiles ui/app.slint with lang/ bundled in, a package of its own
docs/                    # the documents, one small file a subject, each in English and Simplified Chinese
  screenshots/           #   the pictures the READMEs show, from the picture test (scripts/readme-pictures.sh)
lang/                    # translations: lang/<code>/LC_MESSAGES/finchgram.po, compiled into the binary
readme/                  # the README in other languages
release-signing.pub      # the public keys allowed to sign releases; compiled into the app
scripts/
  bundle.sh              # builds dist/FinchGram.app (what the release workflow runs)
  fetch-fonts.sh         # the pinned UI fonts into vendor/fonts/
  fetch-mpv.sh           # the pinned libmpv release into vendor/mpv/bin/
  fetch-skia.sh          # the pinned Skia release into vendor/skia/bin/, and where it is for cargo
  fetch-tdlib.sh         # the pinned finchgram-tdlib release into vendor/tdlib/bin/
  readme-pictures.sh     # copies the READMEs' pictures from target/screenshots/ into docs/screenshots/
  release.sh             # starts a release: version, tag, push; GitHub Actions does the rest
  sign.sh                # signs the app: ad hoc, or with the Developer ID in the release workflow
src/
  main.rs                # the window, the settings, the language and theme, updates; starts Telegram
  fonts.rs               # the UI fonts, compiled into the executable
  images.rs              # pictures in messages, decoded off the UI thread
  command_line.rs        # Terminal's command line: which of its commands start with what is typed
  telegram/              # the only code that talks to finchgram-tdlib
    process.rs           #   runs the program: TDLib's JSON over standard input and output
    api.rs               #   the TDLib types FinchGram uses (td_api.tl of the pinned version)
    mod.rs               #   requests and answers, starting again; updates go to the store
    store.rs             #   what TDLib said about chats, users and messages; the pages' models
    login.rs             #   logging in, signing up
    chats.rs             #   the chat list
    conversation.rs      #   the open chat: messages, writing
    actions.rs           #   what can be done with a message: its menu, replies, forwarding, …
    account.rs           #   the profile, logging out
    password.rs          #   two-step verification in Settings
    files.rs             #   downloading files
    avatars.rs           #   the photos of chats and people, shown instead of their letters
    viewer.rs            #   the media viewer: photos, videos, saving to Downloads
    rich_text.rs         #   formatted text: bold, italic, links, …
    notifications.rs     #   notifications of new messages, the unread count on the Dock icon
    online.rs            #   the account is online while the window is in front and in use
  platform/              # what differs from one operating system to another
  player/                # video through libmpv, which draws into a surface the window shows
  screenshot/            # the screenshot tool: capture, the overlay, annotations, the picture
  downloads/             # the Downloads page: WebDAV, FTP, FTPS and SFTP, with a multi-connection engine
  music/                 # the Music page: the Subsonic API client, songs fetched into the cache, the queue and the player bar
    remote.rs            #   one face on the three clients: list a folder, check a server, fetch a file
    webdav.rs            #   PROPFIND listings, addresses, Basic authentication
    ftp.rs               #   FTP and FTPS over suppaftp: one stream, REST to go on after a stop
    sftp.rs              #   SFTP over russh: chunks over several sessions, the server's key remembered
    transfer.rs          #   a file in chunks over several connections, going on after a stop
    mod.rs               #   the page's state, its threads, the queue, the servers' form
  update.rs              # the self-updater: GitHub Releases, signature check, swap, relaunch
  settings.rs            # the user's preferences (settings.toml)
  i18n.rs                # UI language: saved choice, else the system's, else English
  screenshots.rs         # every page drawn to a PNG (cargo test screenshots -- --ignored)
ui/
  app.slint              # the main window: the menus, and which page shows
  state.slint            # the app's state, shared by Rust and the pages
  telegram.slint         # what Telegram shows: the account, logging in, chats, messages
  downloads.slint        # the Downloads page's state: servers, the folder, the queue
  look.slint             # the theme's colours, type and shapes, for the shared pages
  format.slint           # dates, counts and kinds of message in the UI language
  widgets.slint          # small shared parts; chat.slint: the chat windows' shared parts
  viewer.slint           # the media viewer over the whole window, in each theme's manner
  pages/                 # the pages the three themes share: logging in, settings, the profile, downloads
  workbench/             # the Workbench theme's chat window (the default)
  broadsheet/            # the Broadsheet theme's chat window
  terminal/              # the Terminal theme's chat window
  icons/                 # Phosphor icons (MIT), regular and duotone; icons.slint lists them
  logo/                  # the FinchGram logo (svg, png) and its rules
vendor/fonts/            # not in git: the UI fonts (scripts/fetch-fonts.sh)
vendor/mpv/              # libmpv: mpv and FFmpeg, which play video
  build.sh               #   builds it from pinned sources: versions and SHA-256 at the top
  bin/                   #   not in git: the library (scripts/fetch-mpv.sh, or build.sh install)
vendor/skia/             # Skia, which draws the window
  build.sh               #   builds it from pinned sources: the commits and the archive's name at the top
  Cargo.lock             #   pins the crates that build uses
  bin/                   #   not in git: the archive (scripts/fetch-skia.sh, or build.sh install)
vendor/tdlib/            # finchgram-tdlib
  build.sh               #   builds it from pinned sources: TDLib commit, OpenSSL version and SHA-256 at the top
  host/                  #   our small host program (main.cpp) and its CMakeLists.txt
  bin/                   #   not in git: the program the app uses (scripts/fetch-tdlib.sh, or build.sh install)
  work/, dist/           #   not in git: a local build's intermediate files and its package
```
