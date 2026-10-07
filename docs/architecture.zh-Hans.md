# 架构：app 是外壳，Telegram 交给 TDLib

[English](architecture.md)

状态：方向已确认（2026-09-29），照酷丸工具箱的架构来做。外壳、finchgram-tdlib 和 libmpv 已经在 macOS 上跑起来了，
登录、聊天列表、文字消息、图片和视频、消息操作（右键菜单：回复、编辑、拷贝、转发、举报、删除、多选），以及新消息通知都能用了，
界面是设计稿的三套主题。还没定的事列在最后。

## 为什么

- 一个 Telegram 客户端其实是两件事：一是协议和它的本地状态（MTProto、加密、聊天和消息的数据库、文件、
  和服务器保持同步），二是页面。TDLib 是 Telegram 官方的库，第一件事它全包了。FinchGram 要做的是第二件，
  再加上媒体中心。
- TDLib 是 C++ 写的。把它放进一个单独的程序里，它就不会混进 app 自己的代码：app 用普通的 `cargo build`
  就能编译，不需要 C++ 工具链，也没有 FFI；TDLib 崩了，窗口也不会跟着挂。
- FinchGram 先做 macOS，以后做 Windows 和 Linux，下面这些规则给它们留好了路。

## 思路

```
FinchGram（外壳：Rust + Slint）
│  页面 · app 的状态 · 设置 · 更新 · 通知
│  跟 Telegram 打交道只经过 src/telegram/
▼
finchgram-tdlib   可执行文件旁边的一个独立程序，就像酷丸工具箱的 ffmpeg：
                  锁定版本的 TDLib，加上一个很小的外壳程序，通过标准输入输出传递 TDLib 的 JSON
```

## 名词

- **外壳**：app 本身，包括页面、app 的状态、设置、更新。它知道聊天长什么样、用户能对它做什么，
  但不管 Telegram 的协议是怎么回事。
- **finchgram-tdlib**：由本仓库的 vendor 构建从锁定的源码编译出来的 TDLib，加上我们的外壳程序
  （`vendor/tdlib/host/main.cpp`）。只有它跟 Telegram 通信。
- **适配层**：`src/telegram/`，唯一跟 finchgram-tdlib 打交道的代码。
- **平台层**：外壳里随操作系统而不同的那几块：通知、托盘、Dock 角标、安全存储、打开链接、
  在文件管理器里显示文件、更新。

## 原则

1. 外壳从不自己讲 Telegram 的协议，也从不链接 TDLib。一切都经过 finchgram-tdlib 和适配层。
2. TDLib 保留自己的 JSON 接口，就像 FFmpeg 保留自己的命令行：请求、回复和更新都是 TDLib 的对象，
   跟它的 td_api.tl 定义的一模一样，一行一个。上面不再套我们自己的协议。
3. 一个程序跑一个 TDLib client，也就是一个账号。标准输入关闭时，它让 TDLib 关闭并把数据库写完，然后退出；
   退出登录之后它会自己退出。
4. 崩溃不会带垮窗口。finchgram-tdlib 意外退出时，还在等回复的请求都会失败，适配层再把它启动起来：
   TDLib 的数据库在磁盘上，所以能接着用。启动后 30 秒内就退出的，说明它本身坏了，app 会直接报错，
   而不是一遍遍地重启。
5. 版本是约定的一部分。`src/telegram/api.rs` 是照着某一个 TDLib 版本的 td_api.tl 写的，就是
   `vendor/tdlib/build.sh` 里锁定的那个。适配层的第一个请求就是问 finchgram-tdlib 的版本，
   对不上就是打包错误。这些类型是手写的，只写 FinchGram 用到的部分；TDLib 新加的字段会被忽略。
6. 文件只传路径。TDLib 把文件下载到它的文件目录，告诉我们路径，外壳从那里读。管道里不传大东西。
7. 开发规范照旧（[conventions.md](conventions.zh-Hans.md)）：所有东西都随 app 一起打包，由我们自己的 CI
   从锁定的源码构建，不用用户机器上的任何东西。
8. 平台相关的代码只放在平台层。其他全是共用的 Rust。
9. Telegram 对 API 客户端的条款是设计的一部分（core.telegram.org/api/terms）：用我们自己的 api_id；
   名字里不带 “Telegram”，也不用它的 logo；app 要说明自己用的是 Telegram API；频道里要显示官方的赞助消息；
   不干扰已读回执、正在输入、在线状态和阅后即焚；从 Telegram 拿到的任何数据都不用来训练或喂给 AI。

## 外壳怎么跟 finchgram-tdlib 通信

- 这个程序只在一个地方找：可执行文件旁边（.app 里的 `Contents/MacOS/`；`cargo run` 时是
  `target/<profile>/`，由 build.rs 从 `vendor/tdlib/bin/` 复制过去）。
- 标准输入：请求，一行一个 JSON 对象。每个请求带一个 `"@extra"` 编号，TDLib 会把它原样放进回复里，
  回复再交给发请求时给的回调。
