# 全面接入 Skia：窗口、我们自己画的图，以及 tiny-skia 和 resvg 能去掉多少

[English](skia.md)

状态：暂停（维护者，2026-10-12）。2026-10-11 写成待办的第一项；第二天权衡它能带来什么之后，维护者决定暂停。
窗口仍用 FemtoVG：第 2 步对 app 自己画的东西没有任何改变（那些都是先画成图片再交给 Slint 显示，与渲染器无关），
却带着下文说的 mpv 风险。第 1 步和第 3 步等第一个真正需要 Skia 的功能一起做，比如动态贴纸（Skottie）或图片编辑；
在那之前截图工具的 resvg 留着，或者用纯 Rust 换掉（parley、skrifa、tiny-skia，Slint 已经全带进了构建）。
将来做 PDF 走 macOS 的 PDFKit：Skia 只会写 PDF，不会读也不会改。下面的东西都还没做。文中的事实以 2026-10-11
Cargo.lock 里的版本为准：Slint 1.18.1、skia-safe 和 skia-bindings 0.153.3、resvg 0.48.1。开工需要新增一个
vendor 构建（推一个 `skia-*` tag），并改发布工作流。

## 现在谁在画什么

- 窗口：Slint 的 FemtoVG 渲染器，走 OpenGL（Slint 的默认功能）。媒体查看器里的视频由 libmpv 画进同一个上下文里
  我们自己的一张 OpenGL 纹理（`src/player/`）。
- 测试图片和计时测试：Slint 的软件渲染器（`src/screenshots.rs`、`src/telegram/timing.rs`）。
- 我们自己画的图，用 resvg（resvg 底下用 tiny-skia 画）：
  - 截图工具出的图：标注先写成一段 SVG，用打包的字体画在从屏幕上抠下来的选区上，再由 `image` crate 存成 PNG
    （`src/screenshot/export.rs`）；
  - 菜单栏图标：把标志（`ui/logo/svg/FinchGram-mark-16.svg`）画成 36 像素见方的模板图（`src/platform/macos.rs`
    的 `mark_png`）。
- Slint 自己：`ui/` 里每个 `@image-url("….svg")`（所有 Phosphor 图标和标志）都由 resvg 按显示尺寸栅格化，
  不管窗口用哪个渲染器（`i-slint-core` 的 `graphics/image/svg.rs`）。

`cargo tree -i resvg` 两处都能看到：FinchGram 自己，以及 Slint 经由 `i-slint-common` 和 `i-slint-core`。
tiny-skia（0.12）只是跟着 resvg 进来的。

## 哪些能去掉，哪些只有 Slint 能改

- **我们自己的能去掉。** 上面两张图改用 Skia 画，`resvg` 从 Cargo.toml 里删掉（第 3 步）。
- **Slint 的去不掉，Slint 1.18 现在就是这样。** 它的 Skia 渲染器自己打开了 `i-slint-core` 的 `svg` 功能
  （`i-slint-renderer-skia` 1.18.1 的 Cargo.toml），画 SVG 图片的办法是让 resvg 栅格化，再把像素交给 Skia
  （`cached_image.rs` 的 `ImageInner::Svg`）。FemtoVG 和软件渲染器也一样。所以换过去以后，resvg 和 tiny-skia
  仍然会被 Slint 编进 app，FinchGram 怎么做都一样。
- 要彻底去掉，得让 Slint 用 Skia 自己的 SVG 模块（SkSVGDOM，skia-safe 的 `svg` 功能）画 SVG，而且它的 Skia
  渲染器不再要求 `svg`。这是 Slint 里的改动：向上游提（issue 或 pull request），或者 fork Slint，而 fork
  每次升级 Slint 都得跟着搬一遍。建议：向上游提，不 fork。
- 把图标改成 Slint 的 `Path` 元素（用 Phosphor 的路径数据，由 Skia 直接画）能让每个图标都由 Skia 画，但这些
  crate 照样留在构建里。只有和上面 Slint 那项改动一起做才值得。

## 步骤

