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
- Build and check: `scripts/fetch-tdlib.sh` and `scripts/fetch-fonts.sh` once, then `cargo build`,
  `cargo test`, `cargo clippy --all-targets` (keep it free of warnings). After changing the UI, run
  `cargo test screenshots -- --ignored` and look at `target/screenshots/`: every page in every theme,
  light and dark, drawn with made-up data (there is no api_id here to log in with).
- `FINCHGRAM_TEST_DC=1` uses Telegram's test servers.
- `scripts/bundle.sh` builds `dist/FinchGram.app` for trying the packaged app; a release only ever
  goes through `scripts/release.sh`, and only when asked.
- Think of contributors: a fresh clone must build and run by following the README.
- Telegram's API terms (core.telegram.org/api/terms) are part of the design: nothing that
  interferes with read receipts, typing or online status; official sponsored messages in channels
  are shown; nothing obtained from Telegram trains or feeds AI.