- 标准输出：回复和更新，一行一个。一个读取线程负责解析，只有在它的收件箱原本是空的时候才去叫醒 UI 线程，
  所以一连串更新会作为一批到达。现在还没人关心的更新直接在那里丢掉。
- 标准错误：TDLib 的日志（默认只输出错误，外壳需要时再调高）。
- `finchgram-tdlib --version` 输出 TDLib 的版本和 commit，vendor 构建会检查它。

## 线程

- UI 线程跑 Slint 的事件循环，并持有 app 的状态：TDLib 告诉我们的聊天、用户和消息，以及页面显示用的
  Slint model。状态只在这里、根据更新、成批地改，不用锁。
- 每个 finchgram-tdlib 有一个写线程和一个读线程。
- 耗时的活（解码图片、读文件）放到单独的线程里做，做完通过事件循环汇报，跟酷丸工具箱一样。
  图片在 UI 线程之外解码成 `SharedPixelBuffer`，到 UI 线程上再变成 `slint::Image`。
- 没有 async 运行时：网络全是 TDLib 在做。

## 页面：三套主题

设计稿有三套主题：工作台（默认）、报纸和终端。每套有自己的聊天窗口（`ui/workbench/`、`ui/broadsheet/`、
`ui/terminal/`）；登录、设置和个人资料是三套共用的页面（`ui/pages/`），颜色、字体和形状跟着当前主题
（`ui/look.slint`）。

- 主题只是页面。三套读的是同一组 global（`ui/state.slint`、`ui/telegram.slint`），调用同一组回调，由同一份
  Rust 代码填充和响应。一个功能在 Rust 里只写一次，画三遍；设计稿的规矩是先做工作台，再同步到另外两套，
  保证三套始终一致。
- 切换立即生效（设置 › 外观，或“视图”菜单），不用重启：窗口只是换了页面，别的都不变，连打开的聊天都还在。
- 设计稿里有、FinchGram 还没做的功能一律灰掉，不装作能用。

## 外壳里的代码

- `src/telegram/`：适配层。`process.rs` 负责运行那个程序，`api.rs` 是我们用到的 TDLib 类型，`mod.rs` 负责发请求、
  分发回复、重新启动和转交更新。`store.rs` 保存 TDLib 告诉我们的东西（聊天、用户、群组、文件夹、打开的聊天的消息），
  每批更新之后把页面用的 model 更新到最新，只动真正变了的行。打开的聊天只显示最新的 100 条消息，往上翻时再增加：
  页面会把每一行都排出来，其中任何一行一变就要把所有行重新量一遍，所以行数就是每条新消息的代价。
  `login.rs`、`chats.rs`、`conversation.rs`、`actions.rs`（消息操作）、`account.rs` 和 `password.rs`（两步验证）负责页面要做的事；`files.rs` 负责下载文件，`avatars.rs` 负责给要显示头像的行取来聊天和联系人的头像（取到之前是首字母色块，没有头像的一直是色块），`viewer.rs` 给媒体查看器提供内容，`rich_text.rs` 把消息里带格式的文字（粗体、链接等）转换成 Slint 的富文本。
- 通知（`notifications.rs`，设置 → 通知与声音）用的是 TDLib 自己的：用它的 `notification_group_count_max` 选项打开后，
  该不该通知由 TDLib 决定。它遵循每个聊天的免打扰和每类聊天的设置（Telegram 自己的、跟着账号走的设置），
  账号的另一台设备正在用时会稍等一下，消息在任何地方读过之后会把通知收回。外壳只负责显示 TDLib 加的通知
  （用户正看着的不显示：窗口在前台时打开着的那个聊天），收回 TDLib 去掉的通知；点通知会打开那个聊天。
- Telegram 官方应用让别人看到的，FinchGram 也一样（API 条款）：消息看到了才算已读（`conversation.rs`，viewMessages），
  用户写字时对方能看到“正在输入”（sendChatAction），窗口在前台并且有人在用时账号是在线状态（`online.rs`，
  TDLib 的 `online` 选项）。Telegram 据在线状态压下用户其他设备上的通知，TDLib 据此决定 FinchGram 的通知什么时候出。
  阅后即焚的照片和视频（看一次，或限时）在聊天里只显示模糊的小图，用户在媒体查看器里打开它时才告诉
  TDLib（`viewer.rs`，openMessageContent），之后它按 Telegram 官方应用的做法失效。查看器只在 TDLib 说内容
  可以保存时才提供保存（聊天可以限制保存）。
- 在消息上任何地方点右键都会弹出它的菜单。消息里带格式的文字（Slint 的 StyledText）会把所有点击留给自己，
  所以右键是在窗口自己的事件里看到的（`mod.rs`，通过 winit），记进一个全局属性，再由指针下的那一行去要菜单。
  菜单里有哪些项，看 TDLib 说这条消息能做什么（getMessageProperties）。
- 聊天的菜单（在列表里对它点右键，`chats.rs`）可以免打扰、置顶、标为已读，以及把它加进账号的某个文件夹或从中移出。
  最后这项 TDLib 没有单独的请求：先把整个文件夹取下来（getChatFolder），改它手选的聊天，再整个发回去（editChatFolder）。
  按类别收聊天的文件夹（联系人、群组……）会把移出的聊天照样留着，所以在那种文件夹里还要把它按名字排除掉。