### 1. Skia 由我们自己的 CI 构建

已知的事实：

- Slint 1.18.1 的 Skia 渲染器用的是 skia-safe 0.153.3，也就是 Skia m153：rust-skia 的 commit
  `b7f043e0b1e2a850e702245de811f01df1f6b0ff`（2026-09-04），rust-skia 自己 fork 的 Skia 在 tag
  `m153-0.101.2`（`61e7ca4e`）。crates.io 上的这个 crate 就是这个 commit 的 `skia-bindings/`，只多了打包用的
  文件和给 docs.rs 用的绑定（所以它的 `.cargo_vcs_info.json` 写着 "dirty"）。
- 它在 FinchGram 构建里的功能：`gl`，Apple 系统上还有 `metal`（这两个是 Slint 要的），加上 skia-safe 的默认
  功能 `binary-cache`、`embed-icudtl`、`pdf` 和 `jpeg`（Slint 没有关掉）。预编译包按 key 命名，这一组的 key 是
  `b7f043e0b1e2a850e702-aarch64-apple-darwin-ganesh-gl-jpegd-jpege-metal-pdf`。
- skia-bindings 的构建脚本默认从 rust-skia 自己的 release（`github.com/rust-skia/skia-binaries`，4.95 MB）
  下载这个包，下载失败就自己下载源码来编译 Skia。conventions.md 两样都不允许：不用第三方预编译的二进制，
  也没有退路。
- 它也能从 `SKIA_BINARIES_URL` 指的任何地方取同样的包（一个带 `{tag}` 和 `{key}` 的模板；`file://` 加绝对路径
  可以，相对路径不行）。它的 `no-compile` 功能把"下载失败就编译 Skia"变成构建错误。

要做的：

- `vendor/skia/build.sh`，开头锁定：rust-skia 的 commit、必须产出的 key、最老支持的 macOS（12.0，和其他 vendor
  构建一样；rust-skia 会把 `MACOSX_DEPLOYMENT_TARGET` 传给编译器）。它把 rust-skia 签出到这个 commit（连同 Skia
  子模块），按 Slint 要的功能构建 `skia-safe`，这样功能组合和 key 都和 app 的一致。构建时设
  `FORCE_SKIA_BUILD=1`，保证不会去下载 rust-skia 的包；再设 `BUILD_ARTIFACTSTAGINGDIRECTORY`，让构建导出
  `skia-binaries/`（静态库、生成的绑定、Skia 的许可证和 key）。脚本把它打成 `skia-binaries-<key>.tar.gz`，
  检查 key 对不对、库是不是为 macOS 12.0 编的，再写 BUILD-INFO.txt 和 SHA256SUMS。Skia 自己的第三方代码按它的
  DEPS 文件来，每个都锁定到 commit（`tools/git-sync-deps`）；gn 由 Skia 的 `bin/fetch-gn` 取，ninja 用 Homebrew
  装，和 mpv 用 meson、ninja 一样：都是构建工具，不进 app。
- `.github/workflows/skia.yml`，照 mpv.yml 的样子：tag 是 `skia-<skia-bindings 版本>-<n>`，第一个是
  `skia-0.153.3-1`，用 `--latest=false` 发布。
- `scripts/fetch-skia.sh` 把锁定的包下载到 `vendor/skia/bin/`，检查 SHA-256，再写一个 `.cargo/config.toml`
  （不进 git），内容是
  `[env] SKIA_BINARIES_URL = "file://<仓库路径>/vendor/skia/bin/skia-binaries-{key}.tar.gz"`。路径必须是绝对的，
  进 git 的文件写不出来（Cargo 的 `relative = true` 给出的是路径，不是 `file://` URL）。
- Cargo.toml 加上 `skia-bindings = { version = "=0.153.3", features = ["no-compile"] }`，配注释说明原因。
  `SKIA_BINARIES_URL` 没指向 `vendor/skia/bin/` 里的包时，build.rs 停下来并提示"先运行 scripts/fetch-skia.sh"，
  和 libmpv、字体的做法一样。（没跑这个脚本时，skia-bindings 仍可能在这次被 build.rs 叫停的构建里去取 rust-skia
  的包，但取来的东西不会被链接进去。）
