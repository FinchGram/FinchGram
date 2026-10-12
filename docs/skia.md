# Moving to Skia: the window, our own pictures, and what stays of tiny-skia and resvg

[中文](skia.zh-Hans.md)

Status: paused (the maintainer, 2026-10-12). Written on 2026-10-11 as the first thing to do; the
next day, weighing what it would bring, the maintainer paused it. The window stays on FemtoVG: step
2 changes nothing for what the app draws itself (that is drawn into pictures Slint shows, whatever
its renderer) and carries the mpv risk below. Steps 1 and 3 wait for the first feature that needs
Skia, such as animated stickers (Skottie) or editing pictures; until then the screenshot tool's
resvg stays, or goes in pure Rust (parley, skrifa and tiny-skia, all in the build through Slint).
PDF, when it comes, goes through macOS's PDFKit: Skia writes PDF, it neither reads nor changes it.
Nothing below is built. The facts are those of the versions in Cargo.lock on 2026-10-11: Slint
1.18.1, skia-safe and skia-bindings 0.153.3, resvg 0.48.1. Starting needs a new vendor build (a
`skia-*` tag) and a change to the release workflow.

## What draws what now

- The window: Slint's FemtoVG renderer, over OpenGL (Slint's default features). The media viewer's
  video is drawn by libmpv into an OpenGL texture of ours inside the same context (`src/player/`).
- The test pictures and the timing test: Slint's software renderer (`src/screenshots.rs`,
  `src/telegram/timing.rs`).
- Our own pictures, by resvg, which draws with tiny-skia:
  - the screenshot tool's picture: the annotations are written as an SVG and drawn over the
    selection cut out of the screen, in the bundled fonts, then saved as PNG by the `image` crate
    (`src/screenshot/export.rs`);
  - the menu bar icon: the mark (`ui/logo/svg/FinchGram-mark-16.svg`), drawn 36 pixels square as a
    template image (`mark_png` in `src/platform/macos.rs`).
- Slint itself: every `@image-url("….svg")` in `ui/` (all the Phosphor icons and the logo) is
  rasterized by resvg at the size it is shown, whichever renderer draws the window
  (`i-slint-core`, `graphics/image/svg.rs`).

`cargo tree -i resvg` shows both: FinchGram, and Slint through `i-slint-common` and `i-slint-core`.
tiny-skia (0.12) comes only with resvg.

## What can go, and what only Slint can change

- **Ours can go.** The two pictures above are drawn with Skia instead, and `resvg` leaves
  Cargo.toml (step 3).
- **Slint's stays, as Slint 1.18 is.** Its Skia renderer turns on `i-slint-core`'s `svg` feature
  itself (the Cargo.toml of `i-slint-renderer-skia` 1.18.1) and draws an SVG image by having resvg
  rasterize it, then handing Skia the pixels (`cached_image.rs`, `ImageInner::Svg`). FemtoVG and
  the software renderer do the same. So after the move resvg and tiny-skia are still compiled into
  the app, by Slint, whatever FinchGram does.
- Taking them out completely needs Slint to draw SVG with Skia's own SVG module (SkSVGDOM,
  skia-safe's `svg` feature), and its Skia renderer to stop asking for `svg`. That is a change in
  Slint, asked for upstream (an issue or a pull request), or a fork of Slint, which would have to be
  carried across every Slint upgrade. Recommended: ask upstream, no fork.
