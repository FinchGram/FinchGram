# 直播目录：API 契约

[English](live-directory.md)

状态：一份契约，2026-11-07 按维护者的话写下（“先定义出来一个api contract，我自己建一个api，用honojs，开源出来 … 任何人都
可以提pr … 任何人也可以fork走，自建”）。FinchGram 读它；服务端由维护者用 Hono 在一个单独的仓库里做，开源，接受 PR，也可以
fork。FinchGram 这边还没做。机器可读的版本是 [live-directory.openapi.yaml](live-directory.openapi.yaml)（OpenAPI 3.1）；这一页
解释它，并说明客户端怎么用。

## 它是什么

一个目录说明哪些直播源值得看：会开直播的 Telegram 频道或个人、Owncast 服务器、PeerTube 频道、广播机构通过 HLS 提供的流。它
不说现在谁在播：客户端自己去问每个源（Telegram 走 TDLib，和今天的直播视图一样，见 docs/live-streams.zh-Hans.md；其他的走源自己
的 API）。所以目录就是一张靠 PR 改动的清单，原样提供，没有要轮询的东西，除了一个静态服务器之外不用一直跑任何东西：Cloudflare
Workers 加 Hono，或者任何回答同样 JSON 的 HTTPS 服务器。

谁都可以开一个。客户端可以认识好几个目录，各凭各的地址；目录之间不合并，每个都单独看（和音乐的来源一样）。

## 约定

- **传输。**只用 HTTPS。所有路径都在 `/v1/` 下。GET 和 HEAD；CORS 用 OPTIONS。没有 cookie，没有认证，请求里没有任何表明是谁在问的东西。
- **格式。**UTF-8 的 JSON（`application/json; charset=utf-8`）。字段名用 snake_case。日期和时间用 RFC 3339（`2026-11-07T05:09:47Z`；
  只有日期：`2026-11-07`）。
- **语言。**BCP 47 标签，写法和 FinchGram 一样：`en`、`zh-Hans`、`zh-Hant`、`ja`、`pt`；不需要地区时不带地区。国家用 ISO 3166-1
  两位字母（`US`、`CN`）。
- **多语言文本**是从语言到文本的对象：`{"en": "Keycaps, live", "zh-Hans": "键帽直播"}`。客户端显示界面语言的，没有就用 `en`，
  再没有就用第一个。
- **id** 是目录自己定的：小写字母、数字和连字符，2 到 63 个字符，以字母或数字开头（`mika-keys`）。要稳定：客户端可能按 id 记住一个源。
- **只增不改。**v1 之内字段只会增加。客户端忽略不认识的字段，跳过不认识种类的源。其他变动一律进 `/v2/`。
- **缓存。**回答带 `Cache-Control`（建议 `public, max-age=300`）和 `ETag`；带 `If-None-Match` 的请求在没变时得到
  `304 Not Modified`。
- **CORS。**`Access-Control-Allow-Origin: *`，网页也能读目录。
- **错误**是状态码加一个 JSON：`{"error": {"code": "not_found", "message": "No source mika-keys"}}`。错误码：`bad_request`（400）、
  `not_found`（404）、`rate_limited`（429，带 `Retry-After`）、`internal`（500）。

## 接口

### GET /v1/directory

这个目录是什么。

```json
{
  "api": 1,
  "name": "FinchGram Live",
  "description": {"en": "Live streams picked by FinchGram's maintainer", "zh-Hans": "FinchGram 维护者挑选的直播"},
  "homepage": "https://finchgram.com/live",
  "repository": "https://github.com/<owner>/<repository>",
  "policy": "https://github.com/<owner>/<repository>/blob/main/CONTRIBUTING.md",
  "contact": "mailto:live@example.com",
  "updated": "2026-11-07T05:09:47Z",
  "kinds": ["telegram", "owncast", "peertube", "hls"],
  "counts": {"sources": 42}
}
```

