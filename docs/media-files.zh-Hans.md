# 媒体文件：留下什么、什么时候取、还有什么要做

[English](media-files.md)

状态：前两节是 2026-10-09 时 app 的实际做法（v0.3.14 及其后的那个修复）；"待做"一节里的东西都还没做。
维护者提出能不能提前下载视频，并且觉得照片和视频每次都在重新下载：其实没有重新下载，只是 app 取得晚，
而且把等待显示了出来。

## 留下什么

- TDLib 把每个文件都下载到数据库旁边它自己的文件夹里：macOS 上是
  `~/Library/Application Support/FinchGram/tdlib/`，里面有 `photos/`、`videos/`、`thumbnails/`、
  `animations/`、`profile_photos/` 等（setTdlibParameters 里 `files_directory` 留空，`src/telegram/mod.rs`）。
  它的文件数据库是开着的（`use_file_database`），所以重启之后 TDLib 也知道自己有哪些文件。
  一个文件只下载一次；再要的时候立刻就有。
- 什么都不删。app 没有设过 TDLib 的任何存储选项，TDLib 自带的清理器（`use_storage_optimizer`）默认关着。
  维护者的 Mac 用了十天之后有 393 MB：2028 张照片（171 MB）、62 个视频（159 MB）、662 张缩略图（8.4 MB）。
- 内存里（`store.rs`）给消息行和查看器留着解码好的图片：200 张图片（长边不超过 2048，`images.rs`）、
  1000 张预览图、2048 个头像，满了先丢最旧的。

## 文件怎么要，什么时候取

`files.rs` 只有一个入口：`download(file, priority, ready)`。磁盘上已经完整的文件立刻给出路径。
否则每个文件只向 TDLib 发一次 `downloadFile`，不管有多少处在等它，`updateFile` 说完整了就完整了。
屏幕上的先取（优先级 16），列表里的头像其次（8），用户在查看器里打开的排在一切之前（32）。
app 手里的文件状态落后时（消息在它的文件下完之前就加载了），请求照样发给 TDLib，TDLib 从磁盘上直接回答：
一个来回，什么都不传。

- 聊天里的照片立刻显示它的 minithumbnail（消息里自带的几百字节的模糊小图）。它那一行进入视野时，
  `load-picture` 取长边不小于 640 的那个尺寸，消息行显示它。最大尺寸是另一个文件：查看器第一次打开这张照片时才取，
  之后就留着。
- 聊天里的视频或 GIF 只取封面，视频本身一点不取。
- 查看器把视频整个下载完再交给 mpv 播（`viewer.rs`，`start_video`）；在那之前转圈。已经在磁盘上的视频，
  mpv 一打开就开始，不到一秒。GIF 同样，循环播。

所以没有任何东西下载两次，但每张照片打开都是先模糊后清晰，每个视频第一次都要等整个文件：
这就是看起来像重新下载的原因。（到 v0.3.14 为止，和上一个尺寸相同的视频播放时还会停在封面上，
像一个永远下不完的下载；v0.3.14 之后修了。）

## 待做

按会做的顺序。都不改文件的保存方式。

1. **原图随格子一起取。** 照片那一行进入视野时，格子之后再低优先级地把最大尺寸也取下来，查看器一打开就是清晰的。
   每张几百 KB；`conversation.rs`（`load_picture`）里几行。
2. **当前聊天的视频和 GIF 提前取**，限制大小、低优先级，消息行进入视野或消息到来时取。大小限制跟着 Telegram
   为账号保存的自动下载设置走（TDLib 的 `getAutoDownloadSettingsPresets`，取 `high` 档，也就是用户的手机在
   Wi‑Fi 下的做法，`max_video_file_size` 和 `preload_large_videos`），这样在设计稿有这一页之前不用自己加设置页。
3. **边下边播。** Telegram 官方应用不管视频多大都在一秒内开始播，因为它们发出的视频把索引放在文件开头
   （`supports_streaming`）。mpv 可以通过 app 自己提供的流来读（libmpv 的 `mpv_stream_cb_add_ro`，
   一个带文件 id 的 `finchgram://` 地址）：读回调直接读 TDLib 正在写的那个文件，等到已下载的前缀
   （`updateFile` 的 `downloaded_prefix_size`）盖住 mpv 要的范围为止；跳到前缀之外就挪 TDLib 的下载偏移量
   （带 `offset` 的 `downloadFile`）。文件最后还是完整地留在磁盘上，和现在一样。需要在 `src/player/mpv.rs`
   里加流回调、查看器加"缓冲中"状态、进度条显示已到的部分。
4. **存储上限和清理**，[architecture.zh-Hans.md](architecture.zh-Hans.md) 的待定问题 2：TDLib 的清理器和它的四个选项
   （`storage_max_files_size`、`storage_max_time_from_last_access`、`storage_max_file_count`、
   `storage_immunity_delay`）、`optimizeStorage`、`getStorageStatistics` 都是现成的；设置 → Data & storage 在设计稿里有，
   app 里是灰的。提前下载会让这件事更急：应该和第 2 项一起做。
