# FinchGram: notes for Claude

## Language

- Reply in the language the person writes in. The maintainer writes Chinese: answer them in
  Simplified Chinese (简体中文) in every message, status updates, summaries, questions and
  explanations included, even when the work itself (code, tool output) is in English.
- Everything that goes into the repository is English: code, comments, commit messages, docs.
  The translations are the exception: `readme/`, `docs/*.zh-Hans.md`, and the UI's `lang/`.
- Release notes are English only: the tag message `scripts/release.sh` writes, and the GitHub
  release page the workflow makes from it.

## What this is

An open-source (GPL-3.0) Telegram desktop client with a media center, in Rust + Slint, macOS on
Apple silicon first. Read `docs/architecture.md` and `docs/conventions.md` before changing how
things fit together.

## Settled decisions

- Telegram goes through TDLib only, running as the separate program `finchgram-tdlib`
  (`vendor/tdlib/`), and the app talks to it only through `src/telegram/`, in TDLib's own JSON.
  Never add the `tdlib-rs` crate or any other binding, and never link TDLib into the app.
- Architecture and conventions follow Coova Studio, the maintainer's other Rust + Slint app: the
  app is a shell; every dependency is built from pinned sources by our own CI; nothing is taken
  from the user's machine, and there is no fallback.
- One public repository holds everything: the source, the vendor builds (`vendor/<name>/`) and the
  releases (`v*` for the app; `tdlib-*` for finchgram-tdlib, published with `--latest=false`).
- The UI follows the maintainer's design (Claude Design: "FinchGram Desktop 视觉方向" for the chat
  windows, "FinchGram Desktop 页面" for logging in, settings and the profile). It has three themes:
  Workbench (the design's 1c, the default), Broadsheet (1a) and Terminal (1b), switched in Settings →
  Appearance without a restart (docs/architecture.md). Every change to the pages goes into all three,
  Workbench first. What the design shows but FinchGram cannot do yet is greyed out; never invent
  pages or visual design beyond the design.
- Video and voice messages play through mpv, as libmpv built from pinned sources in
  `vendor/mpv/` the way finchgram-tdlib is (`mpv-*` releases, `scripts/fetch-mpv.sh`); the app links
  it and draws its frames with OpenGL (`src/player/`). Media follow the design's media-viewer.js.
  Voice messages (`src/telegram/voice.rs`, the design's round "秘密聊天与通话", B1 and B2) play
  through an mpv of their own with no video (`src/player/audio.rs`), and are recorded by holding
  the composer's microphone, as AAC in M4A by AVAudioRecorder, which TDLib takes as voice notes.
- The MCP server (`src/mcp/`, `src/telegram/tools.rs`, docs/mcp.md) is what sets FinchGram apart:
  AI assistants' apps work with the user's Telegram through the running app, over a Unix socket
  with `finchgram mcp` as the stdio bridge, inside the one TDLib session and without an async
  runtime. What an assistant may do is the toolbox's AI access page (the design's "AI 访问" round,
  drawn as a section of Settings, a panel inside the toolbox since 2026-10-24; built in
  every theme: `src/mcp/access.rs`, ui/pages/ai-access.slint), off by default; each app is allowed
  by the user the first time it connects, through the dialog, and remembered by its code signature
  as macOS validates it (Team ID and signing identifier, the same through updates and for every
  copy), or, when it has no signature from a team, by its path and the cdhash of its code (since
  2026-10-29: an update changes the hash, and the program asks again, the dialog saying it changed).
- The Downloads page (`src/downloads/`, docs/downloads.md, the design's "FinchGram Desktop 下载"
  round, built in every theme) is a client over WebDAV, FTP, FTPS or SFTP (the last three since
  0.6.0) whose WebDAV engine follows the maintainer's vget: 8 MB chunks over up to 12
  connections, `.part` + `.part.json` for going on after a stop. It changes nothing on a server
  unless the user turns on "Allow changes" in that server's form (the design's ninth round,
  2026-10-24: delete, move, rename, upload, each on a click, a dialog before a delete; `allow_changes`
  in the database, off for every server until then).
  The servers with their passwords live in the app's database (`src/database.rs`), never in
  settings.toml, at the maintainer's word of 2026-10-20; since 0.6.0 the file is plain SQLite and
  only the password column is sealed, with the key the Keychain holds.
  It needs no Telegram account. Its HTTP client
  (`src/downloads/http.rs`) is hyper with HTTP/2 where a server offers it, on a tokio runtime of
  its own, the one async corner of the app: the maintainer required HTTP/2 on 2026-10-16, knowing
  the "no async runtime" line was a simplification from the first day, not a need. (PikPak itself
  redirects every file request to a CDN that speaks HTTP/1.1 only; what its speed depends on is
  the number of streams and the tail, docs/downloads.md.)