`api` 在这份契约里是 1。`policy` 是目录写明收什么、不收什么的地方（见下文“内容”）。`kinds` 是它列出的源的种类。私有目录可以不写
`repository`、`policy` 和 `contact`；其余字段总是有。

### GET /v1/categories

源所归的分类。

```json
{
  "categories": [
    {"id": "tech", "names": {"en": "Technology", "zh-Hans": "科技"}},
    {"id": "music", "names": {"en": "Music", "zh-Hans": "音乐"}}
  ]
}
```

为了 fork 之间说同一种话，这些 id 在每个目录里含义不变：`news`、`tech`、`gaming`、`music`、`education`、`sports`、`talk`、
`lifestyle`、`finance`、`art`、`science`、`religion`、`kids`、`other`。目录可以再加自己的。

### GET /v1/sources

所有源，按目录自己的顺序（排在前面的就是它想先给人看的）。

查询参数（都可选）：

| 名字 | 含义 |
|---|---|
| `kind` | 只要这种；可以重复（`kind=telegram&kind=owncast`） |
| `category` | 只要归在这个分类下的 |
| `language` | 只要这种语言的 |
| `country` | 只要这个国家的 |
| `q` | 名字或简介里的词，不分大小写 |
| `include_nsfw` | `true` 时包含标为 NSFW 的源；否则不给 |
| `limit` | 最多多少个，1 到 500；不给时 100 |
| `cursor` | 上一页结束的位置（它的 `next`） |

```json
{
  "sources": [
    {
      "id": "mika-keys",
      "kind": "telegram",
      "name": "Mika Keys",
      "description": {"en": "Keycap casting, most Friday nights"},
      "languages": ["en"],
      "categories": ["tech", "art"],
      "countries": ["US"],
      "nsfw": false,
      "picture": "https://example.com/mika.jpg",
      "website": "https://mika.example",
      "schedule": [{"days": ["fri"], "start": "20:00", "minutes": 90, "timezone": "America/Los_Angeles"}],
      "added": "2026-11-07",
      "updated": "2026-11-07",
      "telegram": {"username": "mika_keys"}
    }
  ],
  "next": null
}
```

`next` 是下一页的 cursor，最后一页是 `null`。几百个源的目录可以一次全给，`next` 写 `null`。

### GET /v1/sources/{id}

一个源，和列表里的一样；目录里没有这个 id 时回 404。

## 一个源

每个源都有：

| 字段 | 类型 | |
|---|---|---|
| `id` | 字符串 | 目录给它的 id（见“约定”） |
| `kind` | 字符串 | `telegram`、`owncast`、`peertube` 或 `hls` |
| `name` | 字符串 | 它自己的名字，1 到 100 个字符，不翻译 |
| `languages` | 字符串数组 | 用的语言，至少一个 |
| `categories` | 字符串数组 | 分类 id，至少一个 |
| `nsfw` | 布尔 | 成人内容：不特意要就不给 |
| `added` | 日期 | 进目录的日子 |
| 以种类命名的对象 | 对象 | 见下 |

还可以有：`description`（多语言文本，每种最多 500 个字符）、`countries`（字符串数组）、`picture`（方形图片的 https 地址，至少 160
像素）、`website`（https 地址）、`schedule`（通常什么时候播：数组，每项 `{"days": ["mon" … "sun"], "start": "HH:MM", "minutes":
1 … 1440, "timezone": IANA 时区}`）、`updated`（日期）。

各种类：

- **`telegram`**：`{"username": "mika_keys"}`，一个公开用户名（5 到 32 个字母、数字和下划线，以字母开头），不带 @ 和 t.me。个人的
  直播动态、频道的直播动态或直播、群的视频聊天：是哪一种，客户端通过 TDLib 去查。
- **`owncast`**：`{"server": "https://owncast.example.com"}`，服务器的源地址，不带路径。它的 `/api/status` 说 `online` 时就是在播；
  播放 `/hls/stream.m3u8`。
