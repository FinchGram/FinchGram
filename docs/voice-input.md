# Voice input: speaking into the composer, in five steps

[中文](voice-input.zh-Hans.md)

Status: step 1, the user's own key, built on 2026-10-25 at the maintainer's word
("咱们做一下这个语音输入吧，就是语音输入，转文字，类似微信那样") with OpenAI alone, and opened to any vendor on
2026-10-26, at the maintainer's word that Chinese was far below WeChat's voice input and that the
services should go by their API's format and the user's own address and keys ("应该允许大家自己找vendor，
而不是固定成openai"), and the words right first ("以语音转文字的准确度为第一"); later the same day the
services became AI vendors the user adds, and voice input, tidying and transcribing voice messages
three features, each with its own switch, vendor and model, chosen by the user from lists and by
nobody else. An address of the user's own also covers most of step 3 (a server on
their Mac or network). Steps 2, 4 and 5 are not built. Voice input was decided on 2026-10-21 as the
first of the AI features. There was no design round: the microphone is the one the design already
drew in every composer, the strip above the composer is drawn the way each theme draws its strip
for replying, and the settings are a panel of the toolbox drawn with the settings page's rows; a
round in Claude Design can take their place. Since 2026-10-26 voice input is a tool of the toolbox,
at the maintainer's word ("这也是一个tool，而且不是telegram官方原生的feature"): off until the user turns it on
there. The trigger is the sparkle the design draws, in the accent and without a title, in
Workbench's tool row; the composer's microphone is voice messages', as every round of the design has
it (the round "秘密聊天与通话"). The first section says what is built; the rest of the page is the plan of
2026-10-21, with the facts it rests on.

## As built (2026-10-26)

- **Using it.** Once it is on (Toolbox → AI → Voice input), in a chat, the sparkle in the composer
  (Workbench and Broadsheet: next to the microphone; Terminal: type `/voice`, and again to
  finish). A click starts and a second click finishes; held while speaking (longer than 0.6 s),
  letting go finishes. While the user speaks, a strip above the composer shows bars for the
  microphone's loudness, the time, the main language when there are others (a click takes the next
  one, and starts again with it), and the words as they come (the newest two lines; in Terminal one
  line), or, for a format whose words come at the end, that they will. ⏎ or Done finishes too; Esc or
  × drops the words. Then the strip says "Finishing…" while the last words come and "Tidying the
  words…", and the words go into the composer after what it holds (with a space where the script has
  spaces), the cursor after them, for the user to send. A dictation finishes by itself after five
  minutes, so that a microphone left open does not go on costing the user.
- **When it cannot.** The strip says why, naming the vendor, with a button where something can be
  put right: no AI vendor chosen for voice input, no way of hearing chosen for an OpenAI-compatible
  one, no model chosen (each Set up, which opens the panel), no key (Add key, the same), an address
  FinchGram cannot use (Change address), the microphone not allowed (Open System Settings), the
  vendor refusing the key (401 or 403: Change key) or the credit or the pace (429), the network,
  the vendor's own message, nothing heard. The first time, macOS asks whether FinchGram may use the
  microphone.
