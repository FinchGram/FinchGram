# FinchGram

**English** · [简体中文](readme/zh-Hans.md) · [繁體中文](readme/zh-Hant.md) · [Español](readme/es.md) · [Português](readme/pt.md) · [Deutsch](readme/de.md) · [Français](readme/fr.md) · [Русский](readme/ru.md) · [日本語](readme/ja.md) · [한국어](readme/ko.md) · [العربية](readme/ar.md)

An open-source Telegram desktop client with a media center, written in Rust + [Slint](https://slint.dev).
macOS first (Apple silicon); Windows and Linux later.

FinchGram uses the Telegram API and is part of the Telegram ecosystem. It is an unofficial client,
not made by Telegram.

Status: early. Signing up and logging in, the chat list and chats with text messages work, in the
design's three themes: Workbench (the default), Broadsheet and Terminal, switched in Settings →
Appearance. Two-step verification is managed in Settings → Privacy & security. Photos and files,
several accounts, keyword filters and scheduled messages come next.

The app is a shell. Telegram itself is done by TDLib, Telegram's own library, running as a separate
program next to the executable: `finchgram-tdlib`, built from pinned sources by this repository (as
ffmpeg is for Coova Studio). The shell talks to it only through `src/telegram/`, in TDLib's own JSON.
See [docs/architecture.md](docs/architecture.md) and [docs/conventions.md](docs/conventions.md).

Everything is here, in public: the source, the vendor builds (`vendor/`) and the releases.

## Development

Development needs macOS on Apple silicon for now: finchgram-tdlib is only built for it. Linux and
Windows will follow.

finchgram-tdlib and the UI fonts are not in git: `vendor/tdlib/` only holds the script that builds the
program, and the fonts come from Google Fonts. After cloning, get them once:

```sh
scripts/fetch-tdlib.sh    # downloads the pinned finchgram-tdlib release into vendor/tdlib/bin/ and verifies the SHA-256
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
uses exactly the same program as the packaged app, and the fonts in `vendor/fonts/` are compiled into
the executable; without either the build fails with a message saying so. Rust 1.92 or newer.

TDLib's database and downloaded files are in `~/Library/Application Support/FinchGram/tdlib/`, the
settings in `~/Library/Application Support/FinchGram/settings.toml`.

## Layout

```
.github/workflows/
  release.yml            # builds the app on every push to main; publishes v* tags as releases
  tdlib.yml              # builds finchgram-tdlib on a clean runner; publishes tdlib-* tags as releases
Cargo.toml
build.rs                 # compiles ui/app.slint, bundles lang/, copies vendor/tdlib/bin/ next to the executable
docs/                    # architecture.md, conventions.md (+ zh-Hans)
lang/                    # translations: lang/<code>/LC_MESSAGES/finchgram.po, compiled into the binary
readme/                  # this README in other languages
release-signing.pub      # the public keys allowed to sign releases; compiled into the app
scripts/
  bundle.sh              # builds dist/FinchGram.app (what the release workflow runs)
  fetch-fonts.sh         # the pinned UI fonts into vendor/fonts/
  fetch-tdlib.sh         # the pinned finchgram-tdlib release into vendor/tdlib/bin/
  release.sh             # starts a release: version, tag, push; GitHub Actions does the rest
src/
  main.rs                # the window, the settings, the language and theme, updates; starts Telegram
  fonts.rs               # the UI fonts, compiled into the executable
  telegram/              # the only code that talks to finchgram-tdlib
    process.rs           #   runs the program: TDLib's JSON over standard input and output
    api.rs               #   the TDLib types FinchGram uses (td_api.tl of the pinned version)
    mod.rs               #   requests and answers, starting again; updates go to the store
    store.rs             #   what TDLib said about chats, users and messages; the pages' models
    login.rs             #   logging in, signing up
    chats.rs             #   the chat list
    conversation.rs      #   the open chat: messages, writing
    account.rs           #   the profile, logging out
    password.rs          #   two-step verification in Settings
  platform/              # what differs from one operating system to another
  update.rs              # the self-updater: GitHub Releases, signature check, swap, relaunch
  settings.rs            # the user's preferences (settings.toml)
  i18n.rs                # UI language: saved choice, else the system's, else English
  screenshots.rs         # every page drawn to a PNG (cargo test screenshots -- --ignored)
  bin/                   # finchgram-release-sign.rs, the Ed25519 signing tool behind releases
ui/
  app.slint              # the main window: the menus, and which page shows
  state.slint            # the app's state, shared by Rust and the pages
  telegram.slint         # what Telegram shows: the account, logging in, chats, messages
  look.slint             # the theme's colours, type and shapes, for the shared pages
  format.slint           # dates, counts and kinds of message in the UI language
  widgets.slint          # small shared parts; chat.slint: the chat windows' shared parts
  pages/                 # the pages the three themes share: logging in, settings, the profile
  workbench/             # the Workbench theme's chat window (the default)
  broadsheet/            # the Broadsheet theme's chat window
  terminal/              # the Terminal theme's chat window
  icons/                 # Phosphor icons (MIT), regular and duotone; icons.slint lists them
  logo/                  # the FinchGram logo (svg, png) and its rules
vendor/fonts/            # not in git: the UI fonts (scripts/fetch-fonts.sh)
vendor/tdlib/            # finchgram-tdlib
  build.sh               #   builds it from pinned sources: TDLib commit, OpenSSL version and SHA-256 at the top
  host/                  #   our small host program (main.cpp) and its CMakeLists.txt
  bin/                   #   not in git: the program the app uses (scripts/fetch-tdlib.sh, or build.sh install)
  work/, dist/           #   not in git: a local build's intermediate files and its package
```

## Translations

Every piece of UI text is written as `@tr("English text")`. A string has one translation wherever it
appears (build.rs turns off Slint's default context); when the same English needs different words
somewhere else, give it a context: `@tr("menu" => "Open")`. Extract with the official tool, then merge
into each language:

```sh
cargo install slint-tr-extractor                                          # once
find ui -name '*.slint' | sort | xargs slint-tr-extractor --no-default-translation-context -o lang/finchgram.pot
for po in lang/*/LC_MESSAGES/finchgram.po; do msgmerge --update "$po" lang/finchgram.pot; done   # needs brew install gettext
```

Then fill in the `msgstr` entries and `cargo build` again. English is the source language;
Simplified Chinese ships now.

## Releases

Releases are built only by GitHub Actions, from a tagged commit on a clean runner
(`.github/workflows/release.yml`), and published in this repository: the zipped app,
`SHA256SUMS`, and its Ed25519 signature. Installed copies update themselves from there and install
nothing they cannot verify. A maintainer starts a release with one command:

```sh
scripts/release.sh patch    # 0.1.0 -> 0.1.1; also minor, major, or an exact version
```

Signing is ad-hoc for now: on the first launch of a downloaded copy, macOS asks once (System
Settings → Privacy & Security → "Open Anyway"). Updates installed by the app itself start without
that step.

## License

GPL-3.0 ([LICENSE](LICENSE)). finchgram-tdlib contains TDLib (Boost Software License 1.0) and
OpenSSL (Apache License 2.0); the UI fonts are under the SIL Open Font License 1.1, the icons under
the MIT license. Their licence texts travel with the app.
