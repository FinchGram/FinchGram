# 音乐：播放你自己服务器上的音乐

[English](music.md)

状态：2026-10-31 按维护者 2026-10-30 与 2026-10-31 的决定和画布"FinchGram 音乐"（第三稿）建成，共享页面覆盖三个主题，Workbench 优先；尚未发版。

## 这是什么

一个音乐播放器，百宝箱里一个独立的工具，和下载一样。它不是以某个服务器命名的客户端：它的来源是用户自己的、会说 Subsonic API 的服务器，在工具里面添加。Navidrome、gonic、LMS、Ampache、Nextcloud Music、Koel、Airsonic-Advanced、Supysonic 都答得上这套 API，Bandcamp 也给用户买过的音乐开了 Subsonic 接口。每个来源各是一个曲库，一次看一个；曲库不合并（维护者的话）。

这一版不是什么：Jellyfin 和 Emby 是百宝箱里各自独立的工具，仍为灰色，不是这里的来源。网络电台画过，后来因为太小众去掉了。Telegram 自己的音乐消息和 Mac 上的文件夹还不是来源。这一页不牵涉 Telegram 的任何东西：和下载页一样，登录前就能打开。

## 页面

显示 → 音乐（⌥⌘M）；百宝箱里的音乐（⌘J），用户可以把它钉到 Workbench 的活动栏、Broadsheet 的侧栏或 Terminal 的状态栏；Terminal 里是 `/music`。三栏，样子和下载页一样：

- **曲库与来源**，在左边。曲库的分区：专辑、艺术家、歌曲、流派、歌单、收藏。下面是来源，每个有名字、一个 Subsonic 徽标、主机名和状态，页面打开时检查（已连接、密码被拒、无法连接、不是 Subsonic 服务器）；正在看的来源高亮，点一下换另一个，右键编辑。最后一行是添加来源。表单问名字、地址和旁边的端口（地址里带的端口会挪进端口框；空框里以 Navidrome 的 4533 作提示，不替用户假定任何值）、账号，还有"测试连接"，它 ping 服务器并说出对方是什么（"Navidrome 0.64.2 · OpenSubsonic, 7 extensions"）或者为什么不行。
- **正在看的内容**，在中间。专辑是一面封面墙（服务器没有封面时用一块写着专辑首字母的色块），有筛选框、排序（最近添加、按名称、按艺术家、最常播放、最近播放、随机，即服务器 `getAlbumList2` 的几种顺序）和刷新按钮；一次一百张，"更多专辑"再取一百。点开一张专辑：封面、艺术家、年份、流派、时长与格式，播放、随机播放、加入队列和一颗星，下面是编号的曲目；点一首就从它开始播，专辑其余的跟在后面。艺术家、流派、歌单是名字列表：艺术家打开其专辑，流派打开其专辑，歌单打开其歌曲。歌曲列出曲库里的全部歌曲，一次两百首；收藏是打过星的。歌曲列表有"全部播放"和"随机播放"。给歌或专辑打星会写回服务器（`star`、`unstar`）。
- **队列**，在右边，可以收起（播放条上的队列按钮，或"隐藏"）：从专辑或列表来的在"接下来"下面，用户之后加进来的在"你添加的"下面，正在播的高亮；点一下从那里播，× 移出一首，"清空"清掉全部。底部写着缓存占了多少。

播放条在有歌在播时站在窗口底部，每一页、每个主题的聊天窗口都有：封面、歌名和艺术家、一颗星、随机、上一首、播放或暂停、下一首、循环（关、全部、单曲）、带进度条的位置、mpv 等字节时的转圈、队列按钮、音量，以及一个打开音乐页的按钮（在音乐页上则是停止）。

## 歌怎么播

音乐有自己的一个 mpv（`src/player/music.rs`），不带视频，与查看器的、语音消息的各不相扰：歌与歌之间无缝衔接，下一首排在 mpv 播放列表里当前这首的后面，并提前打开（`gapless-audio`、`prefetch-playlist`）。

