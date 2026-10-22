# 崩溃报告

FinchGram 崩溃时有什么可读，以及怎么读。维护者测的是装好的 release（docs/conventions.md，先发布），
所以崩溃首先是出事的那台 Mac 上的一个文件。

## 留下了什么

- **macOS 的报告。** 每次崩溃，不论哪一种（展开不了的 Rust panic、libmpv 或 finchgram-tdlib 里的错误），
  都落在 `~/Library/Logs/DiagnosticReports/finchgram-<日期>.ips`；过一阵 macOS 会把旧的挪进那里的 `Retired/`。
  它是 JSON：版本、时间、信号、每个线程的栈。app 去掉了符号，所以它自己的栏位只是二进制里的偏移量，不是名字。
- **panic.log。** 从 v0.5.1 起，app 里的 Rust panic 会把内容和位置，连同时间、版本和线程，写进 settings.toml
  旁边的 `panic.log`（`~/Library/Application Support/FinchGram/`；`src/main.rs` 的 `log_panics`）：
  从 Dock 启动的 app，默认 hook 写到 stderr 的东西无处可去。Slint 回调里的 panic 会结束整个 app
  （docs/architecture.md，线程），所以报告里只有 abort() 和 panic 的机制，原因在 panic.log 里。
- **每个 release 的符号表。** 从 v0.5.2 起，release 里 app 旁边多一个 `finchgram-<version>-macos-arm64.dSYM.zip`：
  正是那个二进制的函数名和行号表，UUID 相同。Cargo.toml 里的 `debug = "line-tables-only"` 和
  `split-debuginfo = "packed"` 让它们在构建时进入 `finchgram.dSYM`，二进制本身照旧去符号、不变大。符号表压缩后
  约 85 MB（44 MB 代码的行号表，连同哪里内联了什么），构建多花五分钟左右。有了它，之后任何版本的报告都能读成带
  文件和行号的函数名。

## 读一份报告

`scripts/crash-report.py` 负责：

    scripts/crash-report.py                    # 这台 Mac 上 app 最新的一份报告
    scripts/crash-report.py REPORT.ips         # 指定的那份；别人发来的也行
    scripts/crash-report.py --list             # 这台 Mac 上 app 的每一份报告

它打印发生了什么（版本、时间、信号），崩掉的那个线程一帧一帧的栈，app 的栏位带上名字（Xcode 命令行工具里的
atos；c++filt 把 Rust 的 mangled 名字还原成路径），以及崩溃前两分钟内 panic.log 里的行。符号表来自那个版本的
release（`gh release download`，存在 `~/Library/Caches/FinchGram/symbols/`），本地构建崩了就用
`target/release/finchgram.dSYM`；别的构建的 dSYM 会被跳过，因为名字会是错的（`--symbols DSYM` 可以强行指定，带警告）。

v0.5.2 之前的 release 没留符号表。那些版本的报告要用发布运行所用的 rustc（运行日志里有）重新编一份，`strip = false`、
不带调试信息；CI 的 Xcode 编译 C 的部分，所以代码的偏移量一段一段地略有不同，得试着平移直到栏位对上函数名。
v0.5.0 在搜索里敲第一个键就崩的那次就是这样读出来的，一次就够了。

## 现在不做

- app 不往任何地方发自己的东西：它只跟 Telegram 的服务器、GitHub（更新）和用户自己填的 WebDAV 服务器说话。
  遇到崩溃的用户手动把 .ips 文件和 panic.log 发来；“发送报告”这一步得先有设计。
- finchgram-tdlib 和 libmpv 的栏位仍是偏移量：它们的构建（`vendor/tdlib/`、`vendor/mpv/`）没留符号表。
  哪天它们里面的崩溃需要了，它们的工作流可以照样留一份 dSYM。
