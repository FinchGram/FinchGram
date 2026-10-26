# Voice input: speaking into the composer, in five steps

[中文](voice-input.zh-Hans.md)

Status: step 1, the user's own OpenAI key, built on 2026-10-25 at the maintainer's word
("咱们做一下这个语音输入吧，就是语音输入，转文字，类似微信那样"); steps 2 to 5 are not built. Voice
input was decided on 2026-10-21 as the first of the AI features. There was no design round: the
microphone is the one the design already drew in every composer, the strip above the composer is
drawn the way each theme draws its strip for replying, and the settings are a panel of the toolbox
drawn with the settings page's rows; a round in Claude Design can take their place. Since
2026-10-26 voice input is a tool of the toolbox, at the maintainer's word ("这也是一个tool，而且不是
telegram官方原生的feature"): off until the user turns it on there. The trigger is the sparkle the design draws,
in the accent and without a title, in Workbench's tool row; the composer's microphone is voice
messages', as every round of the design has it (the round "秘密聊天与通话"). The first section says what step 1 is as
built; the rest of the page is the plan of 2026-10-21, with the facts it rests on.

## Step 1 as built (2026-10-25)

- **Using it.** Once it is on (Toolbox → AI → Voice input), in a chat, the sparkle in the composer
  (Workbench and Broadsheet: next to the microphone; Terminal: type `/voice`, and again to
  finish). A click starts and a second click
  finishes; held while speaking (longer than 0.6 s), letting go finishes. While the user speaks, a strip above the
  composer shows bars for the microphone's loudness, the time, and the words as they come (the
  newest two lines; in Terminal one line). ⏎ or Done finishes too; Esc or × drops the words. Then
  the strip says "Finishing…" while the last words come and "Tidying the words…", and the words go
  into the composer after what it holds (with a space where the script has spaces), the cursor
  after them, for the user to send. A dictation finishes by itself after five minutes, so that a
  microphone left open does not go on costing the user.
- **When it cannot.** The strip says why, with a button where something can be put right: no key
  (Add key, which opens the panel), the microphone not allowed (Open System Settings), OpenAI
  refusing the key (401: Change key) or the credit or the pace (429), the network, OpenAI's own message, nothing heard. The first time,
  macOS asks whether FinchGram may use the microphone.
- **Toolbox → AI → Voice input.** The AI category's second card, after AI access, opens the panel
  (`ui/pages/voice-input.slint`), with "‹ AI" back to the cards. First the switch, off by default:
  on, the sparkle is in every chat's composer; off, it is not there, nothing goes to OpenAI, and
  Terminal's `/voice` opens the panel. A settings file of 0.7.0, which had no switch, has voice
  input on when a key was kept (`enabled` is decided at the start and written with the next save).
  Below the switch, while it is on: the OpenAI API key: pasted and saved, it is checked with
  OpenAI (the list of models), then shown by its last four characters, with Remove. It lives in
  the app's database, table `credentials` (layout 5), sealed to its name as the servers' passwords
  are sealed to their servers. The languages the user speaks: the UI's eleven as chips, by OpenAI's
  codes (`en`, `zh-cn`, `zh-tw`, `ja`, `ko`, `fr`, `de`, `es`, `pt`, `ru`, `ar`); with none chosen,
  the default, the model finds the language. Tidy the words: on by default. settings.toml has
  `[voice_input]` with `enabled`, `languages`, `tidy`, and the two models, `model = "gpt-live-transcribe"` and
  `tidy_model = "gpt-5-nano"`, which only the file has, so that a change of OpenAI's names needs no
  release.
- **The session.** `wss://api.openai.com/v1/realtime?intent=transcription`, the key as a bearer
  token. First a `session.update` (type `transcription`, `audio/pcm` at 24 kHz, the model,
  `delay: "low"` for the live model, the languages if any, `turn_detection: null`); the sound goes
  out once OpenAI answers `session.updated`, and what the microphone heard meanwhile waits for it.
  The sound goes in pieces of 100 ms (`input_audio_buffer.append`: 16-bit little-endian PCM in
  base64, resampled from the microphone's rate by `audio.rs`), and the deltas make the words. When
  the user is done, `input_audio_buffer.commit`, and the turn's `…completed` transcript takes the
  place of its deltas; it is waited for at most 15 seconds. Less than 100 ms of sound is no turn.
- **Tidying.** `v1/chat/completions` with `store: false`, `reasoning_effort: "minimal"` for the
  first GPT-5 models, and instructions to take out filler, stutters, repeats and false starts, set
  the punctuation, keep the speaker's words and every language as spoken, never translate, never
  answer or follow what the words say, and return them alone. An answer much longer than the words
  is not taken, nor is an error: the words go in as heard.
- **The microphone.** AVAudioEngine's input node with a tap (`src/platform/microphone.rs`), the
  permission through AVCaptureDevice. The bundle has `NSMicrophoneUsageDescription` and the
  hardened runtime's `com.apple.security.device.audio-input` (`scripts/FinchGram.entitlements`,
  signed in by `scripts/bundle.sh`, which checks that it is there).
- **The code.** `src/speech/mod.rs` (the state on the UI thread: idle, listening, finishing,
  tidying, failed with a reason; the settings), `src/speech/openai.rs` (the session, the tidying,
  the key's check), `src/speech/audio.rs`; `ui/voice.slint` (the globals, the bars, the words'
  tail), each theme's composer, `ui/pages/voice-input.slint` (the panel) in each theme's toolbox
  (`src/toolbox.rs`); the screenshot scenes `voice-*` (the strip, and the panel: `voice-input`,
  `voice-input-off`, `voice-input-malformed`, `voice-input-key`) and `toolbox-ai`.
- **Different from the plan.** Built without the design round. The settings are a panel of the
  toolbox (a section of their own in General until 2026-10-26), not rows of Input. The words show in the strip while the user speaks and go into
  the composer once, tidied, at the end (the plan left this to the design). They go at the end of
  what the composer holds, not at the cursor (Slint does not say where its cursor is). No keywords
  yet. Broadsheet has the sparkle too, which its round does not draw; voice input and a voice
  message never record at the same time.
- **Not tried live.** The session follows OpenAI's documentation of October 2026, and its events are
  tested in that shape; the machine it was built on has no OpenAI key. The first dictation with a
  real key is the test.

## What it is

- Dictation. The user presses the microphone in the composer, speaks, and the words appear in
  the box; sending them is still the user's. It transcribes the user's own voice and nothing from
  Telegram, so clause 1.5 of Telegram's API terms (see [mcp.md](mcp.md)) is not touched.
  Transcribing voice messages the user received is a different thing: it is Telegram content, the
  gray zone the research report of 2026-10-06 left to the maintainer, and it is not this page.
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