- **AI vendors, then three features** (the maintainer's words of 2026-10-26: first "可以让用户添加多个
  不同的url和key组合，并且同时选一下格式，还要有一个label…然后，每个能力，选一个组合"; then "把那个connection改了，
  名字叫ai vendor", "每一个feature…应该分别有开关", "用一个下拉菜单来选，不要做任何假设和默认", and for the
  models "用户选模型，而不是手动填写，默认不选任何模型"). The user adds AI vendors, each a name of theirs,
  an API format, an address (empty: the vendor's own) and a key; a new one starts with none of them
  chosen. A vendor does everything its format can: *OpenAI-compatible* hears (realtime, or an upload
  at the end) and writes, so one OpenAI key serves both; *Alibaba Cloud Model Studio* hears
  (Qwen-ASR) and writes (Qwen, in its OpenAI-compatible mode), one key for both; *Anthropic* and
  *Gemini* write; *Doubao speech* hears. Three AI features use them, each with its own switch (off
  until the user turns it on) and its own vendor and model, chosen from dropdown lists: **voice
  input** (the tool's own switch; also the main languages, how an OpenAI-compatible vendor hears,
  the words to know), **tidying** what voice input heard, and **transcribing voice messages**.
  Nothing is chosen for the user: no vendor, no model, no way of hearing, until they choose; a
  feature lists only the vendors that can do it. The vendor list shows each one's name, format,
  host and key, and what its check said after it was last saved. "Add an AI vendor", or a click on
  one, opens a dialog with its name, format, address and key, kept together by one Save and dropped
  by Cancel or Esc (the maintainer's words of 2026-10-27: a popup, and "不是单独的一个save，是整个save");
  nothing is kept before, and typing in it never redraws the panel (the inline form of 0.7.3 and
  0.7.4 lost the focus at every letter). A key kept stays unless another is typed or it is marked
  to go; the dialog also removes the vendor, with its key, at once (the features that used it then
  have none). One panel holds them all, in that order (the maintainer's choice of a panel in three
  sections over a card each).
- **Models.** Chosen from the vendor's own list, with a search over it, never typed: OpenAI-compatible
  vendors' `GET …/models` (all of them, as listed), Anthropic's `GET /v1/models`, Gemini's
  `GET /v1beta/models` (those that generate content, by its own mark), Model Studio's catalog
  (`GET /api/v1/models`, by its own capability: `Realtime-ASR` for hearing, `TG` for writing); Doubao,
  whose API lists none, offers the four resources its documentation names. A list is asked for when
  a vendor is chosen for a feature, when the list is opened, and again after the vendor's key,
  format or address changes; it is kept for the session, and what the vendor said shows under it
  (asking, a key wanted, the key refused, its error, no models).
- **Main languages.** The user picks the language they mostly speak, or "any language", and may add
  others to switch between in the strip; the main language only tells the vendor what is spoken
  (the maintainer's answer: one vendor for each feature, not one for each language). Words of other
  languages mixed in are kept as spoken (each format is told so in its own way, below), and the main
  language fixes the script (简体 or 繁體).
- **Voice input, by its vendor's format.** With the model chosen and words to know (names and terms
  the user says often, separated by commas), which each format takes as hints:
  - *OpenAI Realtime* (`wss://…/realtime?intent=transcription`, 24 kHz): a `session.update` of type
    `transcription` with the model chosen (`gpt-live-transcribe`, say), `delay: "xhigh"` (the most
    context: the live words show later and are right more often; `delay` in settings.toml takes
    another), the main language (`languages: ["zh-cn"]` for the models of 2026, `language: "zh"`
    for the older ones, never both), a sentence of context (`prompt`: a chat message dictated,
    mainly in that language, other languages mixed in to be written as spoken), the words to know
    (`keywords`; in the context for the older models), and `turn_detection: null`. The sound goes in
    pieces of 100 ms once OpenAI answers `session.updated`; the deltas make the live words; the user
    done, `input_audio_buffer.commit`, and the turn's `…completed` transcript takes the place of its
    deltas, waited for at most 15 seconds. Less than 100 ms of sound is no turn.
  - *OpenAI-compatible upload* (`POST …/audio/transcriptions`, the format the most services, relays
    and servers of one's own speak: OpenAI, Groq, SiliconFlow (whose SenseVoice is strong in
    Chinese), Mistral, speaches, LocalAI, vLLM): the recording is kept while the user speaks and
    goes up as one WAV file at 16 kHz once they are done, with the model chosen, the language by
    ISO 639-1, and a prompt: for Chinese a line in the script wanted that leaves English
    words as they are, and the words to know. No words while the user speaks.
  - *Qwen (Alibaba Cloud)* (`wss://dashscope-intl.aliyuncs.com/api-ws/v1/realtime?model=<the model chosen>`,
    Model Studio's key as a bearer token; Beijing's host or a workspace's by the address): a
    `session.update` with `input_audio_format: "pcm"`, `sample_rate: 16000`, the language
    (`input_audio_transcription.language`), the words to know as context (`…corpus.text`), and the
    server's own detection of where sentences end, 1200 ms of silence (in its manual mode Qwen hears
    nothing until the commit, so no live words). Each sentence comes as text the server is sure of
    and a draft (`…text`: `text` + `stash`), then whole (`…completed`); the user done,
    `session.finish`, and `session.finished` ends it. Every client event carries an `event_id`.
  - *Doubao (Volcengine)* (Seed-ASR, 豆包流式语音识别, its binary protocol over a WebSocket, document of
    2026-08-06): a 4-byte header to every frame (version and size, message type and flags,
    serialization and compression), big-endian sizes; the request in JSON, then the sound in pieces
    of 200 ms, the last one flagged; each answer the whole text so far, the answer to the last
    piece the final one. Chinese, English and any language go to `bigmodel_async` with
    `enable_nonstream` (two passes: the live words, then each sentence heard again for the final
    ones); the languages Doubao names one by one go to `bigmodel_nostream` with their code
    (`ja-JP`, …), the words at the end. 繁體 is written by Doubao itself (`output_zh_variant: "tw"`);
    the words to know go as hot words (`corpus.context`). The key is the new console's
    (`X-Api-Key`); the old console's App ID and Access Token go into the key's field as
    `appid:token`. The model is the resource billed (`X-Api-Resource-Id`): one of the four
    Volcengine's documentation names (streaming recognition 1.0 and 2.0, by the hour or by
    concurrency, `volc.bigasr.sauc.*` and `volc.seedasr.sauc.*`), the one the user bought.
