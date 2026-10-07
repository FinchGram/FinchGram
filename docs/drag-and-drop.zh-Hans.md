# 拖放：从 Finder 拖来的文件能到达窗口

[English](drag-and-drop.md)

状态：已验证（2026-10-06），用一个探针构建，在固定版本的 Slint 1.18.1 和 winit 0.30.13、macOS 26 上。
app 里还没有任何相关代码：这是为发送附件（照片、视频、文件）做的核实，那个功能先出设计稿，再做。

## 问题

FinchGram 用 Slint，能不能像 Telegram 官方应用那样接住拖到窗口上的文件？发送附件的设计想在聊天上方放一个投放区。

## 怎么核实的

1. 读 Cargo registry 里的源码：winit 0.30.13 的 macOS 窗口（`src/platform_impl/macos/window_delegate.rs`），
   以及 Slint 1.18.1 的 winit 后端（`i-slint-backend-winit`：`lib.rs`、`winitwindowadapter.rs`）和编译器内置元素。
2. 一个探针构建：在 `telegram::start` 注册的窗口事件钩子里（`src/telegram/mod.rs`）加三行，打印 `HoveredFile`、
   `HoveredFileCancelled` 和 `DroppedFile`；用 `cargo run` 跑在登录页上（这台机器没有 api_id），从 Finder 拖一张 JPEG 到窗口上。

## 结论

- **窗口能收到文件。** winit 给窗口注册了 `NSFilenamesPboardType` 类型的拖放，并自己实现了 `NSDraggingDestination`：
  拖入时发 `WindowEvent::HoveredFile(path)`，每个文件一条；松手时发 `WindowEvent::DroppedFile(path)`，每个文件一条，
  带完整路径；拖出去发 `HoveredFileCancelled`。探针记录到了一次悬停和一次投放，带文件路径。
- **不经过 Slint 的 `DropArea`。** Slint 1.18 的 winit 后端从不转换这些事件，所以在 macOS 上 `DropArea` 只接 app 内部
  `DragArea` 发起的拖放。入口是 `WinitWindowAccessor::on_winit_window_event`，它把每一条 winit 窗口事件都交出来。
- **一个窗口只有一个钩子。** 一个窗口只保存一个事件过滤器，再注册一次会不声不响地替换掉前一个。`telegram::start`
  已经注册了它（窗口焦点用于在线状态、输入、右键），所以拖放的文件必须在那里处理，或者交给它调用的东西。
  一个在 `main.rs` 里、`telegram::start` 之前注册的探针什么都没记到，就是这个原因。
- **没有位置。** winit 0.30 没有实现 `draggingUpdated:`，所以悬停期间没有光标位置，`DroppedFile` 也不带位置。
  窗口只知道文件在它上面、以及松手了，不知道在哪里。

## 对发送附件意味着什么

- 整个窗口一个投放区，从第一条 `HoveredFile` 显示到 `HoveredFileCancelled` 或松手为止，而不是 Telegram Desktop
  的两个（"按照片发"和"按文件发"）。照片还是文件的选择放进松手之后的那个卡片里，那里本来就有这个选择。
- 文件以路径的形式到达，所以拖进来的东西和在打开面板里选的文件一样发送（`inputFileLocal`）。
- 投放给打开着的那个聊天，不管松手在窗口的哪里。

## 没有核实的

- 从 Finder 以外的 app 拖。有些 app（照片、邮件）给的是承诺文件而不是路径；没有 `NSFilenamesPboardType` 的拖放会被
  winit 拒绝，表现为不允许投放。
- 从剪贴板粘贴图片（⌘V）：另一条路径，走 Slint 的文本输入，不走这个钩子。
- 一次拖多个文件：按源码每个文件各发一条事件，没有试。