- `src/player/`：用 libmpv 播放视频，画进窗口（媒体查看器的播放器）。
- `src/platform/`：平台层。目前有：账号的会话列表里怎么称呼这台设备、macOS 上的透明标题栏、打开链接、剪贴板（文字和图片）；以及 macOS 上
  （设置 → 通用）关闭窗口后留在 Dock 里，即设计稿的“关闭窗口时：最小化到托盘”（点 Dock 图标重新打开窗口，退出时先让
  TDLib 关好）、菜单栏图标、开机时启动（SMAppService，macOS 13 及以上）；通过系统的 UserNotifications 框架发通知
  （只有打包成 .app 才有，`cargo run` 时没有）、Dock 图标上的未读数，以及让 Dock 图标跳一下（设置 → 通知与声音）。
- `src/update.rs`：自动更新（[conventions.md](conventions.zh-Hans.md) 第 3 节）。
- `src/settings.rs`、`src/i18n.rs`、`src/fonts.rs`（界面字体，编译进可执行文件）。
- `src/images.rs`：图片（照片、视频封面、聊天和联系人的头像、消息里自带的小预览图）在界面线程之外解码，并缓存最近的。
- `src/screenshots.rs`：用 Slint 的软件渲染器和假数据，把每套主题的每个页面（浅色和深色）画成 PNG
  （`cargo test screenshots -- --ignored`）。
- `src/telegram/timing.rs`：用同样的方式、很多假聊天和假消息，量页面跟上 store 要多久
  （`cargo test --release refresh_timing -- --ignored --nocapture`）。
- `ui/`：`app.slint`（窗口：菜单，以及显示哪个页面）、`state.slint` 和 `telegram.slint`（Rust 和页面共用的 global）、
  `look.slint`、`format.slint`（按界面语言写出的日期、数量和消息类型）、`widgets.slint` 和 `chat.slint`（共用的部件）、
  `viewer.slint`（覆盖整个窗口的媒体查看器）、`pages/`，以及三套主题各自的目录。

## 平台

| | macOS | Windows | Linux |
|---|---|---|---|
| 时间 | 现在 | 计划中 | 以后 |
| finchgram-tdlib | 我们的 CI 构建（Apple silicon） | 计划中 | 计划中 |
| 更新 | 我们自己的更新器 | 我们自己的更新器 | 我们自己的更新器或系统包 |

## 现在不做

- **多账号**。每个账号一个 finchgram-tdlib，各自有自己的数据库目录，外壳在它们之间切换。需要先设计。
- **通话**。TDLib 只管信令；媒体部分（tgcalls）本身就是一个大项目。
- **别人写的插件**。先得有信任模型：谁来签名、用户批准什么、怎么隔离。

## 还没定的事

1. **数据库加密**：TDLib 可以用一个密钥加密它的数据库，密钥放在系统的安全存储（钥匙串）里。
   在那之前，数据库只靠用户的系统账号保护。
2. **下载的文件放哪**：暂时放在数据库旁边。大小上限和缓存目录（`~/Library/Caches`）需要设计。
3. **媒体播放**（媒体中心）：语音消息（Opus）、视频、GIF 和动态贴纸（WebM、Lottie）。2026-09-30 定了：用 mpv。
   它以 libmpv 的形式出现，跟 finchgram-tdlib 一样由 `vendor/mpv/build.sh` 从锁定的源码构建：mpv 加上 FFmpeg、
   libplacebo 和 libass（FreeType、FriBidi、HarfBuzz），全部静态链接进一个只依赖 macOS 的库；app 链接它，
   并把它放在 app 包的 Frameworks 目录里。窗口每次渲染时，mpv 在 Slint 自己的 OpenGL 上下文里把当前帧画进我们的
   一张纹理（`src/player/`），能硬解的用 VideoToolbox；媒体查看器显示这张纹理。代价是 libmpv 在 app 的进程里
   解码，一个故意构造的视频如果弄坏了解码器，窗口会跟着一起崩。FFmpeg 构建时
   去掉了网络、编码器和设备，并跟着上游升级；如果还不够，就把播放挪进一个沙箱里的辅助进程，由它把画面交给 app
   （macOS 上用 IOSurface）。Lottie 贴纸要另想办法（rlottie），WebM 贴纸要能保留透明通道的 VP9 解码（libvpx；
   FFmpeg 自带的 VP9 解码器会丢掉透明通道）。在那之前，动态贴纸只显示它的静态缩略图；静态贴纸照原样显示。
4. **渲染**：暂时用 Slint 的默认渲染器（FemtoVG）。设计稿的字体都已打包，中文也在内（Noto Sans SC）；消息里的
   日文、韩文和彩色 emoji 暂时用系统字体，等界面支持这些语言时再说。Skia 得从源码构建才行：它的 Rust 绑定默认会
   下载预编译的库，这是 conventions.md 不允许的。
