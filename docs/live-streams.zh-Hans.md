# 直播：查到的东西，和第一版

[English](live-streams.md)

状态：2026-11-06 调研；2026-11-07 做了第一版（见下文），Telegram 自己的直播，只限 RTMP 直播。2026-11-06 维护者提出这个题目（“咱们来仔细研究一下live stream”），想法分三部分：内容本身，也许要找一些
KOL；点对点的视频，中间只要一个服务器让双方发现对方；源的带宽不够或延迟太高时做一个 relay service。同一天又问哪些直播源立刻
能用，好把第一版做出来而不是一直讨论（“有哪些直播源，立刻可以用的，咱们做第一版吧”）。KOL 的事按他们的话先放下；这一页是技术上
查到的东西，以 2026-11-06 为准（价格和版本都是当天读的），供第一版的决定用，决定 2026-11-07 做了（见下文“第一版，已定”）。直播源的条款 2026-11-06 和 07 两天核过了。

## 做第一版之前

- 百宝箱里有一张灰色的 **Live streams** 卡（`src/toolbox.rs`，id `live`），来自设计第 7 轮，说明一行是 "Live streams
  going on in your groups and channels"：设计里指的是 Telegram 自己的直播，不是 FinchGram 自己的一张网。
- 锁定的 TDLib 1.8.67 带着全部直播接口（下一节）。应用和 `finchgram-tdlib` 之间是 JSON，stdin 和 stdout 上一行一个对象；
  TDLib 的 `bytes` 用 base64，一秒视频（几百 KB）就是一行长一点，仅此而已。
- 播放器是 libmpv，FFmpeg 编译时去掉了网络协议（mpv-0.41.0-2 起为 Live 视图加上了）、编码器和封装器（`vendor/mpv/build.sh`）。mpv 自己拉不了 HLS 或 RTMP 地址；
  由应用拉取，再通过 `finchgram://` 流（`src/player/stream.rs`）把字节喂给它。这个流已经能喂长度未知的文件（`size` 回
  `MPV_ERROR_UNSUPPORTED`），也不必支持 seek。音乐（`src/music/fetch.rs`）是现成的样板：ureq 拉，缓存文件边长，mpv 边读。
- 应用没有 WebRTC，故事只认 `messageStory`，群通话一点没有。

## 一对多：中继是正道，不是沟边

想法的第二和第三部分，对通话成立，对直播不成立。

- **点对点加一个让双方发现对方的服务器**，就是 WebRTC：信令服务器传 offer 和候选地址，STUN 告诉每一方自己的公网地址，直连
  打不通时媒体走 TURN 中继。这个中继从来不是可选项：实测需要 TURN 的连接占 22%（callstats.io，2016），UDP 12% 加 TCP 5%
  （fippo，2017），一端是有公网地址的服务器时约 4%；libp2p 2025 年对 85,000 个网络里 440 万次打洞的研究，成功率 70% ± 7%。
- **直播是一对多**，同一路流主播要按观众数上传那么多遍。按每个观众 2.5 Mbps、上行可用 60% 算（Ookla 2026 年 9 月中位数）：

  | 上行 | 网状直连能带的观众 |
  |---|---|
  | 美国有线宽带，上行 10–35 Mbps | 2–8 人 |
  | 全球固网中位数 68 Mbps | 约 15 人 |
  | 手机 14–34 Mbps | 3–4 人 |

  每多一个对端，发送方还要多一份拥塞控制、重传队列和 SRTP。网状视频到 5 人还行，过 15 人没救。
- **中继给不了源更多上行。** 源的上行低于编码码率时，只有编码器能帮忙，WebRTC 的拥塞控制（GCC 加全链路反馈）会自动降码率，
  大家收到的就是降了的那一路。中继能做的是另外三件事：源只上传一次；源发两三层 simulcast（RFC 8853）或一路 SVC（VP9、AV1），
  中继按每个观众自己的下行给层，慢的观众永远拖累不了源；离源近的中继把烂的第一跳缩短。两个家庭 ISP 之间的路径常比家到机房再到家
  更差：libp2p 的数据里 10% 打通的直连往返比走中继还高，Riot 测过旧金山到波特兰的游戏路径绕洛杉矶、丹佛、西雅图要 70 ms，直连
  14 ms。Zoom、Meet、Teams 每一路通话都走自己的服务器。业内把这种中继叫 SFU（selective forwarding unit），按地区级联起来
  就是托管服务卖的东西。
- **观众之间互相转发**（peer-assisted，“P2P CDN”）已经退出消费市场：Streamroot 的 mesh delivery 2023-09-30 关停，Peer5
  2021 年成了微软的 eCDN，只做企业内网的全员大会，Hive 和 Kollective 卖给企业，Bilibili 在浏览器里基于 HLS 分块跑自己的一套。
  它需要分段的 HLS（10–30 秒延迟；那些插件不支持 LL-HLS）、同一路流有很多好网络上的观众，对散落在各家 ISP 的 20 到 200 个观众
  毫无帮助；估计同一路流不到 500 人同时看时不值得它的复杂度。

各条路大致的代价：