- `release.yml` 和另外三个脚本一起运行 `scripts/fetch-skia.sh`。
- 从此以后，升级 Slint 如果带来新的 skia-safe，就要重做一次 vendor 构建：Cargo.lock 里 skia-bindings 的版本、
  `vendor/skia/build.sh` 和 `scripts/fetch-skia.sh` 要一起改。

要量的：CI 构建要多久（mpv 的上限是 120 分钟），app 会大多少（这些库是静态的，链接器会丢掉没被调用的部分）。

### 2. 窗口换成 Skia，走 OpenGL

- Cargo.toml：`slint` 设 `default-features = false`，留下 app 用到的默认功能（`std`、`backend-winit`、
  `accessibility`、`compat-1-2`、`system-tray`），加上 `renderer-skia-opengl`，去掉 `renderer-femtovg`。
  `renderer-software` 挪到 `[dev-dependencies]`，只给测试图片和计时测试用；edition 2024 的依赖解析不会把它带进
  app。不留第二个渲染器当退路。
- 用 OpenGL，不用 Metal。macOS 上 Slint 的 Skia 渲染器默认走 Metal（经由 wgpu），但 libmpv 的 render API
  只能画进 OpenGL 或普通内存，而 Slint 只有 Skia 的 OpenGL surface 才能接收播放器的纹理（`BorrowedOpenGLTexture`，
  `opengl_surface.rs` 里的 `import_opengl_texture`）。开了 `renderer-skia-opengl`，这个 surface 排在第一个试，
  渲染通知器给的仍然是 `GraphicsAPI::NativeOpenGL`，所以 `src/player/` 的结构不用变。`select_backend()` 里也写明
  （`renderer_name("skia-opengl")`、`require_opengl()`），这样万一落到别的 surface 上，启动时就报错，而不是视频一片黑。
- Slint 的 winit 后端会打开 Skia 渲染器的默认功能，所以 wgpu 30（Slint 的 Metal 路径）虽然不用也会编进去：
  这也算在要量的体积里。
- **风险在视频。** Skia 会记住自己设过的 OpenGL 状态，不再重复设置；FemtoVG 每一帧开始时都把需要的状态重新设一遍。
  Slint 1.18.1 在 BeforeRendering 之前会 flush Skia，但之后不重置 Skia 的状态（没有调用 `DirectContext::reset`）。
  libmpv 画的时候要求 OpenGL 处在默认状态，画完也留下默认状态，只有 viewport、scissor 区域、混合函数和清屏颜色
  除外（`render_gl.h`）。播放器的 `GlState` 恢复的是 FemtoVG 需要的那些；Skia 下一次画的时候，仍可能以为某个
  program、buffer、纹理或混合设置还是它设的样子，而 mpv 已经改过了。两条出路：
  1. 让 mpv 在自己的 OpenGL 上下文里画，和 Slint 的上下文共享纹理（`CGLCreateContext` 时把 Slint 的上下文作为共享
     对象，mpv 画的前后切换当前上下文，切回去之前 flush），Skia 的状态就完全不会被碰到。render_gl.h 自己也说，
     共享上下文是互不干扰状态的办法。先试这个。
  2. 让 Slint 在通知器之后重置 Skia 的上下文（`gr_context.reset(None)`）：上游一行的改动；先看更新的 Slint 有没有。

  测试：视频播放的同时滚动聊天列表、让查看器的控件淡入淡出；再连着放两个尺寸相同的视频，以及把同一个视频放两次
  （[media-files.zh-Hans.md](media-files.zh-Hans.md)）。
- 字体不用改。Slint 1.18.1 的 Skia 渲染器和 FemtoVG 用同一套 parley 和 fontique 排版（`shared-parley`），
  并用 fontique 的字体数据创建 Skia 的 typeface（`font_cache.rs`），所以打包的字体和 `fonts::prefer` 照常生效。
