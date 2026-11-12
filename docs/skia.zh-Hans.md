# 窗口换成 Skia + Metal，OpenGL 只留给 mpv

[English](skia.md)

状态：2026-11-11 做完，2026-11-12 随 0.8.0 发布，暂时带着它在内存上的代价（见下文"内存"和"已定"）。
维护者 11 日的话："Slint 留下，FemtoVG 换成 Skia，OpenGL 的依赖逐项迁走"；在弄清
mpv 离不开 OpenGL 之后定下的目标是：窗口和 FinchGram 自己画的一切都走 Metal，OpenGL 只留在 mpv 内部，用 mpv
自己的一个上下文。`vendor/skia/build.sh` 从源码构建 Skia、以及视频帧的整条路，都先在 app 之外试过（见下文
"实测结果"），然后才改的 app（见"怎么做的"）。2026-10-11 有过第一版计划：窗口用 Skia 但仍走 OpenGL，和 mpv 共用一个上下文；
第二天就暂停了，因为它没有收益，还带着风险（两边会共用 OpenGL 的状态）。改走 Metal 之后这个风险不存在了，
窗口也真的离开了 OpenGL。

文中的事实以 Cargo.lock 里的版本为准：Slint 1.18.1、skia-safe 和 skia-bindings 0.153.3、wgpu 30.0.1，
以及 mpv 0.41.0（`vendor/mpv/build.sh`）。

## OpenGL 原来用在哪里，各自怎么处理

| | 用在哪里 | 怎么处理 |
|---|---|---|
| 1 | 窗口：Slint 的 FemtoVG 渲染器靠 OpenGL 画每一页 | 换成 Slint 的 Skia 渲染器，它在 macOS 上走 Metal |
| 2 | 视频帧的交接：mpv 画进窗口的 OpenGL 上下文里的一张纹理，Slint 把它当图片显示（`glow`，以及在 mpv 前后保存、恢复 OpenGL 状态） | mpv 画进一块 IOSurface，Metal 读同一块内存，不再共用任何状态 |
| 3 | 弹出窗口有自己的播放器，因为每个窗口有自己的 OpenGL 上下文 | 所有窗口共用一个 Metal 设备；单独的播放器暂时留着，因为那个窗口有自己的属性 |
| 4 | mpv 自己画画面：libmpv 的 render API 只会 OpenGL 和普通内存 | 留着，放在 mpv 自己的、不带窗口的 OpenGL 上下文里 |
| 5 | Slint 的 Skia 渲染器不管要不要都会编译 Skia 的 OpenGL 后端、`glow` 和 `glutin` | 留在构建里，不会被调用；只有 Slint 能改 |

所以 OpenGL 并没有离开 app，而是被关起来了。FinchGram 自己的代码大约调用十五个 OpenGL 和 CGL 函数，全是为了
第 4 项：一个上下文、一张纹理、一个帧缓冲、一次 flush。

## 调研结论

### Slint

- 它的 Skia 渲染器在 macOS 上有三种 surface：wgpu（默认，底下是 Metal；已经没有直接的 Metal surface 了）、
  OpenGL（`renderer-skia-opengl` 功能）和软件（`i-slint-renderer-skia` 的 `lib.rs`）。用
  `BackendSelector::require_wgpu_30(…)` 加 `renderer_name("skia-wgpu")` 得到的就是第一种，没有别的 surface
  可退。
- 所有窗口用同一个 wgpu 设备：第一个窗口创建，后面的窗口找到它（`SkiaSharedContext`）。
- 外来的画面以 `slint::Image::try_from(wgpu::Texture)` 进来，需要 `unstable-wgpu-30` 功能。Slint 只收两种
  格式，`Rgba8Unorm` 和 `Rgba8UnormSrgb`，并要求纹理的用途写着 `TEXTURE_BINDING | RENDER_ATTACHMENT`
  （`i-slint-core` 的 `graphics/wgpu_30.rs`）；它实际上只读这张纹理（`wgpu_30_surface/metal.rs` 的
  `import_metal_texture`）。OpenGL 纹理（`BorrowedOpenGLTexture`）只有 OpenGL surface 才画，所以渲染器和
  交接方式必须一起换。
- 渲染通知器给的是 `GraphicsAPI::WGPU30 { instance, device, queue }`。设了通知器之后 Slint 每次都重画整个
  窗口，所以纹理的内容变了不用重新设置也会显示出来。
