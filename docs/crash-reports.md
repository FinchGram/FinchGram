# Crash reports

What there is to read when FinchGram crashes, and how to read it. The maintainer tests the
installed release (docs/conventions.md, Release first), so a crash is first of all a file on the
Mac it happened on.

## What is kept

- **macOS's report.** Every crash, whatever its kind (a Rust panic that could not unwind, a fault
  in libmpv or finchgram-tdlib), ends in `~/Library/Logs/DiagnosticReports/finchgram-<date>.ips`;
  after a while macOS moves the old ones into `Retired/` there. It is JSON: the version, the time,
  the signal, every thread's frames. The app is stripped, so its own frames are offsets into the
  binary, not names.
- **panic.log.** Since v0.5.1 a Rust panic in the app writes its message and place, with the time,
  the version and the thread, to `panic.log` next to settings.toml
  (`~/Library/Application Support/FinchGram/`; `src/main.rs`, `log_panics`): the default hook's
  stderr has nowhere to go for an app started from the Dock. A panic in a Slint callback ends the
  app (docs/architecture.md, Threads), so the report shows abort() and the panic machinery, and
  panic.log says why.
- **The symbols of every release.** Since v0.5.2 the release keeps
  `finchgram-<version>-macos-arm64.dSYM.zip` next to the app: the names and line tables of exactly
  that binary, the same UUID. `debug = "line-tables-only"` and `split-debuginfo = "packed"` in
  Cargo.toml put them into `finchgram.dSYM` at build time, and the binary stays stripped and as
  small as before. The symbols weigh about 85 MB zipped (the line tables of 44 MB of code, with
  what was inlined where) and add some five minutes to the build. With them a report from any later
  version reads as function names with file and line.

## Reading a report

`scripts/crash-report.py` does it:

    scripts/crash-report.py                    # the newest report of the app on this Mac
    scripts/crash-report.py REPORT.ips         # that one; a report someone sent, too
    scripts/crash-report.py --list             # every report of the app on this Mac

It prints what happened (version, time, signal), the thread that crashed frame by frame with the
app's frames named (atos from the Xcode command line tools; c++filt turns Rust's mangled names into
paths), and the panic.log lines from the two minutes before. The symbols come from the release of
that version (`gh release download`, kept in `~/Library/Caches/FinchGram/symbols/`), or from
`target/release/finchgram.dSYM` when a local build crashed; a dSYM of another build is passed over,
since its names would be wrong (`--symbols DSYM` forces one, with a warning).

Releases before v0.5.2 kept no symbols. A report from one of them needs a rebuild with the rustc
the release run used (its log says which), `strip = false` and no debug info; CI's Xcode compiles
the C parts, so the code sits at slightly different offsets region by region, and the frames have
to be aligned by trying shifts until they name functions. That is how v0.5.0's crash on the first
keystroke in the search was read, once.

## Not now

- The app sends nothing anywhere of its own: the only servers it talks to are Telegram's, GitHub's
  for the updates, and the WebDAV servers the user entered. A user who meets a crash sends the
  .ips file and panic.log by hand; a step that sends a report would need a design first.
- finchgram-tdlib's and libmpv's frames stay offsets: their builds (`vendor/tdlib/`, `vendor/mpv/`)
  keep no symbols. Their workflows can keep a dSYM the same way when a crash in them calls for it.
