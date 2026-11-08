# 直播目录：API 契约

[English](live-directory.md)

状态：维护者的画布「FinchGram Desktop 直播」第 12、13 轮（2026-11-07；claude.ai/artifact/RxoD91XwY8uqgZNNMGcuCB）的契约，地址是维护者同日定的
`https://services.finchgram.com/live/api/v1`。它取代了 2026-11-07 的第一版契约（那一版只是一份靠 pull request 改动的来源清单，不说谁在播）。FinchGram
自己的目录由 [FinchGram/Services](https://github.com/FinchGram/Services) 提供，欢迎 pull request，也欢迎 fork；FinchGram 在 Live
视图里读它（docs/live-streams.md）。机器可读的版本是 [live-directory.openapi.yaml](live-directory.openapi.yaml)（OpenAPI 3.1），与那边的
`src/live/openapi.yaml` 是同一个文件，那边的测试保证两份一致；本页解释它，并说明 FinchGram 怎么用。

## 它是什么

一个会开播的**频道**的目录，频道来自允许收录的**来源**，每个频道带着它的**直播场次**和观察到的时间。它回答搜索。它不转发视频，也不保存任何用户的信息：没有账号、没有关注、没有推流密钥，请求里也没有任何能说明是谁在问的东西。播放地址也不在里面：应用播放时自己去问来源要。

Telegram 频道在不在播，目录说了不算：服务器没有 Telegram 账号可以去问。目录列出频道和它的用户名，状态写 `unknown`，由应用自己通过 TDLib 去问 Telegram，就像 Live 视图对用户自己的聊天做的那样。

谁都可以自己运行一个：fork FinchGram/Services，或者任何回答这份契约的服务器。

## FinchGram 自己的目录：来源

| 来源 | 频道怎么进来 | 直播状态 | 观看人数 | 播放 |
|---|---|---|---|---|
| Owncast | 主人自己把服务器列进 Owncast 目录的 | 目录首页，每 5 分钟；服务器在播时每 4 分钟报一次到 | 没有（每台服务器的 robots.txt 不许爬虫访问 `/api`；用户点开时由应用去问那台服务器） | 在 FinchGram 里（HLS） |
| PeerTube | robots.txt 允许的实例上的公开直播，每天通过 SepiaSearch 找一次实例 | 每个实例，每 5 分钟 | 精确 | 在 FinchGram 里（HLS） |
| Icecast | 服务器在 Xiph 目录（`yp.xml`）里标为公开的电台 | 每 15 分钟；某次导出里没有某个电台，不当作它下播了 | 导出里没有 | 在 FinchGram 里，只有声音 |
| HLS | 播主通过 pull request 提交、经过审核的流 | 每 5 分钟看它们的播放列表 | 没有 | 在 FinchGram 里 |
| Telegram | 通过 pull request 提交、经过审核的公开频道 | 应用去问 Telegram | 应用从 Telegram 拿 | 在 FinchGram 里（docs/live-streams.md） |
| Twitch、YouTube、Kick、Bilibili | 还没接：没有开发者密钥，条款没确认 | — | — | 链接在浏览器里打开 |

主人把频道从它来源的目录里撤掉，这个目录也会跟着撤：Owncast 在下一次同步，Icecast 一天之内，PeerTube 一周之内（下次读它的 robots.txt 时）。

## 四种状态

每个直播场次带 `observed_at`、`expires_at`、`last_confirmed_live_at` 和一个状态：

- `live`：最新的观察说在播，而且还在来源的有效期内（Owncast 5 分钟，PeerTube 和 HLS 10 分钟，Icecast 20 分钟）。
- `stale`：最后一次观察时在播，但那次观察过期不到 60 分钟。应用写「Not confirmed right now」，按钮是「Try opening」。
- `offline`：最近 30 分钟内的一次观察说没在播。
- `unknown`：60 分钟内没有可靠的观察，来源暂停或还没接，或者是目录没法问的来源（Telegram 的频道）。

Live now 先列确认在播的，再列 stale 的；缓存的旧数据从不当成现在的。

## 约定

- **传输。** HTTPS。目录用它的基础地址来认（FinchGram 自己的：`https://services.finchgram.com/live/api/v1`），所有路径都在它下面。GET 和 HEAD，CORS 用 OPTIONS。没有 cookie，不需要认证。
- **格式。** UTF-8 的 JSON。字段名用 snake_case；时间是 UTC 的 RFC 3339。
- **id** 是目录自己的：频道 `fg_ch_…`，场次 `fg_se_…`，由来源和频道在来源处的 id 算出来，所以每次同步、每份目录里都一样。客户端可以用 id 记住一个频道。
- **语言** 是 FinchGram 那样写的 BCP 47 标签（`en`、`zh-Hans`、`pt`）；`language=zh` 包括 `zh-Hans` 和 `zh-Hant`。来源没说语言就是 `null`。
- **不编数据。** 来源没说的字段一律是 `null`；应用这时显示它的兜底文字（「Language not given」「No thumbnail from Icecast」，观看人数写「—」），从不写 0 或估计值。
- **只增不改。** v1 之内只加字段；客户端忽略不认识的字段，不认识的枚举值按该字段的兜底处理。
- **缓存。** `Cache-Control: public, max-age=60`（`/sources` 是 300），加强 `ETag`；带 `If-None-Match` 得到 `304`。`/search` 随 `Accept-Language` 变化。
- **CORS。** `Access-Control-Allow-Origin: *`，ETag 对网页可见。
- **错误** 是状态码加 `{"error": {"code", "message"}}`：`bad_request` 400、`not_found` 404、`method_not_allowed` 405、`rate_limited` 429（带 `Retry-After`）、`internal` 500、`not_implemented` 501。

## 接口

### GET /search

`q`（关键词；不给就是浏览）、`mode`（`live`、`channels`、`upcoming`）、`source`（可重复）、`category`、`language`、`status`（`mode=channels` 时）、`playback`（`native`、`external`）、`content`（`general`，或 `general,mature`）、`sort`、`limit`（1 到 50，默认 24）、`cursor`。

- **Relevance**（有关键词）：名字 > 标签 > 标题 > 类别、来源和语言；匹配程度相同时，确认在播的在前，再按开播时间新的在前。
- **Recommended**（没关键词）：确认在播是硬门槛；然后是应用的语言，应用用 `Accept-Language` 发过来（Settings › Live；请求里别的都不涉及个人）；然后是一小时内开播的。每一层里各来源轮流出，每个来源按自己的顺序（它自己的观看人数，再按名字），所以从不跨平台比较人数，也没有哪个来源能占满一页：光 Xiph 的目录就有几千个电台。
- **Most viewers** 只在同一来源里比，没有人数的排在后面，不当作 0。
- **翻页。** `next_cursor` 在同一个 `snapshot` 里翻；新快照的一页可能重复某一项，应用按场次或频道 id 去掉。`total` 是 `eq`、`gte`（有来源落后时）或 `unknown`；`partial` 和 `missing_sources` 说明哪个来源没回答。
- **Facets** 统计每个筛选的选项时，当作这个筛选没设。

```json
{
  "snapshot": "s_477f57621983",
  "generated_at": "2026-11-08T03:29:03Z",
  "total": { "value": 9239, "relation": "eq" },
  "next_cursor": "eyJzIjoic180NzdmNTc2MjE5ODMiLCJvIjoyNH0",
  "partial": false,
  "missing_sources": [],
  "items": [{
    "channel": {
      "id": "fg_ch_8fe7f6e63c863d0f", "name": "Harbour Cam", "source": "peertube",
      "source_ref": "harbour@tube.example.org", "url": "https://tube.example.org/video-channels/harbour",
      "handle": null, "category": "irl", "language": "en", "classification": "general",
      "tags": [], "added_at": "2026-11-08T03:15:00Z", "last_live_at": "2026-11-08T03:25:00Z"
    },
    "session": {
      "id": "fg_se_1c2d3e4f5a6b7c8d", "title": "Harbour at night",
      "started_at": "2026-11-08T01:00:00Z", "status": "live",
      "observed_at": "2026-11-08T03:25:00Z", "expires_at": "2026-11-08T03:35:00Z",
      "last_confirmed_live_at": "2026-11-08T03:25:00Z",
      "viewers": { "value": 31, "kind": "exact" }, "thumbnail": null
    },
    "playback": { "methods": ["hls"], "availability": "available", "reason": null },
    "upcoming": null,
    "attribution": "PeerTube instances"
  }],
  "facets": { "source": { "icecast": 9082, "owncast": 56, "peertube": 101 }, "category": {}, "language": {}, "approximate": false },
  "sources": [{ "id": "owncast", "status": "ok", "last_successful_sync_at": "2026-11-08T03:25:00Z" }]
}
```

### GET /suggest

`q`，用户打字时用（应用等 200 毫秒，新请求到来就取消旧的）：从标签里来的词、最多五个频道（在播的在前，再是名字以输入开头的），以及匹配的筛选。

### GET /channels/{id}、/channels/{id}/sessions、/channels/{id}/schedule

一个频道（和搜索结果里的一样）、它最近的场次（新的在前）、它预告的场次（来源没有排期就是空的）。

### GET /channels/lookup

用户粘贴的链接，只在目录自己的记录里匹配：`platform` 加 `handle`（`telegram` 加用户名，`peertube` 加 `频道@host`，`owncast` 加 host），或者只给 `host`。服务器不去访问任何地址，应用也不发路径、查询参数或凭证。没有、或者同一个 host 上有好几个时，回答 `{"channel": null}`。

### GET /sources

每个来源：`indexing`（`enabled`、`not_integrated`、`paused`、`restricted_by_terms`、`disabled_by_source`）、它怎么找频道、它的能力（`live_status` 是 true 或 `"client"`、`viewer_counts`、`thumbnails`、`playback`、`auth`）、署名、观察到的东西保留多久，以及最近一次同步（`ok`、`degraded`、`rate_limited`、`failed`、`none`）。

### POST /submissions

这一版没有（第二阶段）：`501`。频道通过 pull request 进来。

## 播放

`methods` 是来源能怎么放（`hls`、`dash`、`icecast`、`telegram`、`official_embed`、`external`）；`availability` 是这一个现在能不能放：`available`、`unconfirmed`（stale 或 unknown）、`sign_in`、`unavailable`，带一个 `reason`。应用唯一的按钮照它们来：Watch here、Listen here（Icecast）、Try opening（未确认）、Open on Twitch（只能用平台自己的播放器）、Sign in to watch、Unavailable。

## 内容

`classification` 是 `general` 或 `mature`（粗口、暴力或赌博）；请求不要就不给 mature，应用也默认隐藏，直到用户在 Settings › Live 里打开。成人直播这一版不收录：来源标了 NSFW 的 Owncast 服务器和 PeerTube 直播一律不进目录，因为这个标记分不出限制级和成人。审核过的频道必须是主人公开的地方；HLS 流只能是播主自己的或得到他们允许的；不能需要密钥、登录或别的应用的请求头才能放。

## FinchGram 怎么读它

- **Discover** 用 `/search` 和 `/suggest`，把用户在 Settings › Live 里选的语言作为 `Accept-Language` 发过去。账号的任何信息都不发给目录：普通的 GET，没有 cookie，`User-Agent: FinchGram/<version>`。
- **Watch。** 应用播放时从来源本身拿播放地址：Owncast 服务器的 `/hls/stream.m3u8`（以及它的 `/api/status`，用来看人数和在不在播）、PeerTube 视频的播放列表（`/api/v1/videos/{uuid}`）、Icecast 电台的流、HLS 播放列表；Telegram 的直播照 Live 视图那样放（TDLib）。
- **目录里的 Telegram 频道** 在进入视野时按用户名通过 TDLib 解析，一次几个（短时间内解析很多正是 Telegram 限制的，FLOOD_WAIT），然后照 Live 视图的方式看它的直播故事和直播。
- **关注** 只存在 Mac 上，从不发出去：应用自己去问目录或来源，关注的那些频道现在怎样。
- **守的规矩。** 不认识的字段忽略，不认识的来源跳过，超过 16 MB 的回答拒收。

## FinchGram 自己的目录

```
src/live/openapi.yaml          这份契约（和本仓库的是同一个文件）
src/live/sources.ts            每个来源的配置；src/live/sync/ 是各自的读取
src/live/cron.ts               同步，每 5 分钟一次（Worker 的定时触发）
src/live/snapshot.ts           一个来源的频道、观察和场次，每次同步合并一次
src/live/search.ts             搜索、联想、排序（纯函数）
src/routes/live/api/v1/        路由；契约本身在 /live/api/v1/openapi.yaml
data/live/channels/<id>.json   审核过的 Telegram 频道和 HLS 流
```

每个来源的快照在 D1 里是一个值（JSON、gzip、base64），所以不管来源多大，一次同步只写一次。审核过的频道在每个 pull request 上和每次构建前检查；那个仓库的 CONTRIBUTING.md 说明怎么加。现在一个都还没有：放什么由维护者决定。

## 待决定

1. 要不要写信给 Xiph（icecast@xiph.org），说明我们每 15 分钟读一次 `yp.xml`：它没写任何条款，而且这个文件不压缩有 8 MB。
2. Twitch、YouTube、Kick：先要开发者密钥，并逐个确认条款，才能收录。
3. 要不要收录成人直播、用什么年龄验证、哪些地区要求验证（第 12 轮的待决问题）。在此之前，来源标了 NSFW 的频道一律不收录。
4. 提交（第二阶段）。
