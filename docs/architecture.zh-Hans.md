# 架构：app 是外壳，Telegram 交给 TDLib

[English](architecture.md)

状态：方向已确认（2026-09-29），照酷丸工具箱的架构来做。外壳和 finchgram-tdlib 已经在 macOS 上跑起来了，
页面按设计稿来做。还没定的事列在最后。

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

## 外壳里的代码

- `src/telegram/`：适配层。`process.rs` 负责运行那个程序，`api.rs` 是我们用到的 TDLib 类型，
  `mod.rs` 负责发请求、分发回复、重新启动和跟进登录状态。
- `src/platform/`：平台层（目前只有一样：账号的会话列表里怎么称呼这台设备）。
- `src/update.rs`：自动更新（[conventions.md](conventions.zh-Hans.md) 第 3 节）。
- `src/settings.rs`、`src/i18n.rs`；定下设计稿的字体后加 `src/fonts.rs`。
- `ui/`：`app.slint`、`state.slint`（Rust 和页面共用的 global）、`theme.slint`，以及各个页面。

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
3. **媒体播放**（媒体中心）：语音消息（Opus）、视频、GIF 和动态贴纸（WebM、Lottie）。解码需要一个自己的引擎，
   跟 TDLib 一样从锁定的源码构建：要么像酷丸工具箱那样用独立程序形式的 FFmpeg，要么在我们自己的程序里用一个库。
   做第一个媒体页面时再定。
4. **渲染**：暂时用 Slint 的默认渲染器（FemtoVG）；中日韩文字和彩色 emoji 要等设计稿的字体定了再验证。
   Skia 得从源码构建才行：它的 Rust 绑定默认会下载预编译的库，这是 conventions.md 不允许的。
