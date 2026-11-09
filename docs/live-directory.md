# The live directory: an API contract

[中文](live-directory.zh-Hans.md)

Status: the contract of the maintainer's canvas "FinchGram Desktop 直播", rounds 12 and 13
(2026-11-07; claude.ai/artifact/RxoD91XwY8uqgZNNMGcuCB), at the address the maintainer chose the same
day, `https://services.finchgram.com/live/api/v1`. It replaces the first contract of 2026-11-07 (a
list of sources that changed by pull request and said nothing of who was live). FinchGram's own
directory is served by [FinchGram/Services](https://github.com/FinchGram/Services), open to pull
requests and to forks; FinchGram reads it in the Live view (docs/live-streams.md). The
machine-readable form is [live-directory.openapi.yaml](live-directory.openapi.yaml) (OpenAPI 3.1),
the same file as `src/live/openapi.yaml` there, whose tests keep the two identical; this page
explains it and says how FinchGram uses it.

## What it is

A directory of **channels** that go live, gathered from **sources** that allow it, with each
channel's **live session** and the time it was observed. It answers searches. It relays no video
and holds nothing about its users: no accounts, no follows, no stream keys, nothing in a request
that says who asks. Playback addresses are not in it either: the app asks each source for one when
it plays.

A Telegram channel's live status is not the directory's: a server has no Telegram account to ask
with. It lists the channel with its username as status `unknown`, and the app asks Telegram itself
through TDLib, as the Live view does for the chats the user has.

Anyone may run one: a fork of FinchGram/Services, or any server that answers the contract.

## FinchGram's own directory: the sources

| Source | How its channels come in | Live status | Viewers | Plays |
|---|---|---|---|---|
| Owncast | servers whose owners list them in the Owncast directory | the directory's home, every 5 minutes; a server pings it every 4 minutes while live | none (each server's own `/api` is off limits to crawlers by its robots.txt; the app asks the server when its user opens it) | in FinchGram (HLS) |
| PeerTube | public lives on instances whose robots.txt allows it, found through SepiaSearch once a day | each instance, every 5 minutes | exact | in FinchGram (HLS) |
| Icecast | stations their servers mark public in Xiph's directory (`yp.xml`) | every 15 minutes; a station missing from one export is not taken as off air | none in the export | in FinchGram, sound only |
| HLS | streams their broadcasters added by pull request, reviewed | their playlists, every 5 minutes | none | in FinchGram |
| Telegram | public channels added by pull request, reviewed | the app asks Telegram | the app's, from Telegram | in FinchGram (docs/live-streams.md) |
| Twitch, YouTube, Kick, Bilibili | not integrated yet: no developer keys, terms not settled | — | — | their links open in the browser |

A channel whose owner takes it out of its source's directory leaves this one too: Owncast at the
next sync, Icecast within a day, PeerTube within a week (when its robots.txt is read again).

What it leaves out by itself (the maintainer's word of 2026-11-08, when the app's first release
showed what the open sources hold): Owncast servers and PeerTube lives that are webcams, along with
PeerTube instances of webcams alone; loops that run day and night (24/7, 24h, nonstop, AutoDJ);
tests; and Owncast's placeholder words or a live without a title. Only Owncast and PeerTube are
judged so, by a channel's name, its stream's title and its host (src/live/quality.ts); a reviewed
channel never is. That day 43 of their 152 lives stayed. Icecast's stations, radio by the thousand,
are listed only when a search asks for Icecast.

## The four statuses

Each live session carries `observed_at`, `expires_at` and `last_confirmed_live_at`, and one status:

- `live`: the latest observation says live and is within the source's time to live (Owncast 5
  minutes, PeerTube and HLS 10, Icecast 20).
- `stale`: it was live when last observed, and that observation expired less than 60 minutes ago.
  The app says "Not confirmed right now" and offers "Try opening".