| 做法 | 延迟 | 谁付带宽 | 观众 | 什么时候不行 |
|---|---|---|---|---|
| 网状直连 | < 0.5 s | 主播 | 5–15 | 上行打满、最烂的 ISP 路径说了算、反正还得有 TURN |
| 一个 SFU 中继 | 0.5–1 s | 我们 | 一台 VPS 每千兆约 300 人；托管几千 | 只有一个地区；源的上行仍是下限 |
| 级联 SFU | < 1 s | 我们 | 10⁴ 以上 | 复杂；实际都是买的（LiveKit Cloud、Cloudflare） |
| 观众互转 + SFU 做种 | 10–30 s | 观众和我们 | 多、且集中 | 人少、手机对端、LL-HLS |
| CDN 上的 LL-HLS | 2–5 s | 我们，按 GB 或分钟 | 不限 | 要亚秒互动时 |
| CDN 上的 HLS | 15–30 s | 我们 | 不限 | 延迟 |
| MoQ 中继 | 号称 < 1 s | 我们或 Cloudflare | 多 | 草案协议、线格式在改、没有第三方播放器 |

2026-11-06 读到的价格，按 2.5 Mbps 每观众小时（1.125 GB）算：Cloudflare Realtime 的 SFU 出口每 GB 0.05 美元，即 0.056，
每月 1 TB 免费；Cloudflare Stream 每送达 1,000 分钟 1 美元，即 0.06；LiveKit Cloud 0.14–0.17；Amazon IVS 实时 0.072；
Agora 广播 0.12；Daily 和 Twilio 约 0.24。自己架：Hetzner 的云主机每月含 20 TB（约 17,800 观众小时），之后每 TB 1 欧元；
上限在网卡，每千兆约 300–400 个观众。软件：LiveKit（Go，Apache-2.0）、MediaMTX（Go，MIT，单个二进制，WHIP/WHEP/SRT/RTMP/
HLS）、SRS（C++，MIT）、mediasoup、Janus、OvenMediaEngine；TURN 用 coturn。

协议：WHIP（推流）2025 年 3 月起是 RFC 9725；WHEP（播放）还是草案（draft-ietf-wish-whep-05，2026 年 10 月）。Media over
QUIC 是 draft-ietf-moq-transport-23，不是 RFC；Cloudflare 在 330 多个城市跑着中继（免费测试，计划每 GB 0.05 美元），Meta
把 Instagram Live 的推流搬了上去（2026 年 10 月）；Rust 的实现（moq-dev/moq：moq-net、hang、moq-relay）全是 tokio，自己
说 API 和协议在版本之间可能变。对一个必须一直能用的应用，2026 年不是能押的东西。

## Telegram 已经把这层跑着了

| 产品 | 谁能开 | 观众 | 起始 |
|---|---|---|---|
| 群组视频聊天 | 有 `can_manage_video_chats` 的管理员 | 听众不限，带视频至多 1,000 | 2021 |
| 频道直播 | 同一权限 | 不限 | 2021-08-31 |
| RTMP 推流（“Stream With…”） | 同上，`createVideoChat(is_rtmp_stream)` | 不限 | 2022-03-11 |
| 会议通话（链接） | 任何用户，`createGroupCall` | 200 人，端到端加密 | 2025-04-30 |
| **Live Stories** | **每个用户**；频道和超级群要靠 boost 解锁故事 | 未公布 | **2025-11-19** |

**Live Stories**（telegram.org/blog/live-stories-gift-auctions）：“Every Telegram user can now host live streams in
Telegram Stories”；隐私设置和故事一样（所有人、联系人、密友、指定的人）；“streams in stories support RTMP keys for
external apps like OBS and XSplit”；观众的评论可以用 Stars 付费，按档位置顶更久、颜色不同（TDLib 的 `groupCallMessageLevel`：
比如 2,000 Stars 置顶 1,800 秒、280 字，10,000 Stars 置顶 3,600 秒、400 字；主播设每条评论的最低价，上限 35,000；没有文字
的付费评论就是打赏）；主播看得到总额，还有打赏榜。每个账号同时只能有一条 live story；它出现在关注者的故事环里，也在
`t.me/<用户名>/s/live`；结束后由录像替代，成为一条视频故事。TDLib 从 1.8.57（2025-11-26）起带它；1.8.67 的签名和 master
一致。没公布的：观众上限、主播拿到的 Stars 比例（付费消息按 Stars 条款收件人拿 85%）、应用版本。

只走 TDLib 的客户端能做什么，从 tdesktop、Telegram iOS、tgcalls、Unigram 和 Telegram Web K 的代码里看：

