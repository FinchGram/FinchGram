# Skia over Metal for the window, and OpenGL kept to mpv

[中文](skia.zh-Hans.md)

Status: built on 2026-11-11, released as 0.8.0 on 2026-11-12, with what it costs in memory for now
("Memory" and "Decided", below). The maintainer's words on the 11th: "Slint 留下，FemtoVG 换成 Skia，OpenGL
的依赖逐项迁走" (Slint stays, FemtoVG gives way to Skia, what depends on OpenGL moves away piece by
piece), and, once it was clear that mpv cannot leave OpenGL: the window and everything FinchGram draws
itself go to Metal, and OpenGL stays only inside mpv, in a context of mpv's own. Skia built from source
by `vendor/skia/build.sh` and the way of a video frame were tried outside the app first (below, "What
was measured"); then the app was changed ("How it is built"). A first plan, of 2026-10-11, had the window on Skia over OpenGL in one
context with mpv; it was paused the next day, because it brought nothing and carried a risk (the two
would have shared OpenGL's state). Over Metal the risk is gone and the window does leave OpenGL.

The facts are those of the versions in Cargo.lock: Slint 1.18.1, skia-safe and skia-bindings 0.153.3,
wgpu 30.0.1, and of mpv 0.41.0 (`vendor/mpv/build.sh`).

## Where OpenGL was, and what becomes of it

| | Where | What becomes of it |
|---|---|---|
| 1 | The window: Slint's FemtoVG renderer drew every page through OpenGL | Slint's Skia renderer, which draws over Metal on macOS |
| 2 | The video frame's hand-over: mpv drew into a texture inside the window's OpenGL context, which Slint showed as an image (`glow`, and the saving and restoring of OpenGL's state around mpv) | mpv draws into an IOSurface; Metal reads the same memory; no state is shared |
| 3 | The pop-out's player, there because every window had an OpenGL context of its own | all windows share one Metal device; the player of its own stays for now, for the window's own properties |
| 4 | mpv's own drawing: libmpv's render API knows OpenGL and plain memory, nothing else | stays, in an OpenGL context of mpv's own with no window |
| 5 | Slint's Skia renderer compiles Skia's OpenGL backend, `glow` and `glutin` whatever is asked of it | stays in the build, never called; only Slint can change that |

So OpenGL does not leave the app; it is confined. FinchGram's own code makes about fifteen OpenGL and
CGL calls, all for item 4: a context, a texture, a framebuffer, a flush.

## What was found

### Slint

- Its Skia renderer has three surfaces on macOS: wgpu (the default, Metal underneath; there is no
  direct Metal surface any more), OpenGL (the feature `renderer-skia-opengl`) and software
  (`i-slint-renderer-skia`, `lib.rs`). Asking for
  `BackendSelector::require_wgpu_30(…)` with `renderer_name("skia-wgpu")` gives the first and nothing
  else: no surface to fall back to.
- All windows draw with one wgpu device: the first window makes it, the later ones find it
  (`SkiaSharedContext`).
- A picture from outside comes in as `slint::Image::try_from(wgpu::Texture)`, behind the feature
  `unstable-wgpu-30`. Slint takes two formats, `Rgba8Unorm` and `Rgba8UnormSrgb`, and wants the
  texture's usage to say `TEXTURE_BINDING | RENDER_ATTACHMENT` (`i-slint-core`, `graphics/wgpu_30.rs`);
  it only ever reads the texture (`wgpu_30_surface/metal.rs`, `import_metal_texture`). An OpenGL
  texture (`BorrowedOpenGLTexture`) is drawn on the OpenGL surface only, so the renderer and the
  hand-over change together.
- The rendering notifier hands over `GraphicsAPI::WGPU30 { instance, device, queue }`. With a notifier
  set, Slint redraws the whole window every time, so a texture whose content changed shows without
  being set again.
- Text is laid out as before (parley and fontique, shared by the renderers); Skia draws the glyphs.

### mpv

- libmpv's render API has two backends, `"opengl"` and `"sw"` (`render.h`). The software one is, in
  the header's words, "extremely simple (but slow)": it converts every frame on the CPU and takes no
  frame from a hardware decoder.
- Upstream: mpv 0.42 added Direct3D 11 to the render API, for Windows; a pull request (#16818) brings
  the newer renderer to it, with OpenGL first. Nothing for Metal or Vulkan on macOS. mpv's own window
  code for macOS cannot be put inside another program's window (0.41 takes no window to draw into).
- So, for mpv's picture: keep its OpenGL and confine it (chosen); or its software renderer, which
  takes OpenGL out and makes video worse (the CPU for every frame, no hardware frames, none of the
  renderer's colour handling); or wait for upstream; or not mpv, which is not on the table.

### Sharing a surface between OpenGL and Metal

Measured on 2026-11-11 with two small programs (macOS 26.6.2, Apple M4 Max; OpenGL on this Mac is
"4.1 Metal", Apple's own OpenGL on top of Metal):

- An OpenGL context with no window (CGL, the 3.2 core profile) draws into a texture that is an
  IOSurface (`CGLTexImageIOSurface2D`, `GL_TEXTURE_RECTANGLE`), in a framebuffer. Metal makes a texture
  over the same surface (`newTextureWithDescriptor:iosurface:plane:`) and reads what OpenGL drew, a
  second frame too, after OpenGL's `glFlush`.
- OpenGL takes the surface as `GL_BGRA` with `GL_UNSIGNED_INT_8_8_8_8_REV` only; `GL_RGBA` is refused
  (CGL error 10008). The bytes in memory are blue, green, red, alpha.
- Metal makes either a BGRA or an RGBA texture over it. The RGBA one, which Slint needs, shows red and
  blue swapped, unless its descriptor says so (`swizzle`: red from blue, blue from red): then the
  colours are right and nothing is copied.
- A texture with a swizzle cannot be a render target (Metal's validation: "Texture swizzling is not
  compatable with MTLTextureUsageRenderTarget"). It is made to be read only; towards wgpu its usage
  still says `RENDER_ATTACHMENT`, which Slint asks for and never uses. This leans on what Slint 1.18.1
  does. Should a later Slint draw into such a texture, the way out is one copy on the GPU into a
  texture of wgpu's own.

## What was measured

All on 2026-11-11, on the same Mac; nothing here ran on GitHub's runner or on macOS 12.

### Skia from source

`vendor/skia/build.sh`, from nothing (the sources, the build, the checks, the package): 88 seconds, 49
of them building (ninja compiled 1037 objects in 31). The package is 4.9 MB: `libskia.a` (13.5 MB)
with the bindings, and the licences of Skia and of what it carries (expat, libjpeg-turbo, libpng,
zlib, wuffs). Its name is the one skia-bindings looks for; every object says macOS 12.0; skia-bindings,
told never to compile Skia itself (`no-compile`), took it from a `file://` address.

GitHub's runner (macos-latest, three cores; macOS 26.7.2, Xcode 26.6) did the same in 6 minutes 21
seconds on 2026-11-12. Its package, 4.8 MB, is the release `skia-0.153.3-1`, which the app is built
with. Its bindings differ from this machine's in 137 lines, all of them libc++'s own internal types as
another Xcode names them; the app's tests and the video check pass against it.

A Slint program of a few lines, built with the app's release profile: 10.1 MB with FemtoVG, 16.4 MB
with Skia over Metal, 6.2 MB more.

### A video frame, from mpv to what Slint draws

A program outside the app (`frame-check`, kept beside the Skia build's working files, not in git) set
mpv up as the app does and drew as the app would: mpv in an OpenGL context of its own, into an
IOSurface, which Slint's Skia renderer draws over Metal. Its test videos are twelve frames of four
quadrants, red, green, blue and white, which move on by one place every frame, around a grey square
that says which frame it is: colours, which way up, and which frame, read from pixel values and not
judged by eye.

With no window (Slint drawing into a texture, read back), mpv stepped a frame at a time:

| Video | Decoder | In mpv's surface | In Slint's picture |
|---|---|---|---|
| FFV1, RGB, 320×180 | FFmpeg | 12 of 12 frames right | 12 of 12, to the exact value |
| H.264, 1280×720 | VideoToolbox (nv12) | 12 of 12 | 12 of 12 |
| HEVC Main 10, 1920×1080 | VideoToolbox (p010) | 12 of 12 | 12 of 12 |

The image was given to Slint once for each surface; every later frame showed without it being set
again. Metal's validation layer was on and said nothing.

In a window made as the app's (the transparent title bar), drawn by Skia over Metal, with mpv's frame
drawn in the rendering notifier: the same three videos, compared in a picture of the window taken from
the screen with squares of red, green, blue and white drawn by the scene itself: the same colours, in
the right places. And playing:

| Video | Frames drawn in 6 s | Frames dropped | mpv's drawing, a frame |
|---|---|---|---|
| H.264, 1920×1080, 60 a second | 336 to 338 | 6 to 8 | about 1.5 ms |
| HEVC Main 10, 3840×2160, 60 a second (drawn 2560×1440) | 337 to 340 | 4 to 7 | about 2.1 ms |

(The six seconds include opening the file; the dropped frames are taken to be the first ones, not
counted one by one.) The flush that hands OpenGL's drawing over takes 0.02 ms.

### The app after the change

- The executable is 157.9 MB (150.4 MB in 0.7.14): 7.5 MB more. `scripts/bundle.sh` stopped at
  160 MB; its limit is 185 MB now.
- Every page the picture test draws (1837: three themes, light and dark, eleven languages) was drawn
  by the software renderer and by Skia over Metal (`cargo test skia_pictures -- --ignored`), and the
  two compared pixel by pixel. In the most different page 1.96% of the pixels are far apart (more
  than 48 of 255 in a channel), in the median page 0.48%: the edges of text and of rounded corners.
  No page has anything missing or moved. Skia's pictures show what the software renderer's never
  did: colour emoji, the avatars' rounded corners, the round video message.
- The video check in the app (`cargo test video_frames -- --ignored`): both videos, 12 of 12 frames
  in their order, and the frame back in a new surface after the window's renderer went.
- `scripts/bundle.sh` on this machine, from start to end: 32 minutes, a bundle of 206 MB with Skia's
  licences in it.
- The program of that bundle was started once, with a home folder of its own and no account (there
  is no api_id on this machine): its window came up on the screen, 96 MB with the login page showing,
  and Quit ended it within a second, with no crash report. Nobody saw that window: the terminal it
  was started from may not take pictures of the screen.

### Memory

One small window (640×120) with a line of text that redraws sixty times a second, then rests; the
process's footprint (the Memory column of Activity Monitor), on a Mac with 128 GB:

| The window is drawn by | At rest | Redrawing, Latin text | With Chinese (PingFang) | With emoji |
|---|---|---|---|---|
| FemtoVG over OpenGL (the app up to 0.7.14) | 54 MB | 279 MB | 281 MB | 282 MB |
| FemtoVG over Metal | 45 to 52 MB | 470 MB | 473 MB | 470 MB |
| Skia over Metal (this change) | 341 MB | 431 MB | 537 MB | 730 MB |
| Skia over Metal, its font cache not copying (a trial) | 77 MB | 430 MB | 457 MB | 466 MB |

- While a window redraws, the GPU's side takes a few hundred MB, whatever draws, and gives them back
  a second after the last frame: about 225 MB over OpenGL, about 400 MB over Metal. (How much may
  follow the Mac's memory.)
- What stays is Slint's doing. Its Skia renderer copies a font's whole file into memory for every
  face and every variation it draws with (`i-slint-renderer-skia` 1.18.1, `font_cache.rs`,
  `Data::new_copy`); FemtoVG and the software renderer share the bytes fontique has mapped. Apple
  Color Emoji is a file of 192 MB; PingFang's is 78 MB, and each further weight of it 12.6 MB; Noto
  Sans SC is 17.8 MB for each weight. A chat with an emoji and Chinese text costs about 300 MB that
  0.7.14 does not need. (The installed 0.7.14 stood at 632 MB after six hours of use.)
- The trial: six changed lines in that file (the bytes not copied, `Data::new_bytes`, over a font
  kept for good: a system font's file is mapped, the app's own fonts are part of the executable)
  bring Skia to 77 MB at rest. It was only a measurement: nothing of Slint's is changed in the app.
- Copied or not, the same is drawn. A copy is the whole file as memory of the process's own, read
  through when the font is first used; it is the simple and safe way, since Skia then owns its bytes.
  Without a copy the bytes are the mapped file's pages, shared with the system and dropped when memory
  is short, but they have to outlive Skia's use of them.
- Slint's Skia renderer keeps a copy of every image's pixels in the same way (not in the table: its
  window shows no image).
- The picture test as a whole (1837 pages, two windows, the fonts of eleven languages) peaked at
  65 MB with the software renderer and at 1427 MB with Skia: 740 MB of it on the GPU's side (Skia
  keeps up to 256 MB of textures for each window, and Slint sets no other limit), most of the rest
  fonts.

### Seen on the way

- mpv says "a frame to draw" for the frame there was, too (after a pause): what it says cannot be
  counted as frames.
- Asking mpv for the video's size while it waits to show a frame takes tens of milliseconds. The app
  takes the size from what mpv reports, and asks only when it does not know.
- macOS lets a window that is covered by others render only now and then; mpv then drops what was not
  drawn in time (sound goes on). A test window has to stay on top.
- mpv's render call waits until the frame's time has come (`MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME`,
  on by default, and the app leaves it so). At 60 frames a second that is nothing; with a video of 10
  frames a second it was 30 to 50 ms for every frame, on the UI thread, against 3 ms with the wait
  off. This is how the app has drawn video since 0.2.0, measured here in the test program and not in
  the app; it is not part of this move and is left as it is: without the wait a frame shows up to
  50 ms early, which wants thought (sound and picture) before it is changed.

## How it is built

### Skia, built by our own CI

- skia-bindings' build script downloads an archive of Skia for its exact version from rust-skia's own
  releases, and builds Skia from downloaded sources when that fails. conventions.md allows neither.
  It takes the same archive from wherever `SKIA_BINARIES_URL` points, and its feature `no-compile`
  turns "the download failed, so build Skia" into an error.
- `vendor/skia/build.sh` builds that archive. Pinned at its top: the commit of rust-skia that
  skia-bindings 0.153.3 was published from (the archive is named by it), the commit of Skia that
  commit pins, the features (those Slint's Skia renderer asks of skia-safe: `gl` and `metal`, beside
  skia-safe's own defaults), the archive's name, and macOS 12.0. It lets skia-bindings' own build
  script do the work (`FORCE_SKIA_BUILD`, so nothing is downloaded from rust-skia;
  `BUILD_ARTIFACTSTAGINGDIRECTORY`, so the archive's contents are written out), through a crate of one
  line whose `Cargo.lock`, kept beside the script, pins the crates that build script uses (bindgen and
  the rest). Skia's third-party code comes at the commits Skia's DEPS file names; gn comes from Skia's
  own `bin/fetch-gn` and ninja from Homebrew: tools of the build, as meson and ninja are for mpv. It
  checks the archive's name, the version, and that every object is for macOS 12.0, and packs the
  archive with the licences and BUILD-INFO.txt.
- `.github/workflows/skia.yml` builds it on a tag `skia-<skia-bindings version>-<n>` and publishes
  it with `--latest=false`, as mpv.yml does.
- `scripts/fetch-skia.sh` downloads the pinned package into `vendor/skia/bin/`, checks its SHA-256,
  and writes `.cargo/config.toml` (not in git) with
  `[env] SKIA_BINARIES_URL = "file://<the repository>/vendor/skia/bin/skia-binaries-{key}.tar.gz"`:
  skia-bindings reads a `file://` address only with an absolute path, which a file in git cannot say.
- Cargo.toml asks for `skia-bindings` with `no-compile`, at exactly the version in Cargo.lock.
  `build.rs` stops with "run scripts/fetch-skia.sh" when the address does not point into
  `vendor/skia/bin/`. (Without the script, skia-bindings may still fetch rust-skia's archive in the
  build that build.rs stops; nothing of it gets linked.)
- A Slint upgrade that brings a new skia-safe brings a new vendor build first: the skia-bindings
  version in Cargo.lock, the pinned block of `vendor/skia/build.sh` with its `Cargo.lock`, and
  `scripts/fetch-skia.sh` move together.

### The window

- Cargo.toml: `slint` without its default features, with the ones the app uses and `renderer-skia`
  and `unstable-wgpu-30`; FemtoVG and `glow` are gone from the app's own dependencies. The software
  renderer stays: the test pictures and the timing test draw with it, and one set of features for
  building and testing keeps the UI's generated Rust compiled once. The app never falls back to it.
- `select_backend()` asks for wgpu on Metal and the renderer `skia-wgpu`. A Mac on which that cannot
  be had gets an error at the start, not another renderer.

### The video frame

- `src/platform/video.rs`, in the platform layer, since all of it is macOS's: an OpenGL context with
  no window, one for each player, with the few OpenGL calls FinchGram makes, declared by hand (mpv
  finds its OpenGL functions among the libraries the program has loaded); and the IOSurface a player
  draws into, as OpenGL's framebuffer and as the texture Slint takes, made anew when the video's size
  changes. Another system's way goes beside it, under the same names.
- A player makes mpv's renderer when it starts, in its own context, so a file no longer waits for the
  window to render first. The window's rendering notifier still says when to draw: before each
  rendering the player enters its context, has mpv draw the newest frame, flushes, and leaves. When
  the window's renderer goes (the window was hidden), the surface goes with it, since its texture
  belongs to the renderer's device; mpv's renderer stays.
- What holds a wgpu texture is let go before the program ends, and the pop-out is closed then: a
  Metal device must not be torn down while the thread's own variables are destroyed.
- `src/player/frames.rs` checks all of it in the app itself (`cargo test video_frames -- --ignored`):
  the app's own player in the app's own window, drawn by Skia over Metal into a texture, which is
  read back. Two small videos of known colours, one decoded by FFmpeg and one by VideoToolbox: the
  colours, which way up, every frame in its order, and the frame coming back in a new surface after
  the window's renderer went.

### Pictures of what Skia draws

- `src/offscreen.rs` is that window which is never on a screen: Slint's Skia renderer over Metal,
  drawing into a texture (`slint::platform::skia_renderer::SkiaWGPU30Renderer`) that is read back. It
  needs a GPU and no display.
- `cargo test skia_pictures -- --ignored` draws with it every page the picture test draws
  (`src/screenshots.rs`: every theme, light and dark, made-up data), into `target/screenshots-skia/`,
  beside the software renderer's in `target/screenshots/`: the text, the rounded corners and the
  shadows as Skia draws them.

### What stays, and for whom

- mpv's OpenGL: until libmpv can draw another way on macOS.
- Skia's OpenGL backend, `glow` and `glutin` in the build: Slint's Skia renderer asks for them
  unconditionally; the program links OpenGL.framework for them and for mpv.
- resvg and tiny-skia: Slint rasterizes every SVG image with resvg, whichever renderer draws
  (`i-slint-core`, `graphics/image/svg.rs`), and its Skia renderer asks for that feature itself.

## Decided: Slint as it is released

The maintainer, 2026-11-12: "你别fork slint，也别修slint，别改slint的任何东西" (no fork of Slint, no
fix to it, nothing of it changed), and "内存问题等到slint发新版自然就解决了" (the memory takes care of
itself when Slint's new version comes). So 0.8.0 is built with Slint 1.18.1 as it is, and takes the
memory above until then.

Slint has changed it already: since 2026-10-26 its main branch hands font files and pixel buffers to
Skia without copying them (commit 35c801a7, "share image and font data with Skia", for issue #14398;
in the changelog of 1.19.0, which was not released on 2026-11-12). That version brings skia-safe
0.156.0, so the upgrade starts with a new vendor build of Skia ("How it is built"), and the wgpu it
names may be a newer one than `wgpu_30`. One part stays: Skia cannot load a face other than the
first from a font collection on macOS (issues.skia.org/issues/310510989), so Slint builds that face
anew, 12.6 MB for each further weight of PingFang.

## Not checked yet

- macOS 12, the oldest the app runs on.
- How the app looks on a screen: Skia's pictures of the pages are drawn into a texture, a pixel to
  a point, and nobody has seen the window itself. The maintainer looks at the installed app: text
  in the three themes, light and dark, at the screen's own scale; the avatars' rounded corners,
  shadows, the transparent title bar, the screenshot tool's overlays (their first frame, the
  crosshair), video, the pop-out, scrolling a long chat.
- Hours of running, a display plugged in or taken away, sleep and waking.
- Whether the app of today, covered by other windows, drops video frames as the test window did.

## Later, not part of this

- **Our own pictures with Skia, and resvg out of Cargo.toml.** The screenshot tool's picture
  (`src/screenshot/export.rs`: the annotations are written as an SVG and drawn by resvg over the
  selection) and the menu bar icon (`mark_png` in `src/platform/macos.rs`) could be drawn with Skia:
  rectangles, ellipses and the paths they already have (`Path::from_svg`), text laid out with parley
  in a collection of only the bundled fonts and drawn as glyph runs, as Slint's Skia renderer does
  (Skia's own `textlayout` would make the archive 17 MB). `resvg` would leave Cargo.toml and stay in
  the build through Slint. Worth doing with the first feature that draws more than this.
- **Slint's own resvg**: taking it out needs Slint to draw SVG with Skia's SVG module, a change in
  Slint (an issue or a pull request upstream; no fork).
- **Moving stickers**: a TGS sticker is a gzipped Lottie animation, which Skia's Skottie plays
  (skia-safe's `skottie` feature, which needs `textlayout`), instead of rlottie (architecture.md, open
  question 3). Whether it plays Telegram's stickers right is to be checked with real ones.
- **mpv off the UI thread**: with a context of its own, mpv's drawing no longer has to happen where
  the window renders; a thread of its own would take the wait above off the UI. It needs two surfaces
  to swap.
- **PDF**: Skia writes PDF, it neither reads nor changes it; looking at one goes through macOS's
  PDFKit.
- The `image` crate stays: Slint decodes images with it, whatever draws.