- 文字的排版和以前一样（parley 和 fontique，各渲染器共用）；字形由 Skia 画。

### mpv

- libmpv 的 render API 有两个后端，`"opengl"` 和 `"sw"`（`render.h`）。软件那个，按头文件自己的话，
  "extremely simple (but slow)"：每一帧都在 CPU 上转换，而且不收硬件解码器的帧。
- 上游：mpv 0.42 给 render API 加了 Direct3D 11，是 Windows 用的；有一个 pull request（#16818）把新的渲染器
  接进 render API，先做的也是 OpenGL。macOS 上的 Metal 或 Vulkan 都没有。mpv 自己在 macOS 上的窗口代码放不进
  别的程序的窗口里（0.41 不接受外来的窗口）。
- 所以 mpv 的画面只有这几条路：留着它的 OpenGL 并把它关起来（选了这条）；用它的软件渲染器，OpenGL 没了但视频
  变差（每一帧都靠 CPU，不能用硬件帧，渲染器的颜色处理也没有了）；等上游；或者不用 mpv，这一条不考虑。

### OpenGL 和 Metal 共用一块 surface

2026-11-11 用两个小程序实测（macOS 26.6.2，Apple M4 Max；这台机器上的 OpenGL 是 "4.1 Metal"，也就是苹果自己
架在 Metal 上的 OpenGL）：

- 不带窗口的 OpenGL 上下文（CGL，3.2 core profile）画进一张以 IOSurface 为存储的纹理
  （`CGLTexImageIOSurface2D`，`GL_TEXTURE_RECTANGLE`），外面套一个帧缓冲。Metal 在同一块 surface 上创建纹理
  （`newTextureWithDescriptor:iosurface:plane:`），读到的就是 OpenGL 画的内容；OpenGL `glFlush` 之后，
  第二帧也能读到。
- OpenGL 只接受 `GL_BGRA` 加 `GL_UNSIGNED_INT_8_8_8_8_REV`；`GL_RGBA` 被拒绝（CGL 错误 10008）。内存里的字节
  顺序是蓝、绿、红、alpha。
- Metal 在它上面既能建 BGRA 纹理也能建 RGBA 纹理。Slint 要的 RGBA 那种，红蓝是反的，除非在描述里写明
  （`swizzle`：红取蓝、蓝取红）：这样颜色就对了，而且不用拷贝。
- 带 swizzle 的纹理不能做渲染目标（Metal 校验层的原话："Texture swizzling is not compatable with
  MTLTextureUsageRenderTarget"）。所以它按只读创建；对 wgpu 那一层，用途仍然写着 `RENDER_ATTACHMENT`，
  这是 Slint 要求的，但它从不使用。这一点依赖 Slint 1.18.1 的现状。以后的 Slint 如果真要往这种纹理里画，
  退路是在 GPU 上多拷一次，拷进一张 wgpu 自己的纹理。

## 实测结果

都是 2026-11-11 在同一台 Mac 上测的；GitHub 的 runner 和 macOS 12 上都没跑过。

### 从源码构建 Skia

`vendor/skia/build.sh` 从零跑完（取源码、构建、检查、打包）：88 秒，其中构建 49 秒（ninja 31 秒编了 1037 个
目标文件）。包是 4.9 MB：`libskia.a`（13.5 MB）和绑定，加上 Skia 及其内含各库（expat、libjpeg-turbo、
libpng、zlib、wuffs）的许可证。它的名字正是 skia-bindings 要找的；每个目标文件都标着 macOS 12.0；
skia-bindings 在不许自己编译 Skia（`no-compile`）的设置下，从一个 `file://` 地址接受了它。

GitHub 的 runner（macos-latest，三核；macOS 26.7.2，Xcode 26.6）在 2026-11-12 做同样的事用了 6 分 21 秒。
它出的包 4.8 MB，就是 `skia-0.153.3-1` 这个 release，app 用它构建。它的绑定和本机的差 137 行，全是 libc++
自己的内部类型在另一个 Xcode 里的叫法；app 的测试和视频检查用它都通过。

一个只有几行的 Slint 程序，按 app 的发布配置编译：用 FemtoVG 是 10.1 MB，用 Skia + Metal 是 16.4 MB，
多 6.2 MB。

### 一帧视频，从 mpv 到 Slint 画出来