- **看 RTMP 模式的直播**（频道的 RTMP 直播、RTMP 的 live story），**完全不用 WebRTC**。用 `joinVideoChat` 或
  `joinLiveStory` 加入，静音、不带视频，给一个占位的负载（tdesktop 发一个随机 `ssrc`，`ufrag`、`pwd`、`fingerprints` 都空；
  回答里是 `"stream": true, "rtmp": true`）；`getGroupCallStreams` 给出通道和流当前结束的时间；然后每秒一次
  `getGroupCallStreamSegment`，从 `floor(end / 1000) × 1000 − 2000` 毫秒起，RTMP 的 `channel_id` 是 1（“unified”，声音和
  画面在一段里），质量 Full。`TIME_TOO_BIG` 是等 100 ms 再问；`GROUPCALL_JOIN_MISSING` 是重新加入。一段是一个小头部（int32
  签名 `0xa12e810d`，TL 字符串的容器名 "mp4"，int32 active mask，int32 事件数，然后每个事件是 int32 offset、TL 字符串
  endpoint、int32 rotation、int32 extra），后面跟着**一秒的完整 MP4**，Opus 的声音，画面各方面看是 H.264（RTMP 入口收的是
  H.264；没有文档明确写）。纯音频的段是 48 kHz 的 Ogg Opus，带 `TG_META`、`ACTIVE_MASK`、`ENDPOINTS` 注释。Telegram Web K
  用纯 JavaScript 播它们（`src/lib/calls/rtmpCallsController.ts`、`videoStreamInfo.ts`、`serviceWorker/rtmp.ts`），把每个
  MP4 重封装成分片给浏览器；tdesktop 和 tgcalls 用 C++ 做同样的事（`calls_group_call.cpp` 的 `LoadPartTask`、
  `broadcastPartStart`；`StreamingMediaContext.cpp`：段长 1,000 ms，缓冲 2,000 ms；`VideoStreamingPart.cpp`）。所以应用的
  libmpv 够用：它解 H.264 和 Opus，也解复用 MP4；缺的只是一次重封装，因为一串完整的 MP4 文件不是 mpv 能当作一路流读的
  （见下）。
- **Stream 模式由服务器决定。**Telegram 的 API 文档（core.telegram.org/api/group-calls，“Stream mode”，2026-11-07 读）说，
  视频聊天、频道直播或 live story 在两种情况下以可下载的分块送出：创建者选的 RTMP 模式；以及自动扩容，“once the audience
  grows past around 30 participants for channel livestreams, 1000 participants for supergroup/basic group video chats,
  or 100 participants when connected to a test DC”，门槛由服务器控制、可能会变；用户个人的 live story 没有给出数字。加入的
  回答说明是哪一种：RTMP 是 `{"stream": true, "rtmp": true}`，自动扩容是 `{"stream": true}`，两者都没有就是 WebRTC。
  自动扩容模式的分块和 RTMP 的统一通道不同：混音是一个 Ogg 块，它的 `ENDPOINTS` 和 `ACTIVE_MASK` 元数据列出视频通道，每个发布者
  的视频各自是一块，时钟来自客户端同步过的服务器时间，而不是 `getGroupCallStreams`。第一版应先做 RTMP 模式（每秒一个 MP4），
  再做自动扩容模式（音频和视频要对齐）。
- **看不了**停留在 WebRTC 上的 live story 或视频聊天：观众还少的手机摄像头直播，以及所有会议通话（它们没有 stream 模式）。这些
  需要官方客户端放在 tgcalls 里、建在 libwebrtc 上的那一套；没有 Rust 客户端有它，找到的唯一不用 libwebrtc 的实现是一个只发
  不收的 Go 机器人。这样的流会列出来，标“在手机上看”。
- **用 RTMP 开播。** `getVideoChatRtmpUrl` 和 `getLiveStoryRtmpUrl` 给出形如 `rtmps://dc4-1.rtmp.t.me/s/` 的地址和一个
  密钥；OBS、ffmpeg（`-f flv -c copy`，H.264 加 AAC）、go2rtc、MediaMTX 都能推。一路流只有一个推流端；Telegram 唯一公布的建议
  是软件编码（OBS 里的 x264）；没有文档写码率上限。API 里的屏幕共享和录制对 RTMP 流和 live story 不可用（主播在 OBS 里录）。
- **以参与者身份直播 Mac 的摄像头或屏幕**需要一个 WebRTC 栈：`joinVideoChat` 只传信令（发出去 `ssrc`、`ufrag`、`pwd`、
  `fingerprints`、`ssrc-groups`；回来 transport 候选和 payload 类型）。
- **延迟：**客户端落后直播边缘 2 秒，再多缓冲一段，客户端侧约 3 秒，加上服务端切段；有用户实测 OBS 到观众约 12 秒（OBS 论坛，
  2025-08）。Telegram 没有公布数字。

这是 Telegram 自己的功能，用法和官方客户端一样，在这个项目对 API 条款的理解之内：观看就是以听众身份加入通话，观众本来就是听众。

## 第一版能用的直播源

一个源算能用，是指一个签名并公证、出自美国公司的应用能播它，不用 WebView，不带内置密钥，也不碰条款禁止第三方播放器的接口。
2026-11-06 和 07 两天对照条款页面、仓库和对流本身的探测核过。

