# The live directory: an API contract

[中文](live-directory.zh-Hans.md)

Status: a contract, written on 2026-11-07 at the maintainer's word ("先定义出来一个api contract，我自己建一个
api，用honojs，开源出来 … 任何人都可以提pr … 任何人也可以fork走，自建"). FinchGram's own directory is served at
`https://services.finchgram.com/live` by [FinchGram/Services](https://github.com/FinchGram/Services),
the site of the maintainer's services (built the same day on the website's stack, TanStack Start on
Cloudflare Workers, rather than a Hono server of its own), open to pull requests and to forks.
FinchGram itself does not read a directory yet. The machine-readable form is
[live-directory.openapi.yaml](live-directory.openapi.yaml) (OpenAPI 3.1), the same file as
`src/live/openapi.yaml` in that repository, whose tests keep the two identical; this page explains it
and says how a client behaves.

## What it is

A directory says which live sources are worth watching: a Telegram channel or person who goes live,
an Owncast server, a PeerTube channel, a stream a broadcaster offers over HLS. It does not say who is
live now: the client asks each source itself (TDLib for Telegram, as the live streams' view does
today, docs/live-streams.md; the source's own API for the others). So a directory is a list that
changes by pull request, served as it is, with nothing to poll and nothing to keep running beyond a
static server: a Worker on Cloudflare, as FinchGram's own is, or any HTTPS server that answers the
same JSON.

Anyone may run one. A client may know several directories, each by its address; none is merged into
another, each is looked at as itself (as the music sources are).

## Conventions

- **Transport.** HTTPS only. A directory is known by its base address, with a path or without one
  (FinchGram's own: `https://services.finchgram.com/live`), and every path is under `/v1/` below it.
  GET and HEAD; OPTIONS for CORS. No cookies, no authentication, nothing in the request that says
  who asks.
- **Format.** JSON in UTF-8 (`application/json; charset=utf-8`). Names are snake_case. Dates and
  times are RFC 3339 (`2026-11-07T05:09:47Z`; a date alone: `2026-11-07`).
- **Languages.** BCP 47 tags, written as FinchGram writes them: `en`, `zh-Hans`, `zh-Hant`, `ja`,
  `pt`; no region where none is needed. Countries are ISO 3166-1 alpha-2 (`US`, `CN`).
- **Localized text** is an object from language to text: `{"en": "Keycaps, live", "zh-Hans":
  "键帽直播"}`. A client shows the UI language's, else `en`, else the first one there.
- **Ids** are a directory's own: lowercase letters, digits and hyphens, 2 to 63 characters, starting
  with a letter or digit (`mika-keys`). Stable: a client may remember a source by its id.
- **Growing.** Within v1, fields are only ever added. A client ignores fields it does not know and
  skips sources of a kind it does not know. Anything else is a `/v2/`.
- **Caching.** Answers carry `Cache-Control` (suggested `public, max-age=300`) and an `ETag`; a
  request with `If-None-Match` gets `304 Not Modified` when nothing changed.
- **CORS.** `Access-Control-Allow-Origin: *`, so that web pages may read a directory too.
- **Errors** are a status code with a JSON body: `{"error": {"code": "not_found", "message": "No
  source mika-keys"}}`. Codes: `bad_request` (400), `not_found` (404), `method_not_allowed` (405,
  with `Allow`: the directory is read-only), `rate_limited` (429, with `Retry-After`), `internal`
  (500).

## The endpoints

### GET /v1/directory

What the directory is.

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

`api` is 1 for this contract. `policy` is where the directory says what it takes and what it does
not (below, Content). `kinds` are the kinds of source it lists. `repository`, `policy` and `contact`
may be left out by a private directory; the rest is always there.

### GET /v1/categories

The categories its sources are filed under.

```json
{
  "categories": [
    {"id": "tech", "names": {"en": "Technology", "zh-Hans": "科技"}},
    {"id": "music", "names": {"en": "Music", "zh-Hans": "音乐"}}
  ]
}
```

So that forks agree, these ids keep their meaning in every directory: `news`, `tech`, `gaming`,
`music`, `education`, `sports`, `talk`, `lifestyle`, `finance`, `art`, `science`, `religion`,
`kids`, `other`. A directory may add its own.

### GET /v1/sources

The sources, in the directory's own order (its first are the ones it would show first).

Query (all optional):

| Name | Meaning |
|---|---|
| `kind` | only this kind; may repeat (`kind=telegram&kind=owncast`) |
| `category` | only sources filed under this category |
| `language` | only sources in this language |
| `country` | only sources of this country |
| `q` | words in the name or the description, any case |
| `include_nsfw` | `true` to include sources marked NSFW; left out otherwise |
| `limit` | how many at most, 1 to 500; 100 if not given |
| `cursor` | where the previous page ended (its `next`) |

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

`next` is the cursor of the next page, or `null` on the last. A directory of a few hundred sources
may answer all of them at once and say `null`.

### GET /v1/sources/{id}

One source, as in the list; 404 for an id the directory does not have.

## A source

Every source has:

| Field | Type | |
|---|---|---|
| `id` | string | the directory's id for it (Conventions) |
| `kind` | string | `telegram`, `owncast`, `peertube` or `hls` |
| `name` | string | its own name, 1 to 100 characters, not translated |
| `languages` | array of strings | the languages spoken, at least one |
| `categories` | array of strings | category ids, at least one |
| `nsfw` | boolean | adult content: left out unless asked for |
| `added` | date | when it came into the directory |
| an object named after its kind | object | below |

And may have: `description` (localized text, each at most 500 characters), `countries` (array of
strings), `picture` (an https URL of a square picture, at least 160 pixels), `website` (an https
URL), `schedule` (when it is usually live: an array of `{"days": ["mon" … "sun"], "start": "HH:MM",
"minutes": 1 … 1440, "timezone": an IANA zone}`), `updated` (date).

The kinds:

- **`telegram`**: `{"username": "mika_keys"}`, a public username (5 to 32 letters, digits and
  underscores, starting with a letter), without @ or t.me. A person's live stories, a channel's live
  stories or live stream, a group's video chat: the client finds out which through TDLib.
- **`owncast`**: `{"server": "https://owncast.example.com"}`, the server's origin, no path. Live when
  its `/api/status` says `online`; plays `/hls/stream.m3u8`.
- **`peertube`**: `{"instance": "https://peertube.example", "channel": "keycaps"}`, the instance's
  origin and a channel's handle on it. Live when the channel has a video with `isLive` and the state
  published; plays that video's HLS playlist.
- **`hls`**: `{"url": "https://broadcaster.example/live/index.m3u8"}`, a playlist on https. No
  request headers: a stream that needs a Referer or a User-Agent of somebody else's is not one a
  directory may give (a client sends its own User-Agent and nothing else).

## Signing, if wanted

A directory may sign what it answers, so that a client that knows its key can tell a changed
answer from its own: the header `X-Directory-Signature: ed25519 <base64>` is the Ed25519 signature
of the body's bytes exactly as sent. The key is given to the client beside the directory's
address (FinchGram would carry the key of a directory it comes with); a key the directory serves
itself proves nothing, so there is no endpoint for it. A client that has a key and gets an answer
without a good signature does not use the answer. Without a key, HTTPS is all.

## Content

What goes into a directory is its maintainer's decision, written down at `policy`. What the contract
itself holds to: a source is somewhere its owner put it in public; an `hls` source is the rights
holder's own stream or one they allow; nothing needs a header, a key or a login a viewer would not
have; adult content is marked. A suggestion for FinchGram's own directory: Telegram sources first
(the content stays on Telegram, watched through Telegram's API as a viewer, and nobody's stream
passes through the directory), Owncast and PeerTube servers whose owners want it, broadcasters'
streams only with their permission. A fork decides its own, and answers for it.

## How FinchGram reads a directory

- **When.** On opening the live streams' view, the directories the user has, each at most every five
  minutes or as its `Cache-Control` says, with `If-None-Match`. Their sources show in the view's
  list under the directory's name, in the directory's order, filtered by the UI's choices.
- **Telegram sources.** Resolved by username through TDLib, a few at a time, as they come into
  view, never all at once: resolving many usernames quickly is what Telegram limits (FLOOD_WAIT).
  TDLib remembers a chat once resolved. Then live as the view already tells it: a live story
  (getChatActiveStories), or a chat's video chat. A channel the user follows needs no resolving.
- **The others.** Owncast, PeerTube and HLS sources wait for the HLS player (docs/live-streams.md,
  To decide); until then FinchGram lists them greyed, as what it shows but cannot do yet.
- **Never sent.** Nothing about the user or the account goes to a directory: a plain GET, no cookies,
  `User-Agent: FinchGram/<version>`.
- **Kept to.** Unknown kinds skipped, unknown fields ignored, NSFW left out, answers over 16 MB
  refused, pictures fetched as the chat photos are.

## FinchGram's own directory

[FinchGram/Services](https://github.com/FinchGram/Services) serves it, so that a contribution is one
file and a check:

```
data/live/directory.json       what GET /v1/directory answers, without what the build counts
data/live/categories.json      the fourteen shared categories, named in FinchGram's eleven languages
data/live/sources/<id>.json    one source each; the file's name is its id
src/live/openapi.yaml          this contract (the same file as this repository's)
src/routes/live/v1/            the routes; the contract itself is served at /live/v1/openapi.yaml
```

Every file is checked against the contract by the tests, on each pull request, and before each
build, so a file that fails never reaches the Worker; the data is bundled into it, so a merged pull
request is live at the next deploy, and a fork that deploys its own Worker has its own directory.
`updated` is the newest day a source was added or changed, `kinds` the kinds it holds, `counts` how
many. The repository's CONTRIBUTING.md says how to add a source. It opened empty: what goes in is
the maintainer's.

## To decide

1. Whether FinchGram comes with its own directory added, or the list starts empty and the user adds
   one by its address (the maintainer's rule of 2026-10-26 is that nothing is chosen for the user).
2. Whether it signs (its address is `https://services.finchgram.com/live`, decided 2026-11-07).
3. Its policy: which kinds it takes, and who reviews a pull request.
