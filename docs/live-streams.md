# Live streams: what was found, and the first version

[中文](live-streams.zh-Hans.md)

Status: researched on 2026-11-06; the first version built on 2026-11-07 (below), Telegram's own
live streams, RTMP streams only. On 2026-11-06 the maintainer opened the subject ("咱们来仔细研究一下
live stream") with an idea in three parts: the content itself, perhaps creators (KOLs) brought in;
video from peer to peer, with a server in the middle only so that the two sides find each other;
and a relay service for a source whose bandwidth or latency is poor. The same day they asked which
live sources could be used at once, so that a first version gets made rather than discussed
("有哪些直播源，立刻可以用的，咱们做第一版吧"). The creators are set aside at their word; this page holds
the technical findings, as of 2026-11-06 (prices and versions read that day), for the decision on
the first version, taken on 2026-11-07 (A first version, as chosen, below). The sources' terms were checked on
2026-11-06 and 07.

## Before the first version

- The toolbox has a grey **Live streams** card (`src/toolbox.rs`, id `live`), from design round 7,
  whose line reads "Live streams going on in your groups and channels": the design meant
  Telegram's own live streams, not a network of FinchGram's own.
- The pinned TDLib, 1.8.67, carries the whole live API (next section). The app speaks to
  `finchgram-tdlib` in JSON, one object per line over stdin and stdout; TDLib's `bytes` come
  base64-encoded, so a second of video (a few hundred KB) is a long line, nothing more.
- The player is libmpv with an FFmpeg built without network protocols, encoders or muxers
  (`vendor/mpv/build.sh`). mpv cannot fetch an HLS or RTMP address itself; the app fetches and
  hands bytes through its `finchgram://` stream (`src/player/stream.rs`), which already serves a
  file of unknown length (`size` answers `MPV_ERROR_UNSUPPORTED`) and need not seek. Music
  (`src/music/fetch.rs`) is the model: ureq fetches, a cache file grows, mpv reads as it arrives.
- The app has no WebRTC, no story UI beyond `messageStory`, and nothing of group calls.

## One to many: the relay is the road, not the ditch

The idea's second and third parts are right for calls and wrong for live streams.

- **Peer to peer with a rendezvous server** is WebRTC: a signalling server carries the offer and
  the candidates, STUN tells each side its public address, and when a direct path cannot be
  punched the media goes through a TURN relay. That relay is never optional: measured shares of
  connections needing TURN are 22% (callstats.io, 2016), 12% over UDP plus 5% over TCP (fippo,
  2017), about 4% once a server with a public address is one end; libp2p's 2025 study of 4.4
  million hole-punch attempts across 85,000 networks succeeded in 70% ± 7%.
- **A live stream is one to many**, and the broadcaster's uplink carries the same stream once per
  viewer. At 2.5 Mbps a viewer, with 60% of the uplink usable (Ookla medians of September 2026):

  | Uplink | Viewers a mesh can carry |
  |---|---|
  | US cable, 10–35 Mbps up | 2–8 |
  | World fixed median, 68 Mbps | about 15 |
  | Phone, 14–34 Mbps | 3–4 |

  Each peer also costs the sender a congestion controller, a retransmission queue and SRTP.
  Mesh video is sound to about 5 viewers and hopeless past about 15.
- **A relay cannot give a source more uplink.** When the source's uplink is below the encoder's
  bitrate, only the encoder can help, and WebRTC's congestion control (GCC with transport-wide
  feedback) lowers it; what everyone then receives is that lowered stream. What a relay does do is
  three other things: the source uploads once; the source sends two or three simulcast layers (RFC
  8853) or one SVC stream (VP9, AV1), and the relay gives each viewer the layer their own downlink
  takes, so a slow viewer never throttles the source; and a relay near the source shortens a bad
  first hop. Paths between two home ISPs are often worse than home to datacenter to home: in the
  libp2p data, 10% of punched direct paths had a higher round trip than the relayed one, and
  Riot measured a San Francisco–Portland game path of 70 ms through Los Angeles, Denver and
  Seattle against 14 ms direct. Zoom, Meet and Teams route every call through their servers. In
  the trade this relay is an SFU (selective forwarding unit); cascaded by region it is what the
  managed services sell.