- 在安装好的 app 里看三套主题的浅色和深色（测试图片看不出来）：文字、头像的圆角、阴影、
  透明标题栏、截图覆盖层（第一帧、十字光标）、视频，以及滚动很长的聊天（数字看 `refresh_timing`）。

### 3. 我们自己的图改用 Skia 画，resvg 从 Cargo.toml 删掉

- `screenshot/export.rs`：在抠下来的选区上建一个 Skia 的 raster surface，标注直接画上去，不再经过 SVG：
  矩形和椭圆画描边，箭头和画笔用它们现成的路径字符串（`Path::from_svg`），马赛克格子画成矩形，和现在一样先画外圈的
  衬色、再画颜色。PNG 用 Skia 的编码器，或者照旧用 `image` crate。
- 图里的文字需要字形整形（shaping）和回退字体（中文、日文、韩文、阿拉伯文），而且只用打包的字体。两种做法：
  1. 用 parley（0.11.1，已经随 Slint 在构建里）在只含打包字体的 fontique 字体集里排版，再用 Skia 画出字形
     （`draw_glyphs_at`，typeface 用同一份字体数据创建），和 Slint 的 Skia 渲染器的做法一样；
  2. 用 Skia 的 `textlayout`（SkParagraph，带 HarfBuzz 和 ICU），这会改变 vendor 构建的功能组合，包从 4.95 MB
     变成 17.3 MB。

  建议：第 1 种。
- `mark_png`：标志只有一条路径，用 even-odd 填充；`Path::from_svg` 可以直接吃它的 `d`，和现在一样缩放进 36 像素。
  不需要 SVG 模块。
- 然后从 Cargo.toml 里删掉 `resvg`（连同 macOS 依赖上面那段提到它的注释），`cargo tree -i resvg` 就只剩 Slint。
- 测试：测试图片里截图工具的那几页，再加一个单元测试，读 `compose()` 出图里的几个像素（箭头尖上的颜色、文字所在处
  有没有字）：看像素值，不靠眼睛。

### 4. 文档，以及要一起改的地方

- architecture.md（及 zh-Hans）：待定问题 4 有了答案；截图工具那一段（用 Skia，不再用 resvg）。
- conventions.md（及 zh-Hans）：Skia 和 finchgram-tdlib、libmpv、字体一样成为 vendor 构建；tag 多一种
  `skia-<版本>-<n>`。
- 11 种语言的 README 和 CLAUDE.md：`scripts/fetch-skia.sh` 和其他 fetch 脚本放在一起；目录树里加上 `vendor/skia/`
  和 `skia.yml`。
- CLAUDE.md 的"保持同步"：Cargo.lock 里 skia-bindings 的版本、`vendor/skia/build.sh` 和 `scripts/fetch-skia.sh`。

## 以后再说，不在这次里

- Slint 自己的 resvg（见上）：向上游提 issue 或 pull request。
- 测试图片改用 Skia 画。这次仍由软件渲染器画，头像还是方角（`ui/widgets.slint`）。Skia 可以画：经由
  `slint::platform::skia_renderer`（公开 API，走 wgpu 和 Metal，需要 `unstable-wgpu-30`），或者经由
  `i-slint-renderer-skia` 的 `SkiaRenderer::new_with_surface` 配一个我们自己的 raster surface（Skia 在 CPU 上画，
  用的是 Slint 的内部 API）。第 2 步做完再定。
- 动态贴纸：TGS 贴纸就是 gzip 压缩的 Lottie 动画，Skia 的 Skottie 能播（skia-safe 的 `skottie` 功能），可以不用
  rlottie（architecture.zh-Hans.md 待定问题 3）。它也需要 `textlayout`，体积如上。能不能把 Telegram 的贴纸播对，
  要拿真贴纸试。
- Metal：只有在有办法把 mpv 的画面送进 Metal 之后（IOSurface，就是待定问题 3 里沙箱播放器那个办法）。
- `image` crate 留着：不管用什么画，Slint 都用它解码图片（`image-decoders`）。