| 源 | 应用怎么拿 | 判断 | 备注 |
|---|---|---|---|
| Telegram 的直播和 Live Stories | TDLib，见上 | 能用 | 只有 FinchGram 这类客户端才做得到的源；只限 stream 模式的通话 |
| 用户自带的 HLS 地址或 M3U 列表；Xtream 或 Stalker 的账号 | 用户粘贴 | 能用，用户自带 | 所有 IPTV 播放器的做法（TiviMate、IPTVnator、VLC：自己不带频道，附一句免责）；应用什么都不自带。Xtream 是付费 IPTV 订阅的 API，多数是盗版：只收用户输入的账号，绝不带列表 |
| Owncast 服务器和 Owncast 目录 | 每台服务器的 `/hls/stream.m3u8`、`/api/status`、WebSocket 上的聊天；目录的 `api/iptv` M3U | 能用 | 自托管，MIT；2026-11-07 某一时刻目录里 75 台在播、147 台离线；落后 5–18 秒；状态和聊天接口标为内部、可能会变 |
| PeerTube 直播、SepiaSearch | `GET /api/v1/videos?isLive=true`，HLS 是静态文件；sepiasearch.org 的跨实例 API | 能用 | 联邦式；某一时刻 129 路在播；按设计落后 30–40 秒；P2P 是网页播放器的事，原生客户端只管拉 |
| Nostr 直播活动（NIP-53）、zap.stream | 向中继订阅一次 kind 30311 事件，里面带 HLS 地址 | 协议能用；各家主机的条款各自算 | 四个中继上 34 路标为在播，其中 12 路是新鲜的 |
| iptv-org | API 的 `channels.json`、`feeds`、`logos`、`guides`（Unlicense），以及 `streams.json`：18,602 条用户提交的链接 | 元数据开放；链接是灰的：运行时按用户的选择去拉，绝不打包 | github.com/iptv-org/iptv，143,000 星；README：链接“据我们所知是版权方自己有意公开的”，收到主张就删；黑名单 836 条，GitHub 上 19 次 DMCA 通知，2020 年因一次通知整个仓库下线五周；6.6% 有地区封锁，6.8% 需要请求头 |
| Free-TV/IPTV | GitHub 上的一份 M3U：1,884 个在播频道，其中 124 个是 YouTube，213 个有地区封锁 | 灰；要用也是运行时拉 | 21,800 星，没有许可证文件，地址取自 iptv-org；Linux Mint 的 Hypnotix 从 2021-04-15 起拉它（“Hypnotix does not provide content or TV channels”）；没找到针对 Mint 的投诉 |
| 公共电视台自己的 HLS | 页面里的 m3u8 | 灰到关 | DW、NHK World、Arirang、TRT World、ABC News Australia、tagesschau24 能连上，条款限个人使用、对播放器没说；Bloomberg、Sky News、CGTN、半岛、CNA、Euronews 能连上，但条款明文禁止工具和第三方应用；France 24、ESA、NASA+、公视、Rai、ZDF、BBC 只有 YouTube、登录或地区围栏；NASA TV 的 HLS 2024 年就没了 |
| 央视和中文“直播源”列表（imDazui/Tvlist-awesome-m3u-m3u8、fanmingming/live、Guovin/iptv-api、YanG-1989/m3u、肥羊） | 指向运营商 IPTV 端点和私人中转的 M3U 列表 | 不能自带；用户自己带的列表是用户自己的事 | 央视网的声明禁止链接；对电视猫、PPTV 的诉讼；2015 年四部门 229 号文把营利性链接服务定为犯罪；电视家 2023-11 关停；列表自己的 README 就写“仅供测试研究使用”；在中国之外多半连不上 |
| FAST 服务（Pluto TV、Xumo、Samsung TV Plus；Roku、Tubi、Rakuten、Plex） | 流传的列表 | 关 | Pluto 的条款禁止它自己之外的任何界面，Xumo 一样；MPA 和华纳的通知把列表打掉了（i.mjh.nz，2024）；Pluto 2026 年起要会话令牌 |
| Twitch、Kick、Rumble、Odysee | 非官方接口 | 灰 | 条款禁止工具（Twitch、Kick、Rumble）；Twitch 还有完整性令牌和拼进流里的广告；Odysee 的 API 开放但没有文档 |
| YouTube Live、Bilibili、斗鱼、虎牙、抖音、Instagram、TikTok、Facebook | 没有，或带签名的接口 | 关 | YouTube 的开发者政策；Bilibili 13 个月里发了三封律师函（PiliPala 2025-07、bilibili-API-collect 2026-01、BiliRoaming 2026-07）；抖音诉小葫芦胜诉 100 万元；Trovo 和 DLive 2026 年关停 |
| Jellyfin 和 Emby 的 Live TV | 它们的 API，用户自己的服务器 | 以后 | 跟那两个工具一起 |

这些流长什么样，在 iptv-org 的 550 路上实测（随机 400 路、中文 150 路）：95% 是 MPEG-TS 分段，2% 是 fMP4，15% 有
AES-128（几乎全是 FAST，中文的一路都没有），287 路里 2 路是 LL-HLS，DASH 占 1.2%（多是 BBC，反正有地区围栏）；分段中位数 5 秒
（第十百分位 2 秒，第九十百分位 10 秒），所以落后直播 10–40 秒；15% 的主播放列表重定向到带令牌的地址；35% 有
`EXT-X-DISCONTINUITY`；有几个“播放列表”其实是裸 TS 或 gzip；地区封锁是 Akamai 的 403 页面。DRM（SAMPLE-AES、FairPlay、
Widevine）从结构上就做不了：应用认出密钥格式，直说。