- **Viewers forwarding to viewers** (peer-assisted delivery, "P2P CDN") has left the consumer
  market: Streamroot's mesh delivery shut on 2023-09-30, Peer5 became Microsoft's eCDN for
  corporate town halls (2021), Hive and Kollective sell to enterprises, Bilibili runs its own in
  the browser over HLS blocks. It needs segmented HLS (10–30 s of latency; the plugins do not
  support LL-HLS), many viewers of one stream on good networks, and gives nothing to 20–200 viewers
  scattered across ISPs; estimate: not worth its complexity below about 500 concurrent viewers of
  one stream.

What each approach costs, as a rule of thumb:

| Approach | Latency | Who pays bandwidth | Viewers | Fails when |
|---|---|---|---|---|
| Direct mesh | < 0.5 s | the broadcaster | 5–15 | uplink saturates, worst ISP path wins, TURN needed anyway |
| One SFU relay | 0.5–1 s | us | ~300 per Gbit of a VPS; thousands managed | one region only; the source's uplink is still the floor |
| Cascaded SFU | < 1 s | us | 10⁴+ | complexity; in practice bought (LiveKit Cloud, Cloudflare) |
| Peer-assisted + SFU seed | 10–30 s | viewers and us | large, co-located | few viewers, mobile peers, LL-HLS |
| LL-HLS over a CDN | 2–5 s | us, per GB or minute | unlimited | no sub-second interaction |
| HLS over a CDN | 15–30 s | us | unlimited | latency |
| MoQ relay | < 1 s claimed | us or Cloudflare | large | draft protocol, wire changes, no third-party players |

Prices read on 2026-11-06, per viewer-hour at 2.5 Mbps (1.125 GB): Cloudflare Realtime SFU
$0.05/GB egress, so $0.056, with 1 TB a month free; Cloudflare Stream $1 per 1,000 minutes
delivered, $0.06; LiveKit Cloud $0.14–0.17; Amazon IVS real-time $0.072; Agora broadcast $0.12;
Daily and Twilio about $0.24. Self-hosted: a Hetzner cloud machine includes 20 TB a month (about
17,800 viewer-hours) and then €1/TB; the cap is the NIC, about 300–400 viewers per gigabit.
Software: LiveKit (Go, Apache-2.0), MediaMTX (Go, MIT, one binary, WHIP/WHEP/SRT/RTMP/HLS), SRS
(C++, MIT), mediasoup, Janus, OvenMediaEngine; coturn for TURN.

Protocols: WHIP (ingest) is RFC 9725 since March 2025; WHEP (playback) is still a draft
(draft-ietf-wish-whep-05, October 2026). Media over QUIC is draft-ietf-moq-transport-23, not an
RFC; Cloudflare runs relays in 330+ cities (free beta, $0.05/GB planned) and Meta moved Instagram
Live's ingest to it (October 2026); the Rust code (moq-dev/moq: moq-net, hang, moq-relay) is all
tokio and says its APIs and protocols may change between releases. Not a thing to build on in 2026
for an app that must keep working.

## Telegram already runs it

| Product | Who starts it | Viewers | Since |
|---|---|---|---|
| Video chat in a group | an admin with `can_manage_video_chats` | unlimited listeners, up to 1,000 with video | 2021 |
| Live stream in a channel | the same right | unlimited | 2021-08-31 |
| RTMP ingest ("Stream With…") | the same, `createVideoChat(is_rtmp_stream)` | unlimited | 2022-03-11 |
| Conference group call (links) | any user, `createGroupCall` | 200, end-to-end encrypted | 2025-04-30 |
| **Live Stories** | **every user**; channels and supergroups with stories unlocked by boosts | not published | **2025-11-19** |