app 之外的一个程序（`frame-check`，和 Skia 构建的中间文件放在一起，不进 git）按 app 的方式设置 mpv，并按 app
将来的方式画：mpv 在自己的 OpenGL 上下文里画进 IOSurface，Slint 的 Skia 渲染器经 Metal 把它画出来。测试视频
是十二帧的四个象限，红、绿、蓝、白，每帧轮换一格，中间一个灰块标明帧号：颜色、上下左右、是第几帧，都从像素值
读出来，不靠眼睛判断。

不开窗口（Slint 画进一张纹理再读回来），mpv 一帧一帧地步进：

| 视频 | 解码 | mpv 写进 surface 的 | Slint 画出来的 |
|---|---|---|---|
| FFV1，RGB，320×180 | FFmpeg | 12 帧全对 | 12 帧全对，数值分毫不差 |
| H.264，1280×720 | VideoToolbox（nv12） | 12 帧全对 | 12 帧全对 |
| HEVC Main 10，1920×1080 | VideoToolbox（p010） | 12 帧全对 | 12 帧全对 |

每块 surface 只把图像交给 Slint 一次，之后的每一帧都不用重新设置就显示出来了。全程开着 Metal 的校验层，
没有任何报错。

在按 app 的方式创建的窗口里（透明标题栏），由 Skia 经 Metal 绘制，mpv 的帧在渲染通知器里画：同样三段视频，
把窗口从屏幕上截下来，和场景自己画的红、绿、蓝、白方块比对：颜色相同，位置正确。播放时：

| 视频 | 6 秒内画出的帧 | 丢掉的帧 | mpv 每帧绘制 |
|---|---|---|---|
| H.264，1920×1080，每秒 60 帧 | 336 到 338 | 6 到 8 | 约 1.5 毫秒 |
| HEVC Main 10，3840×2160，每秒 60 帧（按 2560×1440 画） | 337 到 340 | 4 到 7 | 约 2.1 毫秒 |

（这六秒包含打开文件的时间；丢掉的帧推断是开头那几帧，没有逐帧核对。）把 OpenGL 画的内容交出去的那次 flush
耗时 0.02 毫秒。

### 改完之后的 app

- 可执行文件 157.9 MB（0.7.14 是 150.4 MB），多 7.5 MB。`scripts/bundle.sh` 原来的上限是 160 MB，现在是 185 MB。
- 截图测试画的每一页（1837 页：三套主题、浅色和深色、十一种语言）都用软件渲染器和 Skia + Metal 各画了一遍
  （`cargo test skia_pictures -- --ignored`），再逐像素比较。差别最大的一页有 1.96% 的像素相差较远
  （某个通道差 48/255 以上），中位数是 0.48%：都是文字和圆角的边缘。没有任何一页缺东西或者错位。软件渲染器
  一直画不出来的东西，Skia 的图里都有：彩色 emoji、头像的圆角、圆形的视频消息。
- app 里的视频检查（`cargo test video_frames -- --ignored`）：两段视频都是 12 帧全对、顺序正确，窗口的渲染器
  没了之后那一帧也在新的 surface 里重新出现。
- `scripts/bundle.sh` 在本机从头跑到尾：32 分钟，打出的包 206 MB，里面有 Skia 的许可证。
- 那个包里的程序启动过一次，用的是它自己的主目录，没有账号（本机没有 api_id）：窗口出现在屏幕上，显示登录页时
  占 96 MB，选退出后一秒内结束，没有崩溃报告。没有人看过那个窗口：启动它的终端没有截屏的权限。

### 内存

一个小窗口（640×120），里面一行文字，每秒重绘六十次，然后停下；记录进程的内存占用（活动监视器的"内存"一栏），
机器有 128 GB 内存：

| 窗口由谁画 | 空闲时 | 重绘中，拉丁文 | 加中文（苹方） | 加 emoji |
|---|---|---|---|---|
| FemtoVG + OpenGL（0.7.14 及以前的 app） | 54 MB | 279 MB | 281 MB | 282 MB |
| FemtoVG + Metal | 45 到 52 MB | 470 MB | 473 MB | 470 MB |
| Skia + Metal（这次的改动） | 341 MB | 431 MB | 537 MB | 730 MB |
| Skia + Metal，字体缓存不拷贝（试验） | 77 MB | 430 MB | 457 MB | 466 MB |