- **Tidying, by its vendor's format.** Its switch, then a vendor and a model:
  *OpenAI's Chat Completions* (the format nearly every vendor, relay and local server speaks; the
  `system` role every server takes; `store: false` only to OpenAI itself), *Anthropic's Messages*
  (`anthropic-version: 2023-06-01`, which relays and other vendors' Claude-format endpoints know;
  the key as `x-api-key`, and to an address not Anthropic's as a bearer token too; no
  `temperature`, which the API refuses since 2026-10-21), *Google's Gemini* (`generateContent`,
  `x-goog-api-key`). Each model is asked to think least, as its vendor names it (`tidy.rs`,
  `least_thinking`): tidying is a short job, and a model that reasons first only makes the user
  wait. In Chat Completions, `reasoning_effort` at each family's least: `minimal` for the first
  GPT-5 models, `none` from GPT-5.1 on, `low` for GPT-6.1 Sol and GPT-6 Astra (which always think),
  the o-series, gpt-oss and Grok 3 Mini, and Gemini's in Google's OpenAI-compatible mode; DeepSeek's
  `thinking: {"type": "disabled"}` (V4 thinks unless told not to; Doubao's, GLM's and Kimi's take
  the same field); Model Studio's `enable_thinking: false`. In Anthropic's format,
  `output_config.effort: "low"` from Claude 4.6 on, and the thinking off where it can be (`disabled`
  for Opus 5, Sonnet 5 and Haiku 5.5, `between_tools` for Sonnet 5.5; Opus 5.5, Fable and Mythos
  always think). In Gemini's, `thinkingBudget` 0 for 2.5 Flash and 128 for 2.5 Pro,
  `thinkingLevel` `minimal` for Flash and `low` for Pro from Gemini 3 on. A model that refuses one
  of these with a 400 that names it is asked again the next way down, the last asking nothing, and
  the way it took is kept for the session. The instructions: take out
  filler, stutters, repeats and false starts; set the punctuation; put right a word the recognition
  clearly misheard (a homophone, 在 for 再) where the context makes it certain, and only there; keep
  the speaker's words, tone and every language as spoken; write the main language's script; never
  translate, rephrase, answer or add. An answer much longer than the words is not taken, nor is an
  error, nor is anything when tidying is on with no vendor or model chosen: the words go in as heard.
- **Addresses.** Empty is the vendor's own; a pasted endpoint (`…/v1/audio/transcriptions`,
  `wss://…/api-ws/v1/realtime?model=…`) is taken as its base. Plain http goes only to this Mac or the
  local network, where a server of one's own runs; a name and password in the address are refused;
  redirects are never followed, so that a key in a header does not go to another host.