那么播放器要做的：主播放列表和媒体播放列表；RFC 8216 §6.3.4 的直播刷新（按目标时长，没变化时减半，超过一倍半算卡住）；
相对地址按重定向后的最终 URL 解析；看一眼正文（`#EXTM3U`、`0x47`、gzip）再信扩展名；TS 和 fMP4（init 段，map 变了或遇到
discontinuity 时重发）；AES-128，密钥用同一个客户端拉；discontinuity 时重置；403 和 404 分开；不认识的标签忽略。以后：byte
range、DASH、自适应码率、备用音轨和字幕。请求头：iptv-org 的流里 5.8% 要 `User-Agent`，1.8% 要 `Referer`；填一个用户从没打开过
的页面做 Referer 就是绕防盗链，是把播放器变成规避工具的那一件事，所以应用发自己如实的 User-Agent，每路流的请求头只收用户自己
填的字段，绝不来自自带的列表。（画布第 12 轮后来改了主意：从 mpv-0.41.0-2 起，自建的 FFmpeg 带网络，只给 Live 视图的播放器用，见 docs/architecture.md 的媒体播放一节。下面是当时的打算。）FFmpeg 自己的 HLS 解复用器要 FFmpeg 自己的网络（当时 vendored 构建里是 `--disable-network`），所以
客户端是应用的：`m3u8-rs` 6.0.1（2026-07）解析播放列表，ureq 像音乐那样在线程上拉；Streamlink 是应用侧 HLS 客户端往 mpv 里
灌的最近先例。

## 轮子：Rust 里的和 macOS 上的

| 需要 | 现有的（2026-11-06） | 合不合适 |
|---|---|---|
| WebRTC，如果哪天要 | `str0m` 0.25（sans-IO，完全同步，Lookback 生产在用；BWE、TWCC、simulcast；没有 TURN 客户端）· `webrtc` 0.21（tokio，发布候选的成熟度，2026-09 才有带宽估计）· LiveKit 的 `libwebrtc`（编译时下载 264 MB 预编译包，源码是一个会动的分支） | str0m 合线程模型；LiveKit 的预编译包过不了“一切从锁定源码构建”这条规矩 |
| MoQ | moq-dev/moq（`moq-net`、`hang`、`moq-relay`、带 ScreenCaptureKit 采集和 VideoToolbox 的 `moq-cli`） | 全是 tokio，线格式在动 |
| HLS | `m3u8-rs`、`hls-runtime` | 这里解析，ureq 拉取 |
| MPEG-TS、MP4 | `mpeg2ts` 0.6（能读能写）、`mp4-atom` 0.17（fMP4 的 box，moq 在用） | 够重封装 Telegram 的分段 |
| RTMP 推流，如果哪天要 | `rml_rtmp` 0.8（2023 年起停更）、xiu 的 `rtmp`（tokio，以服务端为主） | 没有维护中且同步的；FLV 封装不大，握手和分块是公开的 |
| 采集和编码，如果哪天要 | `objc2-screen-capture-kit`、`objc2-av-foundation`、`objc2-video-toolbox` 0.3（生成的，完整）· `cidre`（它的 `sc-record` 例子：ScreenCaptureKit 到 VideoToolbox 的 HEVC 加 AudioToolbox 的 AAC）· `opusic-sys` 自带 libopus 1.6.1，从 crate 自己的源码编 | VideoToolbox 吐 AVCC，RTP 要 Annex B |
| mpv | `stream_cb` 能接无尽的流；`low-latency` 配置是 `cache-pause=no`、`demuxer-lavf-o-add=fflags=+nobuffer`、`demuxer-lavf-analyzeduration=0.1`、`video-sync=audio`、`interpolation=no`、`stream-buffer-size=4k`；`--untimed` 有声音时会坏 | 渲染这边原样不动 |

Telegram 分段的重封装：每一秒是一个完整 MP4，FFmpeg 的 mov 解复用器读到第一个文件的末尾就停。应用得把每个 MP4 里的样本取出来
（`mp4-atom`），给 mpv 写成一路连续的流：MPEG-TS（Opus 要它的 registration descriptor 和每包的控制头；H.264 从 AVCC 转
Annex B），或者 Matroska（更简单：长度未知的 cluster，两种编码原样放）。另一条路是在 `vendor/mpv/build.sh` 里打开 FFmpeg 的
`mpegts` 封装器、从应用里直接调 libavformat，但那会在今天只调 libmpv 的地方加一个对 FFmpeg 的直接依赖；几百行 Rust 看起来更划算。
做的时候定。

## 第一版，已做

2026-11-07 定下（“要不第一版把telegram自带的live stories搞出来吧 … 固定在左侧”；“目前左侧有三个：chat，channel，bot，
对吧，加一个live stories”），当天做完，三个主题都有。

- **放在哪：**左侧聊天、频道、机器人旁边的第四个视图：Workbench 的活动栏（有人在播时一个红点），Broadsheet 那一排视图按钮，
  Terminal 的 `LIVE` 标签和 `/live`。百宝箱里的直播卡片打开它（不能钉，它在左侧已经有位置）。列表和面板在 `ui/pages/live.slint`
  里画一次，用 Look 的配色；状态是 `Live` 全局（`ui/live.slint`）。