- Voice input (`src/speech/`, docs/voice-input.md, built 2026-10-25 at the maintainer's word,
  "类似微信那样", opened to any vendor on 2026-10-26): a tool of the toolbox's AI category (not
  Telegram's, the maintainer's word), off until the user turns it on in its panel there
  (ui/pages/voice-input.slint), where its settings are; then the composer's sparkle (Terminal:
  /voice; the microphone is voice messages'). Never fixed to one vendor ("应该允许大家自己找vendor"):
  the user adds AI vendors, each a label, an API format (OpenAI-compatible, Anthropic, Gemini,
  Alibaba Cloud Model Studio, Doubao speech), an address (a vendor's, a relay's, a server of their
  own) and a key (`[[voice_input.vendors]]`). Three AI features use them, each with its own switch,
  vendor and model, chosen from dropdown lists (the models as the vendor's own API lists them) and
  NOTHING chosen or assumed for the user (the maintainer's words of 2026-10-26: "你别多管闲事，用一个
  下拉菜单来选，不要做任何假设和默认", "默认不选任何模型，不要自作主张"): voice input (OpenAI realtime or
  upload, Qwen, Doubao's binary protocol; the main language only tells the vendor what is spoken,
  words of other languages mixed in are kept as spoken), tidying its words (Chat Completions,
  Anthropic's Messages, Gemini), and transcribing voice messages; one panel, three sections. The words right come
  first ("以语音转文字的准确度为第一", 2026-10-26): OpenAI's live model at `delay: xhigh`, words to know
  for every format, band-limited resampling, a tidying that puts misheard words right. No model
  inside the app (the installer stays small); the keys are credentials in the database
  (`credentials`, `voice_input.connection.<id>`, one per AI vendor), never in settings.toml; the sessions are
  blocking (tungstenite WebSockets, ureq) on threads of their own, not async. Transcribe under a
  voice message (the design's B1) is Telegram's own transcription (recognizeSpeech); with the third
  feature on (off by default), the user's own vendor instead, Telegram not asked. Built without a design round,
  drawn in each theme's own style; steps 2, 4 and 5 of docs/voice-input.md (a plan, the OS's
  recognizers, a router) are not built.
- Music (`src/music/`, docs/music.md; the maintainer's word of 2026-10-30, "咱们来做一个新的feature吧，就是
  music", the canvas "FinchGram 音乐" of 2026-10-31, built in every theme): a music player, a tool of
  the toolbox of its own, NOT a client named after any server ("music就是单纯的一个tool，而不是依赖于
  其他什么server之类的"). Its sources are servers the user adds inside it that speak the Subsonic API
  (Navidrome, gonic, LMS, Ampache, Nextcloud Music, Koel…), each a library looked at one at a time
  and never merged ("当然是不能合并了"); internet radio was dropped as too niche; Telegram's own
  audio and local folders are not sources in this version; Jellyfin and Emby stay separate (grey)
  tools. The toolbox's Media category was dissolved the same day: Music, Live streams, Jellyfin and
  Emby are each a category of their own ("全部打散"). Songs stream through the app's own HTTP
  client (ureq, the Keychain's roots too) into `~/Documents/FinchGram/music_cache/`, and the music
  mpv (an instance of its own, gapless) reads them as they arrive through the `finchgram://` stream
  callbacks, because the vendored FFmpeg has no network code and keeps none. The sources with their
  passwords are in the database (`music_sources`, layout 8), each password sealed to its source;
  the volume, order, queue column and cache limit are settings.toml's `[music]`.
- Backward compatibility, since 2026-10-24 (the app is public): every change to the database's
  layout, to settings.toml's keys or to the Keychain items comes with its migration. The database
  carries `PRAGMA user_version` (LAYOUT in `src/database.rs`); an older file is first copied as it
  is into `~/Documents/FinchGram/backup/`, then upgraded in place, step by step, automatically and
  without a dialog. Settings keys are never renamed without a serde alias. The database is a plain
  SQLite file anyone can open; only the password column is sealed with the key in the Keychain.
- Telegram API credentials are never in the repository. Every developer uses their own
  (`FINCHGRAM_API_ID`, `FINCHGRAM_API_HASH` at build time); the release workflow uses secrets.

## Naming

- Plain, established names; no invented compounds. (`vendor-build/` was rejected: third-party
  things live in `vendor/<name>/`.)
- No country or region names in language file names: `pt`, not `pt-BR`; Chinese is `zh-Hans` and
  `zh-Hant`.
- Descriptive names, no cryptic abbreviations, Rust's naming idioms.

## Keeping things in step

- The README exists in 11 languages: `README.md` (English) and `readme/<code>.md` (zh-Hans,
  zh-Hant, es, pt, de, fr, ru, ja, ko, ar). A change goes into all 11 with the same structure;
  line 3 is the language switcher.
- Each doc in `docs/` has a `.zh-Hans.md` version; change both.
- The TDLib version is pinned in `vendor/tdlib/build.sh`, `scripts/fetch-tdlib.sh` and
  `TDLIB_VERSION` in `src/telegram/mod.rs`; `src/telegram/api.rs` follows that version's td_api.tl.

## Working

- Do not commit, push or tag unless asked.
- Releases are only ever built by GitHub Actions, and the maintainer tests the installed release, not
  a local build. So FinchGram's own api_id lives only in the repository's Actions secrets
  (`FINCHGRAM_API_ID`, `FINCHGRAM_API_HASH`); never store it on the machine or in a file.
- The release workflow has two jobs since 2026-10-26 (the maintainer's choice, docs/conventions.md):
  compiling on a Blacksmith runner (M4, paid by the minute, 150 free minutes a month), then signing,
  notarizing and publishing on GitHub's own runner, so that the Developer ID, the notary key and
  the update signing key never leave GitHub; rulesets let only the maintainer write branches and
  tags. Checks stay on GitHub's free runner. Artifacts count against the organization's 0.5 GB of
  Actions storage a month, measured by the hour: keep none that a release page already holds.
- Build and check: `scripts/fetch-tdlib.sh`, `scripts/fetch-mpv.sh` and `scripts/fetch-fonts.sh` once, then `cargo build`,
  `cargo test --workspace`, `cargo clippy --workspace --all-targets` (keep it free of warnings;
  `--workspace` takes in the release signing tool, `crates/finchgram-release-sign/`). After changing the UI, run
  `cargo test screenshots -- --ignored` and look at `target/screenshots/`: every page in every theme,
  light and dark, drawn with made-up data (there is no api_id here to log in with).
- `FINCHGRAM_TEST_DC=1` uses Telegram's test servers.
- `scripts/bundle.sh` builds `dist/FinchGram.app` for trying the packaged app; a release only ever
  goes through `scripts/release.sh`, and only when asked.
- Think of contributors: a fresh clone must build and run by following the README.
- Telegram's API terms (core.telegram.org/api/terms) are part of the design: nothing that
  interferes with read receipts, typing or online status; official sponsored messages in channels
  are shown. FinchGram trains nothing and hands nothing of Telegram to any AI on its own; what of
  Telegram reaches an AI is only what the user hands it through the MCP server (docs/mcp.md): off
  by default, within the chats the user chose, never indexed or stored for it, and reading that
  marks nothing as read; and a voice message the user asks to transcribe with their own AI vendor,
  once they turned that on: off by default, one message at their click, the words stored nowhere. The maintainer decided this on 2026-10-12 knowing the terms' clause 1.5,
  and built the second on 2026-10-26 ("语音信息转文字！可以一起做了"). Voice input sends the user's own
  voice to the AI vendor they chose, under their own key, while they dictate; that is not
  Telegram's data.