- 窗口重绘期间，GPU 一侧会多占几百 MB，不管谁来画；最后一帧之后一秒就还回去：走 OpenGL 约 225 MB，
  走 Metal 约 400 MB。（具体多少可能和机器的内存大小有关。）
- 常驻的那部分是 Slint 造成的。它的 Skia 渲染器每用到一个字体的一个字重或一种变体，就把整个字体文件在内存里
  拷贝一份（`i-slint-renderer-skia` 1.18.1 的 `font_cache.rs`，`Data::new_copy`）；FemtoVG 和软件渲染器
  用的是 fontique 映射进来的那一份。Apple Color Emoji 的文件是 192 MB；苹方是 78 MB，之后每多一个字重
  12.6 MB；Noto Sans SC 每个字重 17.8 MB。一个有 emoji 和中文的聊天，就比 0.7.14 多占大约 300 MB。
  （安装版 0.7.14 用了六个小时之后是 632 MB。）
- 试验：把那个文件改六行（不拷贝，用 `Data::new_bytes`，字体数据永久保留：系统字体的文件是映射进来的，
  app 自己的字体本来就在可执行文件里），Skia 空闲时就是 77 MB。这只是一次测量：app 里没有改 Slint 的任何东西。
- 拷贝与否，画出来的东西一样。拷贝就是整个文件变成进程自己的内存，第一次用到这个字体时要把文件读一遍；
  这是简单而安全的做法，因为 Skia 拿着的是自己的一份。不拷贝，用的就是映射文件的那些页，和系统共享，
  内存紧张时可以丢掉，但这些字节必须活得比 Skia 用它们的时间长。
- Slint 的 Skia 渲染器对每张图片的像素也同样多留一份拷贝（表里没有体现：那个窗口不显示图片）。
- 整轮截图测试（1837 页、两个窗口、十一种语言的字体）的峰值：软件渲染器 65 MB，Skia 1427 MB，其中 740 MB
  在 GPU 一侧（Skia 给每个窗口最多留 256 MB 的纹理，Slint 没有另设上限），其余大部分是字体。

### 过程中看到的

- mpv 说"有一帧要画"的时候，也可能是原来那一帧（比如暂停之后）：不能拿它的通知当帧数来数。
- 在 mpv 等着显示一帧的时候去问它视频的尺寸，要花几十毫秒。app 的做法是用 mpv 主动报告的尺寸，不知道的时候
  才问。
- 窗口被别的窗口完全盖住时，macOS 只偶尔让它重绘；没来得及画的帧 mpv 就丢掉（声音照常）。所以测试窗口得
  置顶。
- mpv 的 render 调用会等到这一帧该显示的时刻才返回（`MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME`，默认打开，
  app 没有改它）。每秒 60 帧时这没什么；每秒 10 帧的视频，每一帧要在界面线程上等 30 到 50 毫秒，关掉等待后
  是 3 毫秒。app 从 0.2.0 起就是这样画视频的；这个数是在测试程序里量的，不是在 app 里量的。它不属于这次迁移，
  先原样留着：关掉等待后，一帧最多会提前 50 毫秒显示，改之前得先想清楚（音画同步）。

## 怎么做的

### Skia，由我们自己的 CI 构建

- skia-bindings 的构建脚本会按自己的确切版本从 rust-skia 的 release 下载一个 Skia 的包，下载失败就下载源码来
  编译。conventions.md 两样都不允许。它也能从 `SKIA_BINARIES_URL` 指的任何地方取同样的包，而它的
  `no-compile` 功能把"下载失败就编译 Skia"变成错误。
- `vendor/skia/build.sh` 构建的就是这个包。开头锁定的有：skia-bindings 0.153.3 发布时所在的 rust-skia commit
  （包就是按它命名的）、那个 commit 锁定的 Skia commit、功能组合（Slint 的 Skia 渲染器向 skia-safe 要的
  `gl` 和 `metal`，加上 skia-safe 自己的默认功能）、包的名字，以及 macOS 12.0。活儿交给 skia-bindings 自己的
  构建脚本去干（设 `FORCE_SKIA_BUILD`，就不会去下载 rust-skia 的包；设 `BUILD_ARTIFACTSTAGINGDIRECTORY`，
  它就把包的内容导出来），中间隔着一个只有一行依赖的 crate，它的 `Cargo.lock` 放在脚本旁边，锁定那个构建脚本
  用到的 crate（bindgen 等等）。Skia 的第三方代码按 Skia 的 DEPS 文件锁定的 commit 来；gn 由 Skia 自己的
  `bin/fetch-gn` 取，ninja 用 Homebrew 装：都是构建工具，和 mpv 用 meson、ninja 一样。脚本检查包的名字、
  版本、每个目标文件是不是为 macOS 12.0 编的，再把包连同许可证和 BUILD-INFO.txt 打在一起。