- **谁在直播**（`src/telegram/live.rs`）：故事列表里的直播动态，账号就绪后加载一次（loadActiveStories 直到 404，之后是
  updateChatActiveStories；getStory 给出直播动态的群通话），以及聊天的视频聊天和直播（updateNewChat 和 updateChatVideoChat 里聊天的
  video_chat，再 getGroupCall；只列正在进行的）。聊天列表给这样的聊天打 LIVE 标记（`ChatRow.live`），正在直播的聊天在标题下面
  有一条横栏，可以在视图里打开。
- **观看：**只限 RTMP 直播，也就是 OBS 之类推的流。以观众身份加入（joinLiveStory 或 joinVideoChat），静音、不带视频，负载和
  Telegram Web K 发的一样（`{"fingerprints":[],"pwd":"","ssrc":<随机>,"ssrc-groups":[],"ufrag":""}`）；回答里没有
  `"stream": true` 的立刻离开，标为在手机上看。getGroupCallStreams 给出边缘（RTMP 直播的统一通道是 1）；从边缘往回 2 秒开始，
  按随时钟前移的边缘，每一秒一出现就用 getGroupCallStreamSegment 去取（TIME_TOO_BIG：250 ms 后再取；TIME_TOO_SMALL：重新取
  边缘；GROUPCALL_JOIN_MISSING：重新加入一次；遵守 FLOOD_WAIT；其他失败连续八次就放弃）。每一秒先读头部（`segment_file`），
  MP4 重封装成 Matroska（`src/player/remux.rs`：一条 H.264 或 HEVC 和一条 Opus 或 AAC 的样本，按编辑列表的起点，一秒一个
  cluster，画面从关键帧开始），推进内存里的管道（`src/player/pipe.rs`），由直播自己的 mpv 播放（`src/player/live.rs`）。分辨率
  或编码变了，或者 mpv 落后超过约 48 MB，就带着新的头部从头开始。在这台机器上用 vendored libmpv 验证过：ffmpeg 做的三秒、
  一秒一秒喂进去，两条轨都在、一帧不丢地播完（`src/player/pipe.rs` 的测试）；真正的直播在发版后用 OBS 推一条直播动态来验证。
- **不播放的：**非 RTMP 的直播（手机摄像头）灰显列出，标“在手机上看”，不加入。观众多了以后（频道约 30 人、群约 1,000 人）
  Telegram 会把这样的直播切成 stream 模式，但那时声音和每一路画面分开送；留给以后的版本。
- **评论：**群通话的消息（updateNewGroupCallMessage，保留最近 100 条），按 updateGroupCallMessageLevels 上色；免费评论用
  sendGroupCallMessage 发。付费评论和打赏会显示；发送暂时交给 Telegram 自己的应用（星形按钮是灰的），评论要花 Stars 的直播在
  输入框的位置说明这一点。
- **主播：**用户自己的直播动态，或他管理的直播（can_be_managed），以主播身份打开：观看人数、目前的 Stars 和前三名支持者
  （getLiveStoryTopDonors、updateLiveStoryTopDonors）、评论开关（toggleGroupCallAreMessagesAllowed）、悬停删评论
  （deleteGroupCallMessages），以及问过之后的“结束直播动态”或“结束直播”（endGroupCall）。
- **开播：**用户自己的直播动态（startLiveStory，chat_id 用 Saved Messages 的聊天 id，is_rtmp_stream，谁能看：所有人、联系人或
  密友；评论开关和每条评论的 Stars），或他拥有或管理视频聊天的频道（createVideoChat，带 is_rtmp_stream 和标题）。表单一打开就
  显示 RTMP 服务器和密钥（getLiveStoryRtmpUrl 或 getVideoChatRtmpUrl），带复制、显示和换新密钥（replace 那两个调用）；开始后
  以主播身份打开新的直播。已经有一条直播动态在播时打开那一条；直播动态开不了的原因（Premium、boost、次数限制）在表单里说明。
- **链接：**在列表的输入框里粘贴 t.me/<用户名>/s/live，打开那条直播动态（getInternalLinkType、searchPublicChat、
  getChatActiveStories）。
- **打开的聊天：**视图开着时盖住打开的聊天，所以那里新到的消息在视图关上之前不标为已看（conversation.rs）；关上视图就离开直播
  （用户自己的直播照常继续），不让没在看的人被算成观众。

## 第二版：一个看各家直播的客户端

2026-11-07 维护者的话（「我其实希望你实现这个 … 既然这有telegram自带的 也有我们提供的 干脆做成这样，不好吗」）：照维护者的画布 **FinchGram Desktop 直播** 第 10 到 12 轮来做，加上第 13 轮（Telegram 作为一个来源），claude.ai/artifact/RxoD91XwY8uqgZNNMGcuCB。2026-11-08 起在三套主题里做，全部做完后一次发版。