- **`peertube`**：`{"instance": "https://peertube.example", "channel": "keycaps"}`，实例的源地址和上面一个频道的 handle。频道有一个
  `isLive` 且已发布的视频时就是在播；播放那个视频的 HLS 列表。
- **`hls`**：`{"url": "https://broadcaster.example/live/index.m3u8"}`，https 上的一个播放列表。不带请求头：需要别人的 Referer 或
  User-Agent 才能播的流，目录不能给（客户端只发自己的 User-Agent，别的什么都不发）。

## 签名（可选）

目录可以给回答签名，让知道它密钥的客户端分辨出被改过的回答：响应头 `X-Directory-Signature: ed25519 <base64>` 是对响应体原样
字节的 Ed25519 签名。公钥和目录地址一起交给客户端（FinchGram 自带某个目录时会带上它的公钥）；目录自己提供的公钥证明不了什么，所以
没有提供公钥的接口。有公钥的客户端收到没有有效签名的回答就不用它。没有公钥时，就只靠 HTTPS。

## 内容

目录里放什么由它的维护者决定，写在 `policy` 里。契约本身只守这几条：源是它的主人公开放出来的地方；`hls` 源是版权方自己的流，或者
他们允许的流；观众本来没有的请求头、密钥或登录一样都不需要；成人内容要标出来。对 FinchGram 自己的目录的建议：先放 Telegram 的源
（内容留在 Telegram 上，以观众身份通过 Telegram 的 API 观看，没有任何人的流经过目录），再放主人愿意被收录的 Owncast 和 PeerTube
服务器，广播机构的流只在得到许可后放。fork 自己决定自己的，也自己负责。

## FinchGram 怎么读目录

- **什么时候。**打开直播视图时读用户已有的目录，每个至多五分钟一次，或者按它的 `Cache-Control`，带 `If-None-Match`。源显示在视图
  列表里目录名下，按目录的顺序，再按界面上的选择筛选。
- **Telegram 源。**通过 TDLib 按用户名解析，一次几个，滚到眼前才解析，绝不一次全解：短时间解析大量用户名正是 Telegram 限制的
  （FLOOD_WAIT）。解析过的聊天 TDLib 会记住。之后是不是在播，和视图现在一样判断：直播动态（getChatActiveStories），或者聊天的视频
  聊天。用户已经关注的频道不用解析。
- **其他的。**Owncast、PeerTube 和 HLS 源要等 HLS 播放器（docs/live-streams.zh-Hans.md，“待定”）；在那之前 FinchGram 把它们灰着
  列出，作为能显示但还做不到的东西。
- **从不发送。**关于用户或账号的任何东西都不会发给目录：一个普通的 GET，没有 cookie，`User-Agent: FinchGram/<版本>`。
- **守住的规矩。**不认识的种类跳过，不认识的字段忽略，NSFW 不显示，超过 16 MB 的回答不收，图片和聊天头像一样去取。

## 一个能用 PR 改动的仓库

给服务端仓库的建议，让一次贡献就是一个文件加一次检查：

```
data/directory.json          GET /v1/directory 回答的内容，不含 counts 和 updated
data/categories.json         分类
data/sources/<id>.json       每个源一个文件；文件名就是它的 id
schema/                      每种文件的 JSON Schema，取自 OpenAPI 文件的 components
src/index.ts                 Hono：路由，读构建时打包进去的数据
.github/workflows/check.yml  每个 PR 都按 schema 校验每个文件
```

数据在构建 Worker 时打包进去，所以 PR 合并后，下一次部署就生效；fork 部署自己的 Worker，就有了自己的目录。`counts` 和 `updated`
在构建时生成。

## 待定

1. FinchGram 是自带它自己的目录，还是列表从空开始、由用户按地址添加（维护者 2026-10-26 定的规矩是不替用户做任何选择）。
2. 它的地址（比如 `https://live.finchgram.com/v1/`），以及签不签名。
3. 它的规矩：收哪些种类，谁来审 PR。