- `.github/workflows/skia.yml` 在 `skia-<skia-bindings 版本>-<n>` 这样的 tag 上构建，用 `--latest=false`
  发布，和 mpv.yml 一样。
- `scripts/fetch-skia.sh` 把锁定的包下载到 `vendor/skia/bin/`，检查 SHA-256，再写 `.cargo/config.toml`
  （不进 git），内容是
  `[env] SKIA_BINARIES_URL = "file://<仓库路径>/vendor/skia/bin/skia-binaries-{key}.tar.gz"`：
  skia-bindings 只认带绝对路径的 `file://` 地址，而进 git 的文件写不出绝对路径。
- Cargo.toml 要求 `skia-bindings` 带 `no-compile`，版本和 Cargo.lock 里的完全一致。地址没指向
  `vendor/skia/bin/` 时，`build.rs` 停下来并提示"先运行 scripts/fetch-skia.sh"。（没跑这个脚本时，
  skia-bindings 仍可能在这次被 build.rs 叫停的构建里去取 rust-skia 的包，但取来的东西不会被链接进去。）
- 升级 Slint 如果带来新的 skia-safe，就要先重做一次 vendor 构建：Cargo.lock 里 skia-bindings 的版本、
  `vendor/skia/build.sh` 开头的锁定块及其 `Cargo.lock`、`scripts/fetch-skia.sh` 要一起改。

### 窗口

- Cargo.toml：`slint` 不用默认功能，只留 app 用到的，加上 `renderer-skia` 和 `unstable-wgpu-30`；FemtoVG 和
  `glow` 不再是 app 自己的依赖。软件渲染器留着：测试图片和计时测试用它画，而且构建和测试用同一组功能，
  UI 生成的那份 Rust 才只编译一次。app 运行时绝不会退到它上面。
- `select_backend()` 要求 Metal 上的 wgpu 和 `skia-wgpu` 渲染器。做不到的 Mac 在启动时报错，而不是换一个
  渲染器。

### 视频帧

- `src/platform/video.rs`，放在平台层，因为它全是 macOS 的东西：不带窗口的 OpenGL 上下文，每个播放器一个，
  连同 FinchGram 调用的那几个 OpenGL 函数（手写声明；mpv 自己在程序已加载的库里找它要的 OpenGL 函数）；
  以及播放器画进去的那块 IOSurface，既是 OpenGL 的帧缓冲，也是交给 Slint 的纹理，视频尺寸变了就重建。
  别的系统的做法以后放在它旁边，用同样的名字。
- 播放器启动时就在自己的上下文里创建 mpv 的渲染器，所以文件不必再等窗口先渲染一次。什么时候画仍由窗口的
  渲染通知器决定：每次渲染之前，播放器进入自己的上下文，让 mpv 画最新一帧，flush，再退出。窗口的渲染器没了
  的时候（窗口被隐藏），surface 跟着释放，因为它的纹理属于那个渲染器的设备；mpv 的渲染器留着。
- 程序结束之前，凡是持有 wgpu 纹理的东西都先放掉，弹出窗口也在那时关闭：不能在线程自己的变量被销毁的过程中
  拆掉 Metal 设备。
- `src/player/frames.rs` 在 app 本身里把这一切检查一遍（`cargo test video_frames -- --ignored`）：app 自己的
  播放器、app 自己的窗口，由 Skia 经 Metal 画进一张纹理再读回来。两段颜色已知的小视频，一段由 FFmpeg 解码，
  一段由 VideoToolbox 解码：颜色、上下左右、每一帧按顺序出现，以及窗口的渲染器没了之后，那一帧在新的 surface
  里重新出现。

### 看 Skia 画出来的样子

- `src/offscreen.rs` 就是那个从不上屏的窗口：Slint 的 Skia 渲染器经 Metal 画进一张纹理
  （`slint::platform::skia_renderer::SkiaWGPU30Renderer`），再读回来。它需要 GPU，不需要显示器。
