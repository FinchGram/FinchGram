# 从源码构建 FinchGram

[English](development.md)

app 是一个外壳。Telegram 本身交给 TDLib，它作为一个独立程序跑在可执行文件旁边：`finchgram-tdlib`，由本仓库从锁定的
源码构建；外壳只通过 `src/telegram/` 跟它通信，用的是 TDLib 自己的 JSON。视频用同样方式构建的 libmpv 播放。
不从用户的机器上拿任何东西，构建时除了这些锁定并带校验和的产物之外什么也不下载。所有东西都公开：源码、vendor 构建
（`vendor/`）和发布。见 [architecture.zh-Hans.md](architecture.zh-Hans.md) 和
[conventions.zh-Hans.md](conventions.zh-Hans.md)。

目前开发需要 Apple silicon 的 macOS：finchgram-tdlib 暂时只为它构建。Linux 和 Windows 以后再支持。

finchgram-tdlib、libmpv、Skia 和界面字体都不进 git：`vendor/tdlib/`、`vendor/mpv/` 和 `vendor/skia/` 里只放构建
它们的脚本，字体来自 Google Fonts。clone 之后先把它们拿下来一次：

```sh
scripts/fetch-tdlib.sh    # 把锁定的 finchgram-tdlib release 下载到 vendor/tdlib/bin/ 并校验 SHA-256
scripts/fetch-mpv.sh      # 把锁定的 libmpv release 下载到 vendor/mpv/bin/ 并校验 SHA-256
scripts/fetch-skia.sh     # 把锁定的 Skia release 下载到 vendor/skia/bin/ 并校验 SHA-256，再告诉 cargo 它在哪里
scripts/fetch-fonts.sh    # 把锁定的界面字体下载到 vendor/fonts/ 并校验 SHA-256
FINCHGRAM_API_ID=… FINCHGRAM_API_HASH=… cargo run
```

- 要修改 finchgram-tdlib 本身时，在本地构建它（几分钟；需要 Xcode 或 Command Line Tools，以及
  `brew install cmake ninja gperf`，都只是构建工具）：

  ```sh
  vendor/tdlib/build.sh && vendor/tdlib/build.sh install
  ```

- `FINCHGRAM_API_ID` 和 `FINCHGRAM_API_HASH`：用你自己的，在 [my.telegram.org](https://my.telegram.org)
  → API development tools 申请。它们在编译时读取，永远不进仓库。没有它们 app 也能启动，会提示没有 API ID。
- `FINCHGRAM_TEST_DC=1 cargo run` 连 Telegram 的测试服务器，那里有单独的账号；app 会为它另外保存一个数据库。
- `cargo test screenshots -- --ignored` 用假数据把每套主题的每个页面（浅色和深色）画成图片，存到
  `target/screenshots/`：不用登录账号也能看界面。

`build.rs` 把 `vendor/tdlib/bin/finchgram-tdlib` 复制到编译出来的可执行文件旁边，所以 `cargo run`
用的和打包后的 app 是完全同一个程序；它还从 `vendor/mpv/bin/` 链接 libmpv 并同样复制过去，`vendor/fonts/`
里的字体编译进可执行文件。画窗口用的 Skia 从 `vendor/skia/bin/` 里的包链接。缺了任何一样，编译都会直接失败并
说明原因。需要 Rust 1.92 或更新。

TDLib 的数据库和下载的文件在 `~/Library/Application Support/FinchGram/tdlib/`，设置在
`~/Library/Application Support/FinchGram/settings.toml`。

另见[仓库目录](layout.zh-Hans.md)、[翻译界面](translations.zh-Hans.md)、[发布](releases.zh-Hans.md)和[开发约定](conventions.zh-Hans.md)。
