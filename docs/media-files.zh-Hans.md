# 媒体文件：留下什么、什么时候取、还有什么要做

[English](media-files.md)

状态：前两节是 2026-10-29 时 app 的实际做法，边下边播（原来"待做"的第 3 项）就是那天做的；"待做"里剩下的还没做。
维护者提出能不能提前下载视频，并且觉得照片和视频每次都在重新下载：其实没有重新下载，只是 app 取得晚，
而且把等待显示了出来。

## 留下什么

- TDLib 把每个文件都下载到 `~/Documents/FinchGram/media_cache/` 里它自己的文件夹中：`photos/`、`videos/`、
  `thumbnails/`、`animations/`、`profile_photos/` 等（setTdlibParameters 里的 `files_directory`，`src/telegram/mod.rs`）。
  数据库本身在 `~/Library/Application Support/FinchGram/tdlib/`。
  它的文件数据库是开着的（`use_file_database`），所以重启之后 TDLib 也知道自己有哪些文件。
  一个文件只下载一次；再要的时候立刻就有。
- 什么都不删。app 没有设过 TDLib 的任何存储选项，TDLib 自带的清理器（`use_storage_optimizer`）默认关着。
  维护者的 Mac 用了十天之后有 393 MB：2028 张照片（171 MB）、62 个视频（159 MB）、662 张缩略图（8.4 MB）。
- 内存里（`store.rs`）给消息行和查看器留着解码好的图片：200 张图片（长边不超过 2048，`images.rs`）、
  1000 张预览图、2048 个头像，满了先丢最旧的。

## 文件怎么要，什么时候取

`files.rs` 有两个入口：`download(file, priority, ready)`，和给查看器用的 `stream(file, priority, …)`（见下）。
磁盘上已经完整的文件立刻给出路径。否则每个文件只向 TDLib 发一次 `downloadFile`，不管有多少处在等它（后来者要得
更急时再发一次，TDLib 会把它提前），`updateFile` 说完整了就完整了；一个下载过的文件完整时，日志里记一行多少 MB 用了
多少秒。屏幕上的先取（优先级 16），列表里的头像其次（8），用户在查看器里打开的排在一切之前（32）。
app 手里的文件状态落后时（消息在它的文件下完之前就加载了），请求照样发给 TDLib，TDLib 从磁盘上直接回答：
一个来回，什么都不传。

- 聊天里的照片立刻显示它的 minithumbnail（消息里自带的几百字节的模糊小图）。它那一行进入视野时，
  `load-picture` 取长边不小于 640 的那个尺寸，消息行显示它。最大尺寸是另一个文件：查看器第一次打开这张照片时才取，
  之后就留着。
- 聊天里的视频或 GIF 只取封面，视频本身一点不取。
- 查看器边下边播（`viewer.rs`，`start_video`）：mpv 通过 app 自己提供的流（`files::stream`、`src/player/stream.rs`，
  libmpv 的 `mpv_stream_cb_add_ro`，地址是 `finchgram://`）直接读 TDLib 正在写的那个文件，要的字节还没到就等
  （mpv 报告在等缓存时转圈），跳到没到的地方就把下载挪过去（带 `offset` 的 `downloadFile`，TDLib 留着已有的部分、
  从那里接着下）；进度条上显示已经到了的那段。索引放在开头的视频（`supports_streaming`，Telegram 官方应用发出的都是）
  一两秒内开始；索引在末尾的，mpv 先取末尾再取开头。文件最后还是完整地留在磁盘上，和以前一样；已经在磁盘上的
  直接从文件播。GIF 同样，循环播。

所以没有任何东西下载两次，但每张照片打开都是先模糊后清晰，而 2026-10-29 之前每个视频第一次都要等整个文件：
这就是过去看起来像重新下载的原因。（到 v0.3.14 为止，和上一个尺寸相同的视频播放时还会停在封面上，
像一个永远下不完的下载；v0.3.14 之后修了。）

## 待做

按会做的顺序。都不改文件的保存方式。

1. **原图随格子一起取。** 照片那一行进入视野时，格子之后再低优先级地把最大尺寸也取下来，查看器一打开就是清晰的。
   每张几百 KB；`conversation.rs`（`load_picture`）里几行。
2. **当前聊天的视频和 GIF 提前取**，限制大小、低优先级，消息行进入视野或消息到来时取。大小限制跟着 Telegram
   为账号保存的自动下载设置走（TDLib 的 `getAutoDownloadSettingsPresets`，取 `high` 档，也就是用户的手机在
   Wi‑Fi 下的做法，`max_video_file_size` 和 `preload_large_videos`），这样在设计稿有这一页之前不用自己加设置页。
3. **存储上限和清理**，[architecture.zh-Hans.md](architecture.zh-Hans.md) 的待定问题 2：TDLib 的清理器和它的四个选项
   （`storage_max_files_size`、`storage_max_time_from_last_access`、`storage_max_file_count`、
   `storage_immunity_delay`）、`optimizeStorage`、`getStorageStatistics` 都是现成的；设置 → Data & storage 在设计稿里有，
   app 里是灰的。提前下载会让这件事更急：应该和第 2 项一起做。