- `cargo test skia_pictures -- --ignored` 用它把截图测试画的每一页（`src/screenshots.rs`：每套主题、浅色和深色、
  假数据）都画一遍，放进 `target/screenshots-skia/`，和软件渲染器画的 `target/screenshots/` 并排：文字、圆角和
  阴影是 Skia 画出来的样子。

### 留下的，以及要等谁

- mpv 的 OpenGL：等 libmpv 在 macOS 上有别的画法。
- 构建里的 Skia OpenGL 后端、`glow` 和 `glutin`：Slint 的 Skia 渲染器无条件要它们；程序为了它们和 mpv
  仍然链接 OpenGL.framework。
- resvg 和 tiny-skia：不管用哪个渲染器，Slint 都用 resvg 栅格化每一张 SVG 图片（`i-slint-core` 的
  `graphics/image/svg.rs`），它的 Skia 渲染器自己就要求这个功能。

## 已定：Slint 发布成什么样就用什么样

维护者，2026-11-12："你别fork slint，也别修slint，别改slint的任何东西"，"内存问题等到slint发新版自然就解决了"。
所以 0.8.0 用原样的 Slint 1.18.1 构建，在那之前就带着上面说的内存占用。

Slint 自己已经改了：从 2026-10-26 起，它的主分支把字体文件和像素缓冲区交给 Skia 时不再拷贝（提交 35c801a7，
"share image and font data with Skia"，对应问题 #14398；列在 1.19.0 的更新日志里，到 2026-11-12 还没发布）。
那一版带来 skia-safe 0.156.0，所以升级要先重做一次 Skia 的 vendor 构建（见"怎么做的"），它用的 wgpu 也可能
比 `wgpu_30` 新。有一块不会变：macOS 上 Skia 读不了字体合集里第一个以外的字重
（issues.skia.org/issues/310510989），Slint 只能把那个字重重建一份，苹方每多用一个字重 12.6 MB。

## 还没验证的

- macOS 12，app 支持的最老版本。
- app 在屏幕上是什么样：Skia 画的各页图片是画进纹理里的，一个点一个像素，窗口本身还没有人看过。由维护者在
  安装好的 app 里看：三套主题浅色和深色下的文字（按屏幕自己的缩放）、头像的圆角、阴影、透明标题栏、
  截图工具的覆盖层（第一帧、十字光标）、视频、弹出窗口、滚动很长的聊天。
- 连续运行几小时、插拔显示器、休眠和唤醒。
- 现在的 app 被别的窗口盖住时，是否也像测试窗口那样丢视频帧。

## 以后再说，不在这次里

- **我们自己的图改用 Skia 画，resvg 从 Cargo.toml 删掉。** 截图工具出的图（`src/screenshot/export.rs`：
  标注先写成 SVG，由 resvg 画在选区上）和菜单栏图标（`src/platform/macos.rs` 的 `mark_png`）可以改用 Skia
  画：矩形、椭圆和它们现成的路径（`Path::from_svg`）；文字用 parley 在只含打包字体的字体集里排版，再按字形
  画出来，和 Slint 的 Skia 渲染器的做法一样（用 Skia 自己的 `textlayout` 会让包变成 17 MB）。`resvg` 会从
  Cargo.toml 里消失，但仍经由 Slint 留在构建里。等第一个要画更多东西的功能一起做。
- **Slint 自己的 resvg**：要去掉它，得让 Slint 用 Skia 的 SVG 模块画 SVG，这是 Slint 里的改动（向上游提
  issue 或 pull request；不 fork）。
- **动态贴纸**：TGS 贴纸就是 gzip 压缩的 Lottie 动画，Skia 的 Skottie 能播（skia-safe 的 `skottie` 功能，
  它需要 `textlayout`），可以不用 rlottie（architecture.zh-Hans.md 待定问题 3）。能不能把 Telegram 的贴纸
  播对，要拿真贴纸试。
- **mpv 离开界面线程**：有了自己的上下文，mpv 的绘制不必再发生在窗口渲染的地方；放到它自己的线程上，上面说的
  那个等待就不会占着界面线程了。这需要两块 surface 轮换。
- **PDF**：Skia 只会写 PDF，不会读也不会改；看 PDF 走 macOS 的 PDFKit。
- `image` crate 留着：不管用什么画，Slint 都用它解码图片。