- **视图** 仍是窗口左边第四个：Live 栏里是 Discover、Following、Broadcast、Connections，下面是关注的频道（在播的在前，其余在后）；Watch 是盖在它们上面的一个状态。和窗口其他部分一样要登录 Telegram：画布原本写不登录也能用，维护者否决了（「必须要求登录 … 你都没登录，你怎么进去telegram得节目」）。百宝箱里的直播卡片去掉了；在百宝箱搜 Live 会给一行，点了直接过去；菜单里有 Live，快捷键 ⇧⌘L。
- **目录**（services.finchgram.com/live/api/v1，见 [live-directory.zh-Hans.md](live-directory.zh-Hans.md)）在后台线程里读（`src/live/directory.rs`），带 ETag：Discover 的搜索（模式、带数量的筛选、排序、按游标翻页）、打字时的联想（最后一次按键 200 毫秒后）、各来源的状态、按 id 取关注的频道、粘贴链接的 host 或用户名。不发任何关于用户的东西：普通的 GET，`User-Agent: FinchGram/<version>`，以及用户在 Settings › Live 里选的语言作为 Accept-Language（没选就不发），Recommended 把这些语言排在前面。
- **卡片**（`src/live/channel.rs`，`LiveChannel` 结构）：四种状态、唯一的按钮和它的理由（Watch here、Listen here、Try opening、Sign in to watch、Open on <来源>、Unavailable），观看人数只用来源自己给的，没有就不写，从不估计。缩略图从各来源取（Owncast 的从 owncast.directory，PeerTube 的从各实例），由单独的线程存进 ~/Documents/FinchGram/live_thumbnails，保留一天；取图时这些服务器会看到用户的 IP，和网上任何图片一样。
- **Watch**（`src/live/watch.rs`）：播放时应用自己向频道的来源要播放地址，从不问目录：Owncast 服务器的 /api/status 和 /hls/stream.m3u8，PeerTube 频道当前的直播（video-channels/<名字>/videos?isLive，再取那个视频的播放列表），Icecast 电台的流，提交的播放列表。直播播放器按地址播，走自建 FFmpeg 的网络（mpv-0.41.0-2），只有 Live 视图的播放器能用到；证书按 Keychain 信任的根证书校验（`src/live/roots.rs` 把它们写给 mbedTLS）。暂停时保留期间收到的内容（mpv 的缓存），「Go live」在直播最新处重新打开；清晰度菜单列出这路流的各档。目录里的 Telegram 频道按用户名通过 TDLib 打开，播放方式和用户自己的直播一样。Owncast 的聊天在画面旁边能读也能发（`src/live/chat.rs`）：FinchGram 以匿名聊天用户加入，用 Owncast 给的名字，从不用 Telegram 的名字，什么也不保存。
- **离开时继续播**：离开 Watch 时直播在小窗里继续，聊天视图上也有；弹出后放进一个始终置顶的独立窗口（`src/live/popout.rs`，有自己的播放器，因为这个窗口有自己的属性）。Telegram 的直播和以前一样，离开视图就停。
- **关注**（`src/live/follows.rs`）：在这里关注的频道存在数据库里（`live_follows`，第 9 版布局），按 id 去问目录；用户在 Telegram 上关注的直播也算，标「Followed on Telegram」，要去 Telegram 取消。
- **目录里的 Telegram 频道** 返回的状态是「未知」（目录没有账号可问）：屏幕上看得到的那些由应用去问 Telegram（`src/telegram/live_check.rs`），两秒一个用户名，Telegram 限流时按它要求的时间等；结果只留在本机。
- **Connections › Open a link**：先按样子认链接（推流密钥或 token 一律不收；t.me 走 TDLib；Twitch、YouTube、Kick、Bilibili 在浏览器打开；.m3u8、.mpd、rtsp:// 直接播），否则只用 host 在目录里查，再不行就问那台服务器是什么（Owncast 的 /api/config、PeerTube 的视频、Icecast 的状态页）。
- **这一版没有**（画布上画了的地方灰着）：Twitch、YouTube、Kick、Bilibili（还没有开发者密钥，条款没确认）、保存的私有服务器和 Submit a stream、FinchGram 自己采集和其他推流目的地、成人直播（不收录，维护者还没决定）。

## 设计

2026-11-07 按维护者的话画的（“你自己直接画一个artifacts，把url记录下来就可以了”），Workbench 浅色，五块 1280×800 的板，在
Design 画布 **FinchGram Live** 上，https://claude.ai/artifact/A9SvXayxBSKmxCgeJKnqz4（只有维护者的账号能打开；各板的源文件在它的
`project/` 下）：

1. Live · 看一场 live story：选中的第四个入口带着点，四路直播的列表（一路灰显、只能在手机上看），观看页和评论栏。
2. Live · 没人在播：空列表带链接输入框，主窗格的空状态。
3. Live · 开播：表单、RTMP 服务器和密钥、等 OBS 的预览。
4. Live · 自己在播：主播视角，观众数、Stars、打赏榜、一条正在删的垃圾评论、结束直播。
5. 聊天页 · 频道正在直播：Live 入口上的点、聊天行上的 LIVE 标、频道标题下面带“观看”按钮的直播条。

画布上有一张便签列着第一版的边界。Broadsheet 和 Terminal 没在画布上画：它们是同一个页面换成各自的样子（ui/pages/live.slint），
截图测试把每一个都画出来，浅色和深色（`cargo test screenshots -- --ignored`，live-* 那些图）。

## 待定

1. 观众多时 Telegram 切成 stream 模式的直播要不要也播（声音和每个发布者的画面分开送，要自己对齐）。
2. 付费评论和打赏要不要从 FinchGram 发（花账号里已有的 Stars；档位来自 TDLib），每次发之前怎么问。
3. 目录第一批审核收录哪些频道（现在一个都没有），以及要不要写信给 Xiph，说明我们在读它的 Icecast 导出（见 live-directory.zh-Hans.md 的待决定）。
4. 要不要从 Mac 自己的摄像头或屏幕开播，这决定要不要做 RTMP 客户端和 VideoToolbox 那条流水线。