- `offline`: an observation of the last 30 minutes says not live.
- `unknown`: nothing reliable for 60 minutes, a source paused or not integrated, or one the
  directory cannot ask (Telegram's channels).

Live now lists the confirmed first and the stale after them; a cached answer is never shown as
current.

## Conventions

- **Transport.** HTTPS. A directory is known by its base address (FinchGram's own:
  `https://services.finchgram.com/live/api/v1`), and every path is under it. GET and HEAD, OPTIONS
  for CORS. No cookies, no authentication.
- **Format.** JSON in UTF-8. Names are snake_case; times are RFC 3339 in UTC.
- **Ids** are the directory's: `fg_ch_…` for a channel, `fg_se_…` for a session, made from the
  source and the channel's id there, so they stay the same at every sync and in every copy of the
  directory. A client may remember a channel by its id.
- **Languages** are BCP 47 tags as FinchGram writes them (`en`, `zh-Hans`, `pt`); `language=zh`
  takes in `zh-Hans` and `zh-Hant`. A source that says no language gives `null`.
- **Nothing made up.** Every field may be `null` where a source does not say; the app shows its
  fallback words then ("Language not given", "No thumbnail from Icecast", "—" for viewers), never a
  0 or an estimate.
- **Growing.** Within v1, fields are only added; a client ignores fields it does not know and treats
  an unknown enum value as the field's fallback.
- **Caching.** `Cache-Control: public, max-age=60` (`/sources`: 300) and a strong `ETag`;
  `If-None-Match` gets `304`. `/search` varies by `Accept-Language`.
- **CORS.** `Access-Control-Allow-Origin: *`, the ETag exposed.
- **Errors** are a status with `{"error": {"code", "message"}}`: `bad_request` 400, `not_found` 404,
  `method_not_allowed` 405, `rate_limited` 429 (with `Retry-After`), `internal` 500,
  `not_implemented` 501.

## The endpoints

### GET /search

`q` (words; none to browse), `mode` (`live`, `channels`, `upcoming`), `source` (repeatable; without
it, every source but those listed only when asked for, Icecast in FinchGram's),
`category`, `language`, `status` (in `mode=channels`), `playback` (`native`, `external`),
`content` (`general`, or `general,mature`), `sort`, `limit` (1 to 50, default 24), `cursor`.

- **Relevance** (with words): name over tags over title over category, source and language; at the
  same match, confirmed live first, then the newest start.
- **Recommended** (without words): confirmed live is the gate; then the app's languages, which it
  sends as `Accept-Language` (Settings › Live; nothing else in a request is personal); then those
  that started within the hour. Within each of these, the sources take turns, each in its own order
  (its viewer counts, then name), so counts are never compared across platforms and no source fills
  the page: Xiph's directory alone lists thousands of stations.
- **Most viewers** compares only within a source and puts channels without a count last, never as 0.
- **Pages.** `next_cursor` pages through one `snapshot`; a page of a newer snapshot may repeat an
  item, which the app drops by session or channel id. `total` is `eq`, `gte` (while a source lags) or
  `unknown`; `partial` and `missing_sources` say which source did not answer.
- **Facets** count each filter's options as if that filter were not set.

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

`q`, while the user types (the app waits 200 ms and cancels the older request): words from tags,
up to five channels (live first, then names that start with the words), and filters that match.

### GET /channels/{id}, /channels/{id}/sessions, /channels/{id}/schedule

One channel as in a search, its recent sessions (newest first), its announced sessions (empty for
a source without schedules).

### GET /channels/lookup

A link the user pasted, matched against the directory's own records: `platform` and `handle`
(`telegram` and a username, `peertube` and `channel@host`, `owncast` and a host), or `host` alone.
The server visits no address, and the app sends no path, query or credentials. `{"channel": null}`
when there is none, or several on that host.

### GET /sources

Each source: `indexing` (`enabled`, `not_integrated`, `paused`, `restricted_by_terms`,
`disabled_by_source`), how it finds channels, its capabilities (`live_status` true or `"client"`,
`viewer_counts`, `thumbnails`, `playback`, `auth`), its attribution, how long it keeps what it
observed, and its last sync (`ok`, `degraded`, `rate_limited`, `failed`, `none`).

### POST /submissions

Not in this version (phase 2): `501`. A channel comes in by a pull request.

## Playback

`methods` are what the source can play (`hls`, `dash`, `icecast`, `telegram`, `official_embed`,
`external`); `availability` is whether this one plays now: `available`, `unconfirmed` (stale or
unknown), `sign_in`, `unavailable`, with a `reason`. The app's one button follows them: Watch here,
Listen here (Icecast), Try opening (unconfirmed), Open on Twitch (a platform's own player only),
Sign in to watch, Unavailable.

## Content

`classification` is `general` or `mature` (strong language, violence or gambling); mature is left
out unless the request asks for it, and the app hides it until the user turns it on (Settings ›
Live). Adult streams are not listed in this version: an Owncast server or a PeerTube live its source
marks NSFW is left out, as the mark does not tell mature from adult. A reviewed
channel is a place its owner made public, a stream over HLS only from its broadcaster or with their
permission, nothing that needs a key, a login or another app's headers.

## How FinchGram reads it

- **Discover** is `/search` and `/suggest`, with the user's languages from Settings › Live as
  `Accept-Language`. Nothing about the account goes to the directory: a plain GET, no cookies,
  `User-Agent: FinchGram/<version>`.
- **Watch.** The app gets the playback address from the source itself when it plays: an Owncast
  server's `/hls/stream.m3u8` (and its `/api/status` for viewers and whether it is on air), a
  PeerTube video's playlist (`/api/v1/videos/{uuid}`), an Icecast station's stream, an HLS
  playlist; Telegram's streams as the Live view plays them (TDLib).
- **Telegram channels** in the directory are resolved by username through TDLib a few at a time as
  they come into view (resolving many quickly is what Telegram limits, FLOOD_WAIT), then their live
  stories and live streams as the Live view tells them.
- **Following** is kept on the Mac, never sent: the app asks the directory, or the source, about the
  channels followed.
- **Kept to.** Unknown fields ignored, unknown sources skipped, answers over 16 MB refused.

## FinchGram's own directory

```
src/live/openapi.yaml          this contract (the same file as this repository's)
src/live/sources.ts            each source's configuration; src/live/sync/ its reader
src/live/cron.ts               the syncs, every five minutes (the Worker's cron trigger)
src/live/snapshot.ts           a source's channels, observations and sessions, merged at each sync
src/live/search.ts             search, suggest, ranking (pure)
src/live/quality.ts            what it leaves out of Owncast and PeerTube
src/routes/live/api/v1/        the routes; the contract itself is served at /live/api/v1/openapi.yaml
data/live/channels/<id>.json   the reviewed Telegram channels and HLS streams
```

Each source's snapshot is one value in D1 (JSON, gzip, base64), so a sync is one write whatever the
source's size. The reviewed channels are checked on each pull request and before each build; the
repository's CONTRIBUTING.md says how to add one. None is in yet: what goes in is the maintainer's.

## To decide

1. Whether to write to Xiph (icecast@xiph.org) about reading `yp.xml` every 15 minutes: it states
   no terms, and the file is 8 MB uncompressed.
2. Twitch, YouTube and Kick: developer keys, and each one's terms, before they are indexed.
3. Whether to list adult streams, with what age checks, in which regions (round 12's open
   questions). Until then a channel its source marks NSFW is left out.
4. Submissions (phase 2).
