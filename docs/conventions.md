# Development conventions

[中文](conventions.zh-Hans.md)

Coova Studio's conventions, with one difference: FinchGram is open source, so its source, its vendor
builds and its releases all live in this one repository.

## 1. Every third-party dependency ships with the app, built from pinned sources

### Rules

- The app talks to Telegram only through the finchgram-tdlib it ships with. It never looks in the
  PATH, in Homebrew, or anywhere else on the user's machine. A missing copy is a packaging error:
  the UI reports it, and there is no fallback of any kind.
- The app a user receives already contains every dependency. Nothing has to be installed or
  downloaded.
- finchgram-tdlib is built from source by this repository's vendor build,
  `vendor/tdlib/build.sh`. The TDLib commit, the OpenSSL version and its SHA-256, and the
  oldest macOS are pinned at the top of that script. All third-party libraries are linked
  statically: the program depends only on what ships with macOS, and the script checks that
  (`otool -L`), checks the version, and runs a smoke test.
- CI builds it on a clean runner and publishes it as a release of this repository, from a tag such
  as `tdlib-1.8.67-1` (`.github/workflows/tdlib.yml`). No third-party prebuilt binaries are
  used.
- The binaries are not in git. Every developer runs `scripts/fetch-tdlib.sh` once, which downloads
  the pinned release into `vendor/tdlib/bin/` (in .gitignore) and checks its SHA-256; a mismatch
  fails.
- Upgrading means changing the pinned block of `vendor/tdlib/build.sh` and pushing a new
  tdlib-* tag, then changing the release tag and SHA-256 in `scripts/fetch-tdlib.sh` and
  `TDLIB_VERSION` in `src/telegram/mod.rs` (and `src/telegram/api.rs`, when td_api.tl changed
  anything we read). Once committed, everyone re-runs the script and lands on the same version;
  rolling back works the same way.
- `build.rs` copies `vendor/tdlib/bin/finchgram-tdlib` next to the executable (`target/debug/` or
  `target/release/`), so `cargo run` and the packaged app use exactly the same program. When it is
  missing, the build fails and says to run the script first.
- To try a change to the vendor build, build it locally: `vendor/tdlib/build.sh`, then
  `vendor/tdlib/build.sh install`. A release never contains a program built on a developer's
  machine.

### Why

- **A controlled version**: every developer, every build and every user runs the same TDLib, so
  behaviour is consistent and problems are reproducible. The types in `api.rs` match exactly that
  version.
- **Upgrade any time**: changing one block moves to a new version, and rolling back is as easy.
- **Easy collaboration**: clone, run the script once, done. Nobody compiles TDLib unless they
  change it.
- **Nothing for users to worry about**: what is on their machine does not matter.

### Every future external dependency follows the same pattern

Another program, a library, a font or a data file: it gets a folder `vendor/<name>/`, with a script
that builds or downloads it from pinned versions and hashes, and it ships with the app. The script is
in git; what it builds or downloads is not. Never depend on anything already
on the user's machine, and never add a "use the system copy if ours is missing" escape hatch. What
is part of every copy of a supported OS may be used (zlib and the system frameworks on macOS).

The UI fonts follow it: `scripts/fetch-fonts.sh` downloads them from a pinned google/fonts commit
into `vendor/fonts/` and checks their SHA-256, `src/fonts.rs` compiles them into the executable, and
`build.rs` stops early when they are missing. The icons are small SVG files (Phosphor, MIT), kept in
git in `ui/icons/` exactly as they came, with their licence, as Coova Studio keeps its icons.

## 2. One repository

- The source, the vendor builds (`vendor/`) and the releases are all here, in public. Coova
  Studio splits them into three repositories because its source is private; FinchGram has no reason
  to.
- Tags: `v<version>` for the app, `tdlib-<version>-<n>` for finchgram-tdlib (`<n>` counts rebuilds
  of the same TDLib version).
- Vendor releases are published with `--latest=false`, so this repository's "Latest release" is
  always the newest app, which installed copies look for to update themselves.
- Binaries live in releases, never in git history: `vendor/*/bin/`, `vendor/*/work/`,
  `vendor/*/dist/`, `dist/` and `target/` are in .gitignore.

## 3. Release first, test the installed app

As in Coova Studio.

- The app is tested the way users get it: install the release and use it. Releasing is cheap and
  can happen many times a day.
- Every release goes through `scripts/release.sh`; nothing is published by hand. The version in
  `Cargo.toml`, the git tag `v<version>` and the GitHub release are one and the same thing.
- Releases are built by GitHub Actions (`.github/workflows/release.yml`) from the tagged commit on a
  clean runner, never on a developer's machine, with the pinned finchgram-tdlib fetched by the same
  script everyone uses. The workflow runs `scripts/bundle.sh`, which is also how to try the
  packaged app locally; a local bundle is never released.
- Every release is signed: CI signs `SHA256SUMS` with an Ed25519 key that only CI has (the
  `RELEASE_SIGNING_KEY` secret, backed up in the release manager's keychain as "FinchGram release
  signing key"), and the app installs no update it cannot verify with the public keys compiled
  into it (`release-signing.pub`). `src/bin/finchgram-release-sign.rs` makes the key pair
  (`keygen`) and is what CI signs and verifies with. To rotate the key: add the new public key as a
  second line of `release-signing.pub`, ship a version, switch the secret, then remove the old line.
- The installed app updates itself (`src/update.rs`): once a day at a random moment
  (`check_for_updates` in settings.toml turns it off) and from Help → Check for Updates…. It
  verifies the signature first, then the zip's SHA-256 against the signed list; the zip name it
  expects comes from the version, so an old signed list cannot be replayed under a newer tag.
- The update path is part of the product and must keep working from every released version to the
  next. Never change the asset names, the `SHA256SUMS` format, the signing key or the repository
  without first shipping a version that understands the new layout.
- Signing is ad-hoc for now, so Gatekeeper stops a downloaded copy on its first launch (System
  Settings → Privacy & Security → "Open Anyway"); updates installed by the app itself carry no
  quarantine flag. A Developer ID signature with notarization removes that step:
  `SIGN_IDENTITY="Developer ID Application: …" scripts/bundle.sh`.

## 4. Telegram API credentials

- FinchGram has its own api_id and api_hash from my.telegram.org. They are never in the
  repository: the release workflow takes them from its secrets (`FINCHGRAM_API_ID`,
  `FINCHGRAM_API_HASH`) and passes them to the compiler; developers set the same two variables to
  their own when building.
- A build without them runs and says on its window that it has no API ID. It never falls back to
  anybody else's.
- In development, Telegram's test servers keep real accounts out of the way: start the app with
  `FINCHGRAM_TEST_DC=1` (it has a database of its own, `tdlib-test`).