## 为此读过的

Telegram：https://telegram.org/blog/live-stories-gift-auctions ·
https://telegram.org/blog/live-streams-forwarding-next-channel ·
https://telegram.org/blog/downloads-attachments-streaming · https://telegram.org/blog/group-calls-made-easy ·
https://core.telegram.org/api/group-calls · https://core.telegram.org/api/stories ·
https://core.telegram.org/constructor/inputGroupCallStream · https://telegram.org/tos/stars ·
https://github.com/tdlib/td/blob/master/td/generate/scheme/td_api.tl ·
https://github.com/telegramdesktop/tdesktop/blob/dev/Telegram/SourceFiles/calls/group/calls_group_call.cpp ·
https://github.com/TelegramMessenger/tgcalls/tree/master/tgcalls/group ·
https://github.com/morethanwords/tweb/blob/master/src/lib/serviceWorker/rtmp.ts ·
https://github.com/UnigramDev/Unigram/blob/develop/Telegram/Services/Calls/VoipGroupCall.cs。
拓扑与价格：https://www.speedtest.net/global-index · https://webrtchacks.com/?p=2560 ·
https://medium.com/@fippo/what-kind-of-turn-server-is-being-used-d67dbfc2ff5d ·
https://arxiv.org/abs/2510.27500 · https://www.riotgames.com/en/news/fixing-internet-real-time-applications-part-i ·
https://www.rfc-editor.org/rfc/rfc8853 · https://www.rfc-editor.org/rfc/rfc9725 ·
https://datatracker.ietf.org/doc/draft-ietf-wish-whep/ · https://datatracker.ietf.org/doc/draft-ietf-moq-transport/ ·
https://blog.cloudflare.com/moq/ · https://blog.cloudflare.com/moq-relays/ ·
https://engineering.fb.com/2026/10/12/video-engineering/shipping-media-over-quic-at-meta-scale-for-instagram-live-ingest/ ·
https://developers.cloudflare.com/realtime/sfu/pricing/ · https://developers.cloudflare.com/stream/pricing/ ·
https://livekit.com/pricing · https://aws.amazon.com/ivs/pricing/ · https://www.hetzner.com/cloud/regular-performance/ ·
https://assets.lumen.com/is/content/Lumen/us-mesh-delivery-load-balancer-ecdn-eol-notification ·
https://learn.microsoft.com/en-us/ecdn/ · https://github.com/Novage/p2p-media-loader。
Rust 与 macOS：https://github.com/algesten/str0m · https://webrtc.rs/blog/2026/09/19/announcing-webrtc-v0.21.0.html ·
https://docs.rs/crate/webrtc-sys-build/latest/source/src/lib.rs · https://github.com/moq-dev/moq ·
https://docs.rs/hls-runtime · https://crates.io/crates/m3u8-rs · https://crates.io/crates/mpeg2ts ·
https://crates.io/crates/mp4-atom · https://github.com/yury/cidre/blob/main/cidre/examples/sc-record/main.rs ·
https://docs.rs/crate/opusic-sys/latest · https://github.com/mpv-player/mpv/blob/master/include/mpv/stream_cb.h ·
https://github.com/mpv-player/mpv/blob/master/etc/builtin.conf。
来源的条款： https://owncast.online/docs/watching-streams/ · https://owncast.online/docs/directory/ ·
https://raw.githubusercontent.com/Chocobozzz/PeerTube/develop/support/doc/api/openapi.yaml ·
https://github.com/nostr-protocol/nips/blob/master/53.md · https://github.com/iptv-org/iptv#legal ·
https://github.com/iptv-org/api#overview · https://iptv-org.github.io/api/blocklist.json ·
https://github.com/github/dmca/blob/master/2020/09/2020-09-16-dfl.md ·
https://torrentfreak.com/iptv-playlist-portal-survives-dmca-takedown-from-warner-bros-240624/ ·
https://github.com/linuxmint/hypnotix/blob/master/README.md · https://blog.linuxmint.com/?p=4054 ·
https://github.com/Free-TV/IPTV/blob/master/README.md · https://github.com/4gray/iptvnator/blob/master/README.md ·
https://legal.paramount.com/us/en/plutotv/terms-of-use · https://www.xumo.com/playterms ·
https://github.com/matthuisman/i.mjh.nz/issues/127 · https://www.chinalawtranslate.com/?p=4679 ·
https://www.cctv.com/special/guanyunew/gongsijianjie/index.shtml#wzsm · https://www.appinn.com/pilipala-bilibili-legal-stop/ ·
https://m.ithome.com/html/973202.htm · https://www.thepaper.cn/newsDetail_forward_16287000 ·
https://developers.google.com/youtube/terms/developer-policies · https://kick.com/terms-of-service ·
https://streamlink.github.io/cli/plugins/twitch.html · https://datatracker.ietf.org/doc/html/rfc8216 ·
https://github.com/streamlink/streamlink/blob/master/src/streamlink/stream/hls/hls.py。