**Live Stories** (telegram.org/blog/live-stories-gift-auctions): "Every Telegram user can now host
live streams in Telegram Stories"; the privacy settings are a story's (everyone, contacts, close
friends, selected); "streams in stories support RTMP keys for external apps like OBS and XSplit";
viewers' comments can be paid in Stars and are pinned longer and coloured by tier (TDLib's
`groupCallMessageLevel`: for example 2,000 Stars pin for 1,800 s with 280 characters, 10,000 for
3,600 s with 400 characters; the broadcaster sets the minimum per comment, up to 35,000; a paid
comment with no text is a tip); the broadcaster sees the total, and a top-donors list exists. One
live story per account at a time; it shows in the followers' story ring and at `t.me/<username>/s/live`;
when it ends it is replaced by the recording as a video story. TDLib carries it since 1.8.57
(2025-11-26); 1.8.67's signatures match master. Not published: the viewers' cap, the creator's
share of the Stars (paid messages give the recipient 85% under the Stars terms), the app version.

What a client that speaks only TDLib can do, from tdesktop, Telegram iOS, tgcalls, Unigram and
Telegram Web K:

- **Watch an RTMP-mode stream** (a channel's RTMP live stream, an RTMP live story) **with no
  WebRTC at all.** Join with `joinVideoChat` or `joinLiveStory`, muted and without video, giving a
  placeholder payload (tdesktop sends a random `ssrc` with empty `ufrag`, `pwd` and
  `fingerprints`; the answer says `"stream": true, "rtmp": true`); `getGroupCallStreams` gives the
  channels and the time the stream currently ends; then `getGroupCallStreamSegment` once a second,
  from `floor(end / 1000) × 1000 − 2000` ms on, `channel_id` 1 for RTMP ("unified": sound and
  picture in one segment), quality Full. `TIME_TOO_BIG` means wait 100 ms and ask again;
  `GROUPCALL_JOIN_MISSING` means join again. A segment is a small header (int32 signature
  `0xa12e810d`, a TL string with the container name, "mp4", int32 active mask, int32 event count,
  then events of int32 offset, TL string endpoint, int32 rotation, int32 extra) followed by a
  **complete MP4 of one second**, Opus sound, H.264 picture by all signs (the RTMP ingest takes
  H.264; not confirmed in any document). Audio-only segments are Ogg Opus at 48 kHz with
  `TG_META`, `ACTIVE_MASK` and `ENDPOINTS` comments. Telegram Web K plays these in plain
  JavaScript (`src/lib/calls/rtmpCallsController.ts`, `videoStreamInfo.ts`,
  `serviceWorker/rtmp.ts`), remuxing each MP4 into fragments for the browser; tdesktop and
  tgcalls do the same in C++ (`calls_group_call.cpp`: `LoadPartTask`, `broadcastPartStart`;
  `StreamingMediaContext.cpp`: segments of 1,000 ms, a buffer of 2,000 ms; `VideoStreamingPart.cpp`).
  So the app's libmpv, which decodes H.264 and Opus and demuxes MP4, is enough; the one piece
  missing is a remux, since a chain of complete MP4 files is not a stream mpv can read as one
  (below).
- **Stream mode is the server's decision.** Telegram's API documentation (core.telegram.org/api/group-calls,
  "Stream mode", read 2026-11-07) says a video chat, livestream or live story is served as
  downloadable chunks in two cases: RTMP mode, chosen by the creator; and automatic scaling,
  "once the audience grows past around 30 participants for channel livestreams, 1000 participants
  for supergroup/basic group video chats, or 100 participants when connected to a test DC",
  thresholds that are server-controlled and may change; no number is given for a user's live
  story. The join answer tells which: `{"stream": true, "rtmp": true}` for RTMP, `{"stream":
  true}` for automatic scaling, neither for WebRTC. In automatic stream mode the chunks differ
  from RTMP's unified channel: the mixed audio is an Ogg chunk whose `ENDPOINTS` and `ACTIVE_MASK`
  metadata name the video channels, each publisher's video is fetched as a chunk of its own, and
  the clock comes from the client's synced server time rather than from `getGroupCallStreams`.
  A first version would take RTMP mode first (one MP4 a second) and the automatic mode after it
  (audio and video in step).
- **Cannot watch** a live story or video chat that stays on WebRTC: a phone's camera stream with a
  small audience, and every conference group call (those have no stream mode). These need what
  the official clients have in tgcalls on libwebrtc; no Rust client has it, and the only
  non-libwebrtc implementation found is a Go bot that sends and does not receive. Such a stream
  would be listed with "watch on your phone".
- **Broadcast through RTMP.** `getVideoChatRtmpUrl` and `getLiveStoryRtmpUrl` give an address of
  the form `rtmps://dc4-1.rtmp.t.me/s/` and a key; OBS, ffmpeg (`-f flv -c copy`, H.264 and AAC),
  go2rtc and MediaMTX all push to it. One publisher per stream; Telegram's only published advice is
  software encoding (x264 in OBS); no bitrate cap is documented. Screen sharing and recording calls
  of the API are not for RTMP streams or live stories (the broadcaster records in OBS).
- **Broadcasting the Mac's camera or screen as a participant** needs a WebRTC stack: `joinVideoChat`
  only carries the signalling (`ssrc`, `ufrag`, `pwd`, `fingerprints`, `ssrc-groups` out; transport
  candidates and payload types back).
- **Latency:** the clients stay 2 s behind the live edge and buffer one more segment, so about 3 s
  on the client side plus the server's segmenting; one user measured about 12 s from OBS to the
  viewers (OBS forum, 2025-08). Telegram publishes no figure.

This is Telegram's own feature, used as the official clients use it, inside the API terms as they
are read for this project: watching joins the call as a listener, which is what a viewer is.

## Sources a first version could use

A source counts as usable when a signed, notarized app from a US company can play it without a
WebView, without a shipped key, and without an endpoint whose terms forbid third-party players.
Checked on 2026-11-06 and 07 against the terms pages, the repositories and probes of the streams
themselves.

| Source | How the app gets it | Verdict | Notes |
|---|---|---|---|
| Telegram live streams and Live Stories | TDLib, as above | usable | the one source only a client like FinchGram can offer; stream-mode calls only |
| An HLS address or an M3U list the user brings; Xtream or Stalker credentials | the user pastes it | usable, user-supplied | what every IPTV player does (TiviMate, IPTVnator, VLC: no channels of their own, a disclaimer); the app ships nothing. Xtream is the API of paid IPTV subscriptions, most of them pirate: credentials the user types, never a list |
| Owncast servers and the Owncast directory | `/hls/stream.m3u8` on each server, `/api/status`, chat over a WebSocket; the directory's `api/iptv` M3U | usable | self-hosted, MIT; 75 live and 147 offline instances listed at one moment on 2026-11-07; 5–18 s behind; the status and chat calls are marked internal, may change |
| PeerTube live, SepiaSearch | `GET /api/v1/videos?isLive=true`, HLS as static files; sepiasearch.org's API across instances | usable | federated; 129 live at one moment; 30–40 s behind by design; the P2P belongs to the web player, a native client just fetches |
| Nostr live activities (NIP-53), zap.stream | one relay subscription for kind 30311 events carrying an HLS address | usable as a protocol; each host's terms its own | 34 marked live on four relays, 12 of them fresh |
| iptv-org | the API's `channels.json`, `feeds`, `logos`, `guides` (Unlicense), and `streams.json`: 18,602 user-submitted links | the metadata open; the links grey: fetched at run time at the user's choice, never bundled | github.com/iptv-org/iptv, 143,000 stars; README: links "which to the best of our knowledge have been intentionally made publicly by the copyright holders", removed on a claim; 836 blocklisted entries, 19 DMCA notices at GitHub, the repository dark for five weeks in 2020 after one; 6.6% geo-blocked, 6.8% need a header |
| Free-TV/IPTV | one M3U on GitHub: 1,884 live channels, 124 of them YouTube, 213 geo-blocked | grey; fetched at run time if at all | 21,800 stars, no licence file, URLs taken from iptv-org; Linux Mint's Hypnotix fetches it since 2021-04-15 ("Hypnotix does not provide content or TV channels"); no complaint against Mint found |
| Public broadcasters' own HLS | an m3u8 inside their pages | grey to closed | DW, NHK World, Arirang, TRT World, ABC News Australia and tagesschau24 reachable, terms personal and silent on players; Bloomberg, Sky News, CGTN, Al Jazeera, CNA and Euronews reachable but their terms forbid tools and third-party apps; France 24, ESA, NASA+, PTS, Rai, ZDF and BBC only through YouTube, logins or geo-fences; NASA TV's HLS died in 2024 |
| CCTV and the Chinese "直播源" lists (imDazui/Tvlist-awesome-m3u-m3u8, fanmingming/live, Guovin/iptv-api, YanG-1989/m3u, 肥羊) | M3U lists pointing at operators' IPTV endpoints and private relays | closed to ship; a list the user brings is theirs | 央视网's notice forbids linking; suits against 电视猫 and PPTV; the 2015 四部门 229号文 makes a for-profit link service a crime; 电视家 shut in 2023-11; the lists' own READMEs say "仅供测试研究使用"; mostly unreachable outside China |
| FAST services (Pluto TV, Xumo, Samsung TV Plus; Roku, Tubi, Rakuten, Plex) | circulating lists | closed | Pluto's terms forbid any interface but theirs, Xumo's the same; MPA and Warner notices took the lists down (i.mjh.nz, 2024); Pluto needs a session token since 2026 |
| Twitch, Kick, Rumble, Odysee | unofficial endpoints | grey | the terms forbid tools (Twitch, Kick, Rumble); Twitch adds integrity tokens and stitched ads; Odysee's API is open but undocumented |
| YouTube Live, Bilibili, Douyu, Huya, Douyin, Instagram, TikTok, Facebook | none, or signed endpoints | closed | YouTube's developer policies; Bilibili sent three lawyer letters in 13 months (PiliPala 2025-07, bilibili-API-collect 2026-01, BiliRoaming 2026-07); Douyin won ¥1,000,000 against 小葫芦; Trovo and DLive shut down in 2026 |
| Jellyfin's and Emby's Live TV | their APIs, the user's server | later | with those two tools |

What the streams look like, measured on 550 of iptv-org's (400 drawn at random, 150 Chinese):
95% MPEG-TS segments, 2% fMP4, AES-128 on 15% (almost all FAST, none of the Chinese), LL-HLS on 2
of 287, DASH 1.2% (mostly BBC, geo-fenced anyway); median segment 5 s (2 s at the tenth
percentile, 10 s at the ninetieth), so 10–40 s behind live; 15% of master playlists redirect to
tokenised addresses; `EXT-X-DISCONTINUITY` in 35%; a few "playlists" that are raw TS or gzip;
geo-blocks arrive as Akamai's 403 page. DRM (SAMPLE-AES, FairPlay, Widevine) is out by
construction: the app recognises the key format and says so.

What the player must do, then: master and media playlists; the live reload of RFC 8216 §6.3.4
(the target duration, half of it when nothing changed, stalled past one and a half); relative
addresses resolved against the final URL after redirects; a look at the body (`#EXTM3U`, `0x47`,
gzip) before trusting the extension; TS and fMP4 (the init segment, sent again on a map change
or a discontinuity); AES-128 with the key fetched by the same client; discontinuity resets; 403 and
404 told apart; unknown tags ignored. Later: byte ranges, DASH, adaptive bitrate, alternate audio
and subtitles. Headers: 5.8% of iptv-org's streams want a `User-Agent` and 1.8% a `Referer`; a
Referer naming a page the user never opened is a hotlink bypass, the one thing that turns a player
into a circumvention tool, so the app sends a truthful User-Agent of its own and takes per-stream
headers only as fields the user types, never from a shipped list. Since FFmpeg's own HLS demuxer
needs FFmpeg's network (`--disable-network` in the vendored build), the client is the app's:
`m3u8-rs` 6.0.1 (2026-07) parses playlists, ureq fetches on a thread as in Music; Streamlink is
the closest precedent of an app-side HLS client piping into mpv.

## Building blocks, in Rust and on macOS

| Need | What exists (2026-11-06) | Fit |
|---|---|---|
| WebRTC, if ever | `str0m` 0.25 (sans-IO, fully synchronous, production at Lookback; BWE, TWCC, simulcast; no TURN client) · `webrtc` 0.21 (tokio, release-candidate quality, bandwidth estimation since 2026-09) · LiveKit's `libwebrtc` (a 264 MB prebuilt downloaded at build time, sources a moving branch) | str0m fits the thread model; LiveKit's prebuilt fails the pinned-sources rule |
| MoQ | moq-dev/moq (`moq-net`, `hang`, `moq-relay`, `moq-cli` with ScreenCaptureKit capture and VideoToolbox) | all tokio, wire format moving |
| HLS | `m3u8-rs`, `hls-runtime` | parse here, fetch with ureq |
| MPEG-TS, MP4 | `mpeg2ts` 0.6 (reads and writes), `mp4-atom` 0.17 (fMP4 boxes, used by moq) | enough for a remux of Telegram's segments |
| RTMP push, if ever | `rml_rtmp` 0.8 (dormant since 2023), xiu's `rtmp` (tokio, server-first) | nothing maintained and synchronous; FLV muxing is small, the handshake and chunking are known |
| Capture and encode, if ever | `objc2-screen-capture-kit`, `objc2-av-foundation`, `objc2-video-toolbox` 0.3 (generated, complete) · `cidre` (its `sc-record` example: ScreenCaptureKit to VideoToolbox HEVC plus AudioToolbox AAC) · `opusic-sys` bundles libopus 1.6.1 built from the crate's own sources | VideoToolbox emits AVCC, RTP wants Annex B |
| mpv | `stream_cb` takes an endless stream; the `low-latency` profile is `cache-pause=no`, `demuxer-lavf-o-add=fflags=+nobuffer`, `demuxer-lavf-analyzeduration=0.1`, `video-sync=audio`, `interpolation=no`, `stream-buffer-size=4k`; `--untimed` breaks with sound | the renderer stays as it is |

The remux of Telegram's segments: each second is a complete MP4, and FFmpeg's mov demuxer stops
at the first file's end. The app has to take the samples out of each MP4 (`mp4-atom`) and write
one continuous stream for mpv, MPEG-TS (Opus needs its registration descriptor and control header;
H.264 goes from AVCC to Annex B) or Matroska (simpler: clusters of unknown size, both codecs as
they are). The other way, enabling FFmpeg's `mpegts` muxer in `vendor/mpv/build.sh` and calling
libavformat from the app, would add a direct FFmpeg dependency where today only libmpv is called;
a few hundred lines of Rust seem the better trade. To be decided when built.

## The first version, as built

Chosen on 2026-11-07 ("要不第一版把telegram自带的live stories搞出来吧 … 固定在左侧"; "目前左侧有三个：
chat，channel，bot，对吧，加一个live stories") and built the same day, in every theme.

- **Where:** a fourth view on the left, beside the chats, the channels and the bots: Workbench's
  activity bar (a red dot while somebody is live), Broadsheet's row of views, Terminal's `LIVE` tab
  and `/live`. The toolbox's Live streams card opens it (it is not pinnable: its place is on the
  left already). The list and the pane are drawn once, in Look's colours, in `ui/pages/live.slint`;
  the state is the `Live` global (`ui/live.slint`).
- **Who is live** (`src/telegram/live.rs`): the live stories of the story list, loaded once the
  account is ready (loadActiveStories until 404, then updateChatActiveStories; getStory gives a
  live story's group call), and the video chats and live streams of chats (a chat's video_chat on
  updateNewChat and updateChatVideoChat, then getGroupCall; only active ones are listed). The
  chat lists carry a LIVE mark for such a chat (`ChatRow.live`), and a bar under a live chat's
  header opens it in the view.
- **Watching:** only RTMP streams, the ones OBS and the like send. Joined as a viewer
  (joinLiveStory, or joinVideoChat), muted, without video, with the payload Telegram Web K sends
  (`{"fingerprints":[],"pwd":"","ssrc":<random>,"ssrc-groups":[],"ufrag":""}`); an answer without
  `"stream": true` is left at once and shown as for the phone. getGroupCallStreams gives the edge
  (channel 1 is the unified channel of an RTMP stream); the feed starts 2 s behind it and asks
  getGroupCallStreamSegment for each second once it exists, by the edge moving on with the clock
  (TIME_TOO_BIG: again after 250 ms; TIME_TOO_SMALL: the edge again; GROUPCALL_JOIN_MISSING: joined
  again, once; FLOOD_WAIT honoured; eight other failures in a row give up). Each second's header is
  read (`segment_file`), the MP4 remuxed into Matroska (`src/player/remux.rs`: the samples of one
  H.264 or HEVC and one Opus or AAC track, by the edit list's start, a cluster a second, the picture
  starting at a key frame) and pushed into a pipe in memory (`src/player/pipe.rs`) that an mpv of
  its own plays (`src/player/live.rs`). A change of resolution or codec, or an mpv more than about
  48 MB behind, starts the stream again with a new header. Verified here with the vendored libmpv:
  three seconds made by ffmpeg, fed one a second, play through with both tracks and nothing
  dropped (`src/player/pipe.rs`'s test); the real thing is verified in the release, against a live
  story streamed from OBS.
- **Not played:** a non-RTMP stream (a phone's camera) is listed greyed with "Watch on your phone"
  and is not joined. Telegram switches such streams to stream mode once their audience is large
  (about 30 in a channel, 1,000 in a group), but then sends sound and each picture apart; that is
  for a later version.
- **Comments:** the group call's messages (updateNewGroupCallMessage, the last 100 kept), coloured by
  updateGroupCallMessageLevels; free ones are sent with sendGroupCallMessage. Paid comments and tips
  are shown; sending them is left to Telegram's own apps for now (the star button is faded), and a
  stream whose comments cost Stars says so instead of a composer.
- **The host:** the user's own live story, or a stream they manage (can_be_managed), opens as its
  host: viewers, Stars so far and the top three supporters (getLiveStoryTopDonors,
  updateLiveStoryTopDonors), comments on or off (toggleGroupCallAreMessagesAllowed), a comment
  deleted on hover (deleteGroupCallMessages), and End live story or End live stream, after a
  question (endGroupCall).
- **Going live:** the user's own live story (startLiveStory with the Saved Messages chat id,
  is_rtmp_stream, who may see it: everyone, contacts or close friends; comments on or off and the
  Stars a comment costs) or a channel they own or whose video chats they manage (createVideoChat
  with is_rtmp_stream and a title). The form shows the RTMP server and key at once
  (getLiveStoryRtmpUrl or getVideoChatRtmpUrl), with Copy, Show and Get a new key (the replace
  calls); Start opens the new stream as its host. A live story already on is opened instead; the
  reasons a story cannot start (Premium, boosts, limits) are said in the form.
- **A link:** t.me/<username>/s/live pasted in the list's field opens that live story
  (getInternalLinkType, searchPublicChat, getChatActiveStories).
- **The open chat:** while the view is open it covers the open chat, so what arrives there is not
  marked as seen (conversation.rs) until the view closes; closing the view leaves the stream (the
  user's own stream goes on), so that nobody is counted as a viewer who is not watching.

## The design

Drawn on 2026-11-07 at the maintainer's word ("你自己直接画一个artifacts，把url记录下来就可以了"),
Workbench light, five boards of 1280×800 on the Design canvas **FinchGram Live**,
https://claude.ai/artifact/A9SvXayxBSKmxCgeJKnqz4 (private to the maintainer's account; the
boards' sources are its files under `project/`):

1. Live · watching a live story: the fourth entry selected with its dot, the list of four live
   streams (one greyed, phone only), the viewer with the comments column.
2. Live · nobody live: the empty list with the link field, the main pane's empty state.
3. Live · Go live: the form, the RTMP server and key, the preview waiting for OBS.
4. Live · hosting: the host's view with viewers, Stars, top supporters, a spam comment being
   deleted, End live story.
5. Chats · a channel is live: the Live entry's dot, LIVE marks on chat rows, a live bar under the
   channel's header with Watch.

A note on the canvas lists the first version's limits. Broadsheet and Terminal were not drawn on
the canvas: they are the same page in their own look (ui/pages/live.slint), and the picture test
draws every one of them, light and dark (`cargo test screenshots -- --ignored`, the live-* pictures).

## To decide

1. Whether the streams Telegram switches to stream mode for a large audience are played too (sound
   and each publisher's picture come apart, and have to be put together).
2. Whether paid comments and tips are sent from FinchGram (Stars the account holds; levels from
   TDLib), and with what question before each.
3. The later versions' sources: a public TV directory fetched at run time at the user's choice
   (iptv-org's links, Free-TV), or only lists the user brings; Owncast and PeerTube directories.
4. Whether going live from the Mac's own camera or screen is wanted, which decides whether an RTMP
   client and the VideoToolbox pipeline get built.

## Read for this

Telegram: https://telegram.org/blog/live-stories-gift-auctions ·
https://telegram.org/blog/live-streams-forwarding-next-channel ·
https://telegram.org/blog/downloads-attachments-streaming · https://telegram.org/blog/group-calls-made-easy ·
https://core.telegram.org/api/group-calls · https://core.telegram.org/api/stories ·
https://core.telegram.org/constructor/inputGroupCallStream · https://telegram.org/tos/stars ·
https://github.com/tdlib/td/blob/master/td/generate/scheme/td_api.tl ·
https://github.com/telegramdesktop/tdesktop/blob/dev/Telegram/SourceFiles/calls/group/calls_group_call.cpp ·
https://github.com/TelegramMessenger/tgcalls/tree/master/tgcalls/group ·
https://github.com/morethanwords/tweb/blob/master/src/lib/serviceWorker/rtmp.ts ·
https://github.com/UnigramDev/Unigram/blob/develop/Telegram/Services/Calls/VoipGroupCall.cs.
Topology and prices: https://www.speedtest.net/global-index · https://webrtchacks.com/?p=2560 ·
https://medium.com/@fippo/what-kind-of-turn-server-is-being-used-d67dbfc2ff5d ·
https://arxiv.org/abs/2510.27500 · https://www.riotgames.com/en/news/fixing-internet-real-time-applications-part-i ·
https://www.rfc-editor.org/rfc/rfc8853 · https://www.rfc-editor.org/rfc/rfc9725 ·
https://datatracker.ietf.org/doc/draft-ietf-wish-whep/ · https://datatracker.ietf.org/doc/draft-ietf-moq-transport/ ·
https://blog.cloudflare.com/moq/ · https://blog.cloudflare.com/moq-relays/ ·
https://engineering.fb.com/2026/10/12/video-engineering/shipping-media-over-quic-at-meta-scale-for-instagram-live-ingest/ ·
https://developers.cloudflare.com/realtime/sfu/pricing/ · https://developers.cloudflare.com/stream/pricing/ ·
https://livekit.com/pricing · https://aws.amazon.com/ivs/pricing/ · https://www.hetzner.com/cloud/regular-performance/ ·
https://assets.lumen.com/is/content/Lumen/us-mesh-delivery-load-balancer-ecdn-eol-notification ·
https://learn.microsoft.com/en-us/ecdn/ · https://github.com/Novage/p2p-media-loader.
Rust and macOS: https://github.com/algesten/str0m · https://webrtc.rs/blog/2026/09/19/announcing-webrtc-v0.21.0.html ·
https://docs.rs/crate/webrtc-sys-build/latest/source/src/lib.rs · https://github.com/moq-dev/moq ·
https://docs.rs/hls-runtime · https://crates.io/crates/m3u8-rs · https://crates.io/crates/mpeg2ts ·
https://crates.io/crates/mp4-atom · https://github.com/yury/cidre/blob/main/cidre/examples/sc-record/main.rs ·
https://docs.rs/crate/opusic-sys/latest · https://github.com/mpv-player/mpv/blob/master/include/mpv/stream_cb.h ·
https://github.com/mpv-player/mpv/blob/master/etc/builtin.conf.
Sources' terms: https://owncast.online/docs/watching-streams/ · https://owncast.online/docs/directory/ ·
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
https://github.com/streamlink/streamlink/blob/master/src/streamlink/stream/hls/hls.py.