项目自建的 FFmpeg 按决定不含网络代码（docs/architecture.md），所以 mpv 不自己去取歌。取歌的是 `src/music/fetch.rs`，用 app 自己的 HTTP 客户端（ureq，除常规根证书外也信任 Keychain 里的，照顾用户自签证书的服务器）：以 Range 请求把服务器的 `stream` 地址读进缓存文件，mpv 通过 `finchgram://` 流回调（`src/player/stream.rs`，和 TDLib 送来的视频一个路数）边到边读。拖到还没到的位置，取歌就挪过去，之后再回头补上跳过的部分。整个文件留在缓存里，以自己的名字（在路上时叫 ".part"），下次这首歌直接从磁盘播。

线上送来什么格式，由服务器对名为"FinchGram"的客户端的转码设置决定；FinchGram 不特别要求什么。一首歌开始时告诉服务器（`scrobble`，`submission=false`，即"正在播放"），播过一半或四分钟（以先到者为准）再告诉一次（`submission=true`）：播放次数，以及服务器配了 Last.fm 或 ListenBrainz 时的 scrobble。

## 东西放在哪

- 来源连同密码放在 app 的数据库里（`music_sources`，布局 8），每个密码封存时绑定到它的来源（种类、地址、端口、账号），和下载服务器的密码绑定到服务器一样。
- 音量、随机、循环、队列栏是否显示、上次看的来源、缓存上限放在 settings.toml 的 `[music]`。
- 歌和封面放在 `~/Documents/FinchGram/music_cache/<来源>/`，来源是其地址与账号的哈希，歌按 id 命名，封面在 `covers/` 下。页面打开和一首歌结束时按 `cache_limit_mb`（默认 2048，0 为不限）裁掉最旧的歌；封面不计入。

## API

Subsonic REST API 1.16.1，JSON（`src/music/subsonic.rs`），服务器有 OpenSubsonic 扩展时从 `getOpenSubsonicExtensions` 读取。每个请求带一个盐和"密码 + 盐"的 MD5，密码本身从不上线（token 认证）；吃不下 token 的服务器（错误 41，LDAP 账号）改为十六进制编码的密码，问过一次之后一直如此。用到的调用：`ping`、`getAlbumList2`、`getAlbum`、`getArtists`、`getArtist`、`getGenres`、`getPlaylists`、`getPlaylist`、`getStarred2`、`search3`（空查询按 OpenSubsonic 的理解逐页列出全部歌曲）、`star`、`unstar`、`scrobble`、`getCoverArt`、`stream`。

## 这一版没有的

歌词（服务器有 `getLyricsBySongId` 时）、全曲库搜索框、编辑歌单、ReplayGain、交叉淡入淡出与均衡器、Mac 的"正在播放"与媒体键、播放语音消息时暂停音乐、特意为离线保留整张专辑、拖动排队列、Telegram 的音频消息与本机文件夹作为来源、Jellyfin 和 Emby 作为来源、网络电台。Broadsheet 和 Terminal 用 Look 的颜色与形状画这一页，和其他共享页面一样，还没有自己的设计轮次。

## 测试

- `cargo test -- music`：API 客户端对着从 demo.navidrome.org 录下的应答（`src/music/fixtures/`），取歌模块对着进程内的 HTTP 服务器（支持与不支持 Range 两种），队列的顺序与循环，表单，缓存裁剪。
- `cargo test live_subsonic -- --ignored` 并设置 `FINCHGRAM_SUBSONIC_URL`、`_USER`、`_PASSWORD`：对真服务器列目录、搜索、取流（demo.navidrome.org 用 demo/demo 能通；Homebrew 的 Navidrome，加 Docker 里的 gonic、LMS、Ampache 挂同一个音乐文件夹，就是兼容性矩阵）。
- `cargo test screenshots -- --ignored`：这一页在三个主题下的各个场景（`music`、`music-album`、`music-artists`、`music-favourites`、`music-add`、`music-add-tested`、`music-error`、`music-none`、`music-collapsed`）和带播放条的聊天窗口（`chats-playing`）。