- **Keys.** One per vendor, a credential in the app's database, table `credentials`, sealed to its
  name, `voice_input.connection.<id>` (named when the vendors were called connections; no layout
  change); shown by its first and last letters. A vendor is checked when its dialog is saved: the
  list of models (Model Studio's catalog for it), and for Doubao the WebSocket's handshake under each of its four resources, any of
  which the user may have bought; the answer shows under its row. A vendor's own address wants a key; one
  of the user's own may not, and then none is sent.
- **The words right first (2026-10-26).** OpenAI's live model at `xhigh`; words to know for every
  format; Doubao's two passes; Qwen's context; the tidying that puts misheard words right; and the
  microphone's sound resampled band-limited (`audio.rs`: a windowed sinc under a Blackman window, 16
  zero crossings, below 92 % of the lower rate's half), where the averaging of before let a 10 kHz
  hiss fold into 16 kHz's band at half its loudness, right where Chinese's s, sh, x, z, c and q are
  (a test keeps it out by more than 49 dB).
- **Voice messages.** Transcribe under a voice message (the design's fifth round, B1). With the
  third feature off, it is Telegram's own transcription (`recognizeSpeech`: Telegram Premium, a few
  tries a week without it, or a group boosted for it), its words coming into the message as
  Telegram has them (`voiceNote.speech_recognition_result`, pending with the words so far, done, or
  failed), and when Telegram cannot, the bubble says that Telegram wants Premium and where the
  user's own vendor can be set. With it on, Telegram is not asked: the message goes straight to the
  vendor and model chosen for voice messages (the maintainer's answer of 2026-10-26, "直接用 AI
  vendor"): its file as it is to an OpenAI-compatible vendor's upload, or decoded by libmpv's PCM
  writer (`src/player/decode.rs`, faster than real time) to the rate of the realtime ones; the
  language found from the message itself; no tidying. With nothing chosen, the bubble says to
  choose them. A second click hides the words, a third shows them again; they are kept in memory
  for the session, nowhere else. This hands Telegram's content (someone else's voice) to a service
  of the user's choosing, which touches clause 1.5 of the API terms (see [mcp.md](mcp.md)): off by
  default, one message at the user's click, nothing stored, as the maintainer decided for the MCP
  server on 2026-10-12; built at the maintainer's word of 2026-10-26 ("语音信息转文字！可以一起做了"),
  and theirs to keep or remove.
- **The settings file.** `[voice_input]` has `enabled` (voice input's switch), `language` and
  `main_languages`, a `[[voice_input.vendors]]` for each vendor (`id`, `label`, `format`, `url`),
  then each feature's own: voice input's `vendor`, `mode` (`realtime`, `upload` or none), `model`,
  `keywords` and `delay` (only when set by hand); `tidy`, `tidy_vendor`, `tidy_model`;
  `transcribe_messages`, `messages_vendor`, `messages_model`. "" is nothing chosen. Older files
  migrate, and keep hearing and tidying as they did, with the models they used named: 0.7.0's and
  0.7.1's `languages`, `tidy`, `model` and `tidy_model` make an OpenAI vendor for voice input and
  tidying (one language chosen is the main one, several or none are any language); 0.7.2's
  per-language formats, addresses and keys become vendors, one for each address and key; 0.7.3's
  connections are the vendors (`connections` and `messages_connection` are read as `vendors` and
  `messages_vendor`), and its choices for each main language become the features' own, those of the
  main language in use, with every language's words to know (tidying on with no vendor is off;
  voice messages sent to the speech recognition are sent to its vendor, uploading). The keys move
  into the vendors at the next start (`key_from` points at the old credential until then, even across
  a save), and vendors that turn out to share a format, an address and a key become one.
- **The code.** `src/speech/mod.rs` (the state on the UI thread, the panel, the vendors and their
  models, the features and the keys), `dictation.rs` (a dictation from the first sound to the
  words), `service.rs` (formats, addresses, model lists, connections, checks), `realtime.rs`
  (OpenAI's and Qwen's sessions), `doubao.rs`, `upload.rs`, `tidy.rs`, `messages.rs` (voice
  messages), `audio.rs`; `src/player/decode.rs`; `src/telegram/voice.rs` (Transcribe);
  `ui/voice.slint`, each theme's composer and strip, `ui/pages/voice-input.slint` in each theme's
  toolbox; the screenshot scenes `voice-*` (`voice-upload`, `voice-transcripts`, `voice-no-service`,
  the panel's `voice-input-add` and `voice-input-malformed` (the dialog, empty and wrongly filled),
  `voice-input-key` and `…-key-remove` (a key kept), `voice-input-doubao` (the checks under the
  rows), `…-dialog`, `…-speech`, `…-tidy`, `voice-input-choose` (nothing chosen),
  `voice-input-relay`, `voice-input-upload` among them) and `toolbox-ai`.
- **Different from the plan.** Built without the design round. The settings are a panel of the
  toolbox, not rows of Input. The words show in the strip while the user speaks and go into the
  composer once, tidied, at the end. They go at the end of what the composer holds, not at the
  cursor (Slint does not say where its cursor is). Broadsheet has the sparkle too, which its round
  does not draw; voice input and a voice message never record at the same time. The router of step
  5 is the user's choice of main language, by hand.
- **Tried live.** OpenAI's realtime session, by the maintainer on 0.7.0 and 0.7.1: English good,
  Chinese far below WeChat's. Qwen, Doubao, the upload, Anthropic's and Gemini's tidying and the
  voice messages' own-vendor path and the model lists follow their documents of October 2026 and are tested in those
  shapes; this machine has no key for them. The decoding of an Opus voice message is tested with a
  real one.

## What it is

- Dictation. The user presses the microphone in the composer, speaks, and the words appear in
  the box; sending them is still the user's. It transcribes the user's own voice and nothing from
  Telegram, so clause 1.5 of Telegram's API terms (see [mcp.md](mcp.md)) is not touched.
  Transcribing voice messages the user received is a different thing: it is Telegram content, the
  gray zone the research report of 2026-10-06 left to the maintainer. Telegram's own transcription
  is used for it first; the user's own service only when they turn it on (above, Voice messages).
- What the maintainer asked for: all 11 languages of the UI (English, Chinese in both scripts,
  Spanish, Portuguese, German, French, Russian, Japanese, Korean, Arabic), sentences that mix
  languages, and an installer that stays small (60-some MB today). Chinese accuracy alone was
  never the measure.
- What goes where. The app records (AVAudioEngine through objc2, 16-bit PCM; no Swift needed for
  that); an engine turns the sound into words; a small text model tidies them; the words go into
  the composer at the cursor, in all three themes. `src/speech/` is the boundary: the engines are
  the steps below, and the UI knows one voice input with one state machine (idle, listening,
  finishing, tidying, failed with a reason).
- Permissions. The microphone: an `NSMicrophoneUsageDescription` in the Info.plist that
  `scripts/bundle.sh` writes, and the hardened-runtime entitlement
  `com.apple.security.device.audio-input`, in `scripts/FinchGram.entitlements` since 2026-10-25; a
  notarized app without that entitlement gets silence and no prompt. The Speech Recognition
  permission is only for step 4.
- Words amended when step 1 shipped: CLAUDE.md's "FinchGram itself calls no AI", the same sentence
  in mcp.md, and the README's "Nothing leaves your machine except to Telegram, and to GitHub for
  updates" in all 11 languages. While the user dictates, their voice goes to the engine they chose,
  and nothing else does.
- What works today without a line of code: macOS's own dictation (press fn twice) types into any
  text field, and winit implements `NSTextInputClient`, so it should type into the composer
  (untested). It is cloud or on-device as macOS decides, one language at a time.

## What was ruled out on 2026-10-21, and why

A model inside the app:

| Model | 11 languages | Mixed speech | Why not |
|---|---|---|---|
| Apple SpeechTranscriber (macOS 26, on-device) | 9: no Russian, no Arabic | no: one locale per session, no language detection | see step 4; it comes back as one engine among others |
| Whisper large-v3 (whisper.cpp) | all 99 | detects one language per utterance; foreign words usually kept | 1.1 GB model at 5-bit next to a 60 MB app |
| Qwen3-ASR 0.6B / 1.7B | 30 languages + 22 Chinese dialects, language ID | best on paper | the only C runtime (antirez/qwen-asr) is CPU-only on a Mac, 2.8 / 6.9 GiB resident, eight commits old |
| Cohere Transcribe (2B) | 14 languages, Russian unconfirmed | no claim | same size problem |
| VibeVoice-ASR (Microsoft, 9B) | 50+, mixed speech | yes | 18 GB of weights, PyTorch only, built for hour-long meetings |

The sizes are the reason: every one of them is ten to a hundred times the app. They come back in
step 3, run by a server the user installs, not shipped by FinchGram.

## Step 1: the user's own key

The user pastes an OpenAI API key; FinchGram pays nothing, ships no key and runs no server.

- Engine: OpenAI's Realtime API, the transcription session
  (`v1/realtime/transcription_sessions`), with the model `gpt-live-transcribe`, $0.017 per
  minute of audio. The words stream back while the user speaks
  (`conversation.item.input_audio_transcription.delta`); when the app commits the turn, the final
  transcript arrives (`...completed`). The session takes `languages`, an array of expected
  languages in `zh-cn` / `zh-tw` style, which separates the two Chinese scripts; `keywords`, hints
  for names and terms; `prompt`, free context; and `delay` from `minimal` to `xhigh`, trading
  speed for accuracy. It has no server-side turn detection, which suits press-to-talk: press
  opens the session and streams, release commits. Only OpenAI speaks this protocol; Groq and
  self-hosted servers come in step 3.
- Keywords come only from what the user types in settings. Contacts' names from Telegram are
  Telegram data and go to no AI without the AI access page's say (mcp.md).
- Transport: a WebSocket client, `tungstenite` in its blocking form with the rustls already in the
  tree; one thread per dictation; audio as 24 kHz 16-bit mono PCM, base64, a chunk every 100 ms or
  so; the events reach the UI thread through `upgrade_in_event_loop`, as the MCP server's do.
  Nothing async (architecture.md, Threads).
- Tidying. After the final transcript, a small text model removes the filler ("嗯", "那个",
  "um", "uh"), hesitations, repeated words and false starts, and sets the punctuation; it keeps the
  meaning, the user's own words and every language as spoken, never translates, never adds,
  returns the text alone. Through the same key: `gpt-5-nano` ($0.05 in, $0.40 out per million
  tokens) costs well under a hundredth of a cent per message; the request says `store: false`.
  A switch turns tidying off. Whether the raw words show while speaking and the tidied text replaces
  them on release, or the tidied text alone appears, is the design round's.
- The settings (built as a section of General, a panel of the toolbox since 2026-10-26): the key,
  the languages, the tidying switch. The key is a credential and lives in the app's database, sealed as the servers'
  passwords are, never in settings.toml ([architecture.md](architecture.md), The app's database).
- What it costs the user: $1.02 an hour of speech plus tidying; a typical month of chat dictation,
  one to three hours, is $1 to $3; ten hours is $10. OpenAI's audio prices have fallen every year so far.

## Step 2: a plan sold by FinchGram

Later, when there are users who would rather not paste a key: the user logs in to a FinchGram
plan instead. The maintainer's figure was $9.9 for 500 minutes, prepaid, never going negative.

- The arithmetic, at the prices of 2026-10-21. 500 minutes with `gpt-live-transcribe` cost
  $8.50, and a merchant of record takes 5 to 8 percent: nothing is left. With `gpt-transcribe`
  (the same real-time pipe, text after the commit, $0.0045 per minute) they cost $2.25; with
  `gpt-4o-transcribe` (upload after release, $0.006) $3.00. So the plan either uses the cheaper
  model, or carries fewer minutes, or costs more. Dictation apps sell at $8.49 (Superwhisper),
  $9.99 (Spokenly, cloud) and $15 a month (Wispr Flow); $50 was discussed and is far above all of
  them.
- What it needs: a relay, a small Rust service that holds the key, passes the WebSocket through to
  OpenAI, counts seconds and enforces the balance; accounts or license keys; a merchant of record
  (Paddle, Lemon Squeezy) that handles the world's sales taxes; a privacy policy and terms, since
  the user's voice passes through the maintainer's server; support and refunds. A VPS of a few
  dollars a month. It makes FinchGram, a client with no server, into a service with one.
- In the client it is the same code: the speech backend has two logins, the user's own key or the
  plan, and the plan is a different endpoint and token.
- When: once real users ask for it, not before.

## Step 3: local models, free, on Macs that can run them

FinchGram talks to a server the user runs on their own Mac, and nothing leaves the machine; no
key, no plan, no model inside FinchGram.

- Speech: a server that speaks OpenAI's `v1/audio/transcriptions` format, such as whisper.cpp's
  server, speaches or vLLM. Tidying: a server that speaks chat completions, such as Ollama, LM
  Studio or llama-server. Settings take a base URL and a model name for each.
- This is the generic path, upload after release and words a second or two later, with no live
  words: these servers do not speak the real-time protocol. The same path reaches Groq's hosted
  Whisper ($0.04 an hour for large-v3-turbo, $0.111 for large-v3) and any other OpenAI-format
  cloud, so the choice of provider is the user's.
- Engines worth running this way, from the research of 2026-10-21: Whisper large-v3 (99
  languages, 1.1 GB at 5-bit, Metal through whisper.cpp); Qwen3-ASR 0.6B / 1.7B (30 languages and
  22 Chinese dialects, language identification, Apache-2.0, through vLLM or antirez's C runtime);
  Cohere Transcribe (14 languages, Russian unconfirmed, Apache-2.0). Apple's on-device model cannot
  be reached this way (step 4). The Mac that can run them: Apple silicon with 16 GB or more for
  Whisper large-v3; the server's own docs decide.

## Step 4: the operating system's own speech recognition

Free, offline, and already on the machine. One more engine behind the boundary; its language
coverage per platform is what the router of step 5 reads.

- macOS. Since macOS 26, `SpeechAnalyzer` with `SpeechTranscriber` in the Speech framework:
  on-device, the model downloaded and kept by the system (AssetInventory). The API is Swift only,
  so it needs a small Swift file compiled by build.rs; swiftc comes with the command line tools
  Rust already needs, and CI's `macos-latest` has been macOS 26 with Xcode 26 since mid-2026. It
  needs the Microphone and Speech Recognition permissions. Verified on the maintainer's Mac on
  2026-10-21 (macOS 26.6.2): 30 locales, among them `zh_CN`, `zh_TW`, `zh_HK`, `yue_CN`, English,
  Japanese, Korean, German, French, Spanish, Italian, Portuguese; of the UI's 11 languages that is
  9, with no Russian and no Arabic. Only the English models were installed; the app would ask the
  system for `zh_CN` on first use. One locale per session, no language detection, no mixing. The
  older `DictationTranscriber` (the pre-26 model) covers 50-odd locales including `ru_RU` and
  `ar_SA`, one at a time. iOS 26 has the same API, for the day FinchGram has an iOS app.
- Android. `android.speech.SpeechRecognizer`: on-device recognition since Android 12
  (`createOnDeviceSpeechRecognizer`, API 31, `isOnDeviceRecognitionAvailable`, models fetched per
  language with `triggerModelDownload`). Android 14 (API 34) added language detection and automatic
  language switching inside one session (`EXTRA_ENABLE_LANGUAGE_DETECTION`,
  `EXTRA_ENABLE_LANGUAGE_SWITCH` with a list of allowed languages): mixed speech handled by the
  system itself. For the day FinchGram has an Android app.
- Windows. New in 2026: an on-device Speech Recognition API among the Windows AI APIs,
  `Microsoft.Windows.AI.Speech` (`SpeechRecognitionModel`, `BatchRecognition` for files,
  `StreamingRecognition` for the microphone with a `Recognized` event). Windows 11 24H2 (build
  26100) or later and Windows App SDK 2.2.2-experimental9 (June 2026) or later; on a Copilot+ PC it
  runs on the NPU with the model preinstalled, on any other PC on the CPU with the model downloaded
  through Windows Update the first time (`EnsureReadyAsync`, with the user's consent, removable at
  Settings → System → AI Components). As of Microsoft's page of 2026-10-08 it is experimental, not
  for production and not for the Store, and the page lists no languages; the press said the
  preview starts with English. Older: `Windows.Media.SpeechRecognition` (WinRT) with a handful of
  offline languages, and Windows's own voice typing (Win+H), which is Azure in the cloud, 46
  languages, and types into any text field already. FinchGram has no Windows build
  (architecture.md, Platforms); when it does, this API is step 4 there, once it leaves
  experimental and says which languages it speaks.

## Step 5: the router

The app picks the engine for each dictation; the user never picks one by hand unless they want
to.

- What it reads: the languages the user chose (one, several, or any), whether they expect mixing,
  the platform and OS version, whether a local server answers, whether a key or a plan is there,
  whether the network is, and the privacy setting "never send my voice off this Mac".
- The rules, in order:

| If | Then |
|---|---|
| the user said never off this Mac | the OS engine when it covers the language, else the local server, else the button is grey with the reason |
| mixed languages expected, or Russian or Arabic on a Mac | a local server, else the cloud (the OS engine cannot) |
| the OS engine covers the language and the user allows it | the OS engine: free, fastest to start |
| a local server answers | the local server |
| a key or a plan | the cloud |
| nothing | the button is grey, and the reason is written next to it |

- Tidying is routed the same way: the local server, the cloud, or off. On macOS 26 Apple's
  Foundation Models framework could tidy on-device (Swift only, like step 4); noted, not decided.
- The rules are visible: the listening state names the engine, settings show the order, and no
  voice goes to a cloud the user did not allow. The router comes last, when steps 1, 3 and 4
  exist; it is nothing to build before.

## The steps side by side

| Step | Needs | Costs the user | When |
|---|---|---|---|
| 1. Own key | design round, WebSocket client, microphone entitlement, settings | about $1 an hour of speech | first |
| 2. Plan | relay server, accounts, merchant of record, policy, support | $9.9 for 500 minutes, model to be chosen by the arithmetic above | when users ask |
| 3. Local server | the generic OpenAI-format path, two base URLs in settings | nothing | after 1, small |
| 4. OS engine | a Swift shim on macOS, Speech Recognition permission; platform APIs elsewhere | nothing | after 1 |
| 5. Router | 1, 3 and 4 | | last |

## Decisions of 2026-10-21

1. Voice input is the first AI feature; the measure is 11 languages and mixed speech, not Chinese
   alone.
2. No model inside the app; the installer stays small.
3. Not Apple's framework alone: no Russian or Arabic, one language at a time.
4. The user's own OpenAI key first, with the real-time transcription session and a tidying pass
   by a small model.
5. A plan later, at the price the arithmetic allows; a local server for the Macs that can; the
   operating systems' own recognizers where they exist; a router over all of it.
6. Not today.

On 2026-10-25 the maintainer asked for it, "类似微信那样" (as WeChat has it), and step 1 was built.

## Sources

- OpenAI: [realtime transcription](https://developers.openai.com/api/docs/guides/realtime-transcription),
  [gpt-live-transcribe](https://developers.openai.com/api/docs/models/gpt-live-transcribe),
  [pricing](https://developers.openai.com/api/docs/pricing).
- Apple: `SpeechTranscriber.supportedLocales` run on the maintainer's Mac, 2026-10-21.
- Android: [SpeechRecognizer](https://developer.android.com/reference/android/speech/SpeechRecognizer),
  [RecognizerIntent](https://developer.android.com/reference/android/speech/RecognizerIntent).
- Windows: [Speech Recognition with Windows AI APIs](https://learn.microsoft.com/en-us/windows/ai/apis/speech-recognition),
  [Build 2026](https://blogs.windows.com/windowsdeveloper/2026/06/02/build-2026-furthering-windows-as-the-trusted-platform-for-development/).
- Models: [Qwen3-ASR](https://github.com/QwenLM/Qwen3-ASR), [antirez/qwen-asr](https://github.com/antirez/qwen-asr),
  [Cohere Transcribe](https://the-decoder.com/cohere-releases-open-source-model-that-tops-speech-recognition-benchmarks/),
  [VibeVoice-ASR](https://labs.ai.azure.com/projects/vibevoice-asr/), [Groq pricing](https://groq.com/pricing).
- Prices of dictation apps: [Superwhisper](https://spokenly.app/blog/superwhisper-review),
  [Wispr Flow](https://spokenly.app/blog/wispr-flow-pricing).