- Turning the icons into Slint `Path` elements (Phosphor's path data, which Skia then draws itself)
  would make every icon Skia's own, but would not take the crates out of the build. It is only worth
  doing together with that change in Slint.

## Steps

### 1. Skia built by our own CI

What is known:

- Slint 1.18.1's Skia renderer uses skia-safe 0.153.3, that is Skia m153: rust-skia commit
  `b7f043e0b1e2a850e702245de811f01df1f6b0ff` (2026-09-04), with rust-skia's fork of Skia at the tag
  `m153-0.101.2` (`61e7ca4e`). The crate on crates.io is that commit's `skia-bindings/`, apart from
  packaging files and the bindings it carries for docs.rs (which is why its `.cargo_vcs_info.json`
  says "dirty").
- Its features in FinchGram's build: `gl`, and `metal` on Apple systems (Slint asks for both), and
  skia-safe's defaults `binary-cache`, `embed-icudtl`, `pdf` and `jpeg`, which Slint leaves on. The
  key the prebuilt archive is named by is then
  `b7f043e0b1e2a850e702-aarch64-apple-darwin-ganesh-gl-jpegd-jpege-metal-pdf`.
- By default skia-bindings' build script downloads that archive from rust-skia's own releases
  (`github.com/rust-skia/skia-binaries`, 4.95 MB), and when that fails it builds Skia from sources
  it downloads. conventions.md allows neither: no third-party prebuilt binaries, no fallback.
- It takes the same archive from wherever `SKIA_BINARIES_URL` points (a template with `{tag}` and
  `{key}`; a `file://` URL with an absolute path works, a relative one does not). Its feature
  `no-compile` turns "the download failed, so build Skia" into a build error.

To do:

- `vendor/skia/build.sh`, pinned at the top: the rust-skia commit, the key it must produce, and the
  oldest macOS (12.0, as for the other vendor builds; rust-skia passes `MACOSX_DEPLOYMENT_TARGET`
  on to the compiler). It checks out rust-skia at that commit with its Skia submodule and builds
  `skia-safe` with the features Slint asks for, so that the set, and the key, are the app's. The
  build runs with `FORCE_SKIA_BUILD=1`, so it never downloads rust-skia's archive, and with
  `BUILD_ARTIFACTSTAGINGDIRECTORY` set, so that it exports `skia-binaries/` (the static libraries,
  the generated bindings, Skia's licence and the key). The script packs that as
  `skia-binaries-<key>.tar.gz`, checks the key and that the libraries are for macOS 12.0, and
  writes BUILD-INFO.txt and SHA256SUMS. Skia's own third-party code comes from its DEPS file,
  pinned by commit (`tools/git-sync-deps`); gn comes from Skia's `bin/fetch-gn` and ninja from
  Homebrew, as meson and ninja do for mpv: tools of the build, not part of the app.
- `.github/workflows/skia.yml`, as mpv.yml: tags `skia-<skia-bindings version>-<n>`, the first
  `skia-0.153.3-1`, published with `--latest=false`.
- `scripts/fetch-skia.sh` downloads the pinned archive into `vendor/skia/bin/`, checks its SHA-256,
  and writes `.cargo/config.toml` (not in git) with
  `[env] SKIA_BINARIES_URL = "file://<repository>/vendor/skia/bin/skia-binaries-{key}.tar.gz"`. The
  path has to be absolute, which a file in git cannot say (Cargo's `relative = true` gives a path,
  not a `file://` URL).
- Cargo.toml: `skia-bindings = { version = "=0.153.3", features = ["no-compile"] }`, with a comment
  saying why. build.rs stops with "run scripts/fetch-skia.sh" when `SKIA_BINARIES_URL` does not
  point to an archive in `vendor/skia/bin/`, as it does for libmpv and the fonts. (Without the
  script, skia-bindings may still fetch rust-skia's archive in the build that build.rs stops;
  nothing of it gets linked.)
- `release.yml` runs `scripts/fetch-skia.sh` with the other three.
- From then on, a Slint upgrade that brings a new skia-safe brings a new vendor build: the
  skia-bindings version in Cargo.lock, `vendor/skia/build.sh` and `scripts/fetch-skia.sh` move
  together.

To measure: how long CI takes (mpv's limit is 120 minutes), and how much bigger the app gets (the
libraries are static, and the linker leaves out what is never called).

### 2. The window on Skia, over OpenGL

- Cargo.toml: `slint` with `default-features = false`, keeping the defaults the app uses (`std`,
  `backend-winit`, `accessibility`, `compat-1-2`, `system-tray`) and adding `renderer-skia-opengl`;
  `renderer-femtovg` goes. `renderer-software` moves to `[dev-dependencies]`, for the picture and
  timing tests; edition 2024's resolver keeps it out of the app. No second renderer stays as a
  fallback.
- OpenGL, not Metal. On macOS Slint's Skia renderer prefers Metal (through wgpu), but libmpv's
  render API draws only into OpenGL or into plain memory, and Slint takes the player's texture
  (`BorrowedOpenGLTexture`) only on its Skia OpenGL surface (`import_opengl_texture` in
  `opengl_surface.rs`). With `renderer-skia-opengl` that surface is tried first, and the rendering
  notifier still hands over `GraphicsAPI::NativeOpenGL`, so `src/player/` keeps its shape.
  `select_backend()` says so as well (`renderer_name("skia-opengl")`, `require_opengl()`), so that a
  build that would land on another surface fails at start instead of showing black video.
- Slint's winit backend asks for the Skia renderer's default features, so wgpu 30 (Slint's Metal
  path) is compiled in though unused: part of the size to measure.
- **The risk is the video.** Skia remembers the OpenGL state it has set and does not set it again;
  FemtoVG sets what it needs at the start of every frame. Slint 1.18.1 flushes Skia before
  BeforeRendering but does not reset Skia's state afterwards (no `DirectContext::reset`). libmpv
  expects OpenGL's defaults when it draws and leaves them behind, except the viewport, the scissor
  box, the blend function and the clear colour (`render_gl.h`). The player's `GlState` puts back
  what FemtoVG needs; Skia's next draw can still count on a program, a buffer, a texture or blending
  that mpv has changed. Two ways out:
  1. mpv in an OpenGL context of its own that shares textures with Slint's (`CGLCreateContext`
     with Slint's context to share with, made current around mpv's render and flushed before
     switching back), so that Skia's state is never touched. render_gl.h itself names shared
     contexts as the way to stay out of each other's state. Try this first.
  2. Slint resetting Skia's context after the notifier (`gr_context.reset(None)`): a one-line
     change upstream; check first whether a newer Slint has it.

  Test with a video playing while the chat list scrolls and the viewer's controls fade, then with
  two videos of one size in a row and the same video twice ([media-files.md](media-files.md)).
- Fonts do not change. Slint 1.18.1's Skia renderer lays text out with the same parley and fontique
  as FemtoVG (`shared-parley`) and makes its Skia typefaces from fontique's font data
  (`font_cache.rs`), so the bundled fonts and `fonts::prefer` carry over.
- Look at the installed app in the three themes, light and dark (the test pictures do not show it):
  text, the avatars' rounded corners, shadows, the transparent title bar, the screenshot
  overlays (their first frame, the crosshair), video, and scrolling a long chat (`refresh_timing`
  for the numbers).

### 3. Our own pictures with Skia, and resvg out of Cargo.toml

- `screenshot/export.rs`: a Skia raster surface over the cut-out selection, with the annotations
  drawn straight onto it instead of through an SVG: rectangles and ellipses as strokes, the arrow
  and the pen from the path strings they already have (`Path::from_svg`), mosaic cells as
  rectangles, halo first and colour over it as now. PNG through Skia's encoder, or through the
  `image` crate as now.
- The text in the picture needs shaping and fallbacks (Chinese, Japanese, Korean, Arabic) in the
  bundled fonts only. Two ways:
  1. lay it out with parley (0.11.1, already in the build through Slint) in a fontique collection
     of only the bundled fonts, and draw the glyph runs with Skia (`draw_glyphs_at`, typefaces made
     from the same font data), as Slint's Skia renderer does;
  2. Skia's `textlayout` (SkParagraph with HarfBuzz and ICU), which changes the vendor build's
     features and makes its archive 17.3 MB instead of 4.95 MB.

  Recommended: 1.
- `mark_png`: the mark is a single path with an even-odd fill; `Path::from_svg` takes its `d` as it
  is, scaled into 36 pixels as now. No SVG module needed.
- Then `resvg` leaves Cargo.toml (and the comment above the macOS dependencies that names it), and
  `cargo tree -i resvg` shows Slint only.
- Tests: the picture test's screenshot-tool pages, and a unit test that reads a few pixels of what
  `compose()` makes (the arrow's colour at its tip, ink where the text is): pixel values, not eyes.

### 4. Docs, and what moves together

- architecture.md (+ zh-Hans): open question 4 answered; the screenshot tool's paragraph (Skia, not
  resvg).
- conventions.md (+ zh-Hans): Skia joins finchgram-tdlib, libmpv and the fonts as a vendor build;
  the tags gain `skia-<version>-<n>`.
- The README in its 11 languages, and CLAUDE.md: `scripts/fetch-skia.sh` with the other fetch
  scripts; `vendor/skia/` and `skia.yml` in the trees.
- CLAUDE.md, "Keeping things in step": the skia-bindings version in Cargo.lock,
  `vendor/skia/build.sh` and `scripts/fetch-skia.sh`.

## Later, not part of this

- Slint's own resvg (above): an issue or a pull request upstream.
- The test pictures with Skia. The software renderer goes on drawing them here, square avatar
  corners and all (`ui/widgets.slint`). Skia could: through `slint::platform::skia_renderer`
  (public, over wgpu and Metal, needs `unstable-wgpu-30`), or through `i-slint-renderer-skia`'s
  `SkiaRenderer::new_with_surface` with a raster surface of our own (Skia on the CPU, Slint's
  internal API). Decide after step 2.
- Moving stickers: a TGS sticker is a gzipped Lottie animation, which Skia's Skottie plays
  (skia-safe's `skottie` feature) instead of rlottie (architecture.md, open question 3). It needs
  `textlayout` too, with the size above. Whether it plays Telegram's stickers right is to be checked
  with real ones.
- Metal: only with a way to bring mpv's frames into Metal (IOSurface, as in the sandboxed player of
  open question 3).
- The `image` crate stays: Slint decodes images with it (`image-decoders`), whatever draws.
