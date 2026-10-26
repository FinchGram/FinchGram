# 语音输入：对着输入框说话，分五步

[English](voice-input.md)

状态：第 1 步（用户自己的密钥）2026-10-25 按维护者的话做了（“咱们做一下这个语音输入吧，就是语音输入，
转文字，类似微信那样”），当时只有 OpenAI；2026-10-26 向所有厂商放开：维护者说中文远不如微信语音输入，服务应该按 API
格式来、地址和密钥由用户自己定（“应该允许大家自己找vendor，而不是固定成openai”），每种主要语言一套，并且“以语音转文字的
准确度为第一”。用户自己的地址也顺带做掉了第 3 步的大半（跑在自己 Mac 或局域网上的服务器）。第 2、4、5 步没做。语音输入在
2026-10-21 定为 AI 功能里的第一项。没有出设计稿：麦克风是设计稿在每个输入框里本来就画了的那个，输入框上方的那一条按各主题画
「回复」那一条的方式画，设置项是百宝箱里的一个面板，用设置页自己的行画；以后 Claude Design 出一轮可以替换它们。2026-10-26 起
语音输入是百宝箱里的一个工具，按维护者的话（“这也是一个tool，而且不是telegram官方原生的feature”）：用户在那里打开之前是关的。
入口是设计稿在工作台工具栏里画的那个强调色、没写用途的 ✦；输入框的麦克风归语音消息，设计稿每一轮都是这样画的（「秘密聊天与
通话」那一轮）。第一节说做成了什么样；这一页其余部分是 2026-10-21 的计划和它依据的事实。

## 做成的样子（2026-10-26）

- **怎么用。** 打开之后（百宝箱 → AI → 语音输入），在聊天里按输入框里的 ✦（工作台和报纸：在麦克风旁边；终端：输入
  `/voice`，再输一次结束）。点一下开始，再点一下结束；按住说话（超过 0.6 秒），松手就结束。说话时输入框上方有一条：
  表示麦克风音量的竖条、时长、有别的主要语言时显示当前的主要语言（点一下换到下一种，并用它重新开始），以及随说随出的文字
  （最新的两行；终端里一行）；说完才出字的格式，那里会写明。按 ⏎ 或「完成」也结束；Esc 或 × 丢掉这些字。之后那一条先显示
  「正在收尾…」等最后的字，再显示「正在整理文字…」，然后文字接在输入框已有内容的后面（该有空格的文字之间加空格），光标停在
  它们后面，由用户发送。一次口述五分钟后自己结束，免得忘了关的麦克风一直花用户的钱。
- **用不了的时候。** 那一条说明原因并点出是哪个服务，能补救的给一个按钮：主要语言的语音识别还没选连接（去设置，打开那个面板）、
  没有密钥（添加密钥，也是打开面板）、地址
  FinchGram 用不了（更改地址）、不许用麦克风（打开系统设置）、服务拒绝密钥（401 或 403：更换密钥）或余额、频率不够（429）、
  网络、服务自己的报错、什么也没听到。第一次用时 macOS 会问 FinchGram 能不能用麦克风。
- **先有连接，再给每项能力选连接**（维护者 2026-10-26 定的模型：“可以让用户添加多个不同的url和key组合，并且同时选一下
  格式，还要有一个label…然后，每个能力，选一个组合”）。用户添加连接，每个连接有自己起的名字、API 格式、地址（留空：厂商
  自己的）和密钥；每项能力选一个连接和一个模型（留空：这种格式的默认）。一个连接能做它的格式能做的所有事：*OpenAI 兼容*
  能识别（边说边出字，或说完上传）也能整理，一个 OpenAI 密钥两样都用；*阿里云百炼* 能识别（Qwen-ASR）也能整理（千问，
  走它的 OpenAI 兼容模式），一个密钥两样都用；*Anthropic* 和 *Gemini* 只整理；*豆包语音* 只识别。每项能力只列出能做它的
  连接。连接列表显示每个连接的名字、格式、主机和密钥；点一下展开，改名字、格式、地址和密钥，或者移除（用到它的能力就没有
  连接了）。
- **主要语言。** 用户选自己主要说的语言，或者「任意语言」；夹在里面的其他语言照说的原样保留（每种格式用各自的方式告诉服务，
  见下），主要语言也定了文字是简体还是繁体。每种主要语言有自己的能力设置：用哪个连接识别（OpenAI 的连接还要选怎么识别：
  边说边出字，或说完再出字）、热词、用哪个连接整理。添加一种语言时从正在用的设置开始（比如中文走豆包、英文走 OpenAI：
  两个连接，每种语言各选各的）。
- **语音识别，按连接的格式。** 带模型和热词（用户常说的人名和术语，用逗号隔开），热词各格式按自己的方式当提示：
  - *OpenAI Realtime*（`wss://…/realtime?intent=transcription`，24 kHz）：`session.update`，类型 `transcription`，带模型
    （`gpt-live-transcribe`）、`delay: "xhigh"`（上下文最多：实时的字出得晚一些，但对得更多；settings.toml 里某个 profile
    的 `delay` 可以换成别的）、主要语言（2026 年的模型用 `languages: ["zh-cn"]`，老模型用 `language: "zh"`，绝不两个都发）、
    一句上下文（`prompt`：这是口述的一条聊天消息，主要用这种语言，夹杂的其他语言照说的写）、热词（`keywords`；老模型放进
    上下文里）和 `turn_detection: null`。OpenAI 回 `session.updated` 之后声音每 100 毫秒一段发出去；增量拼成实时的字；
    用户说完发 `input_audio_buffer.commit`，这一轮的 `…completed` 全文替换它的增量，最多等 15 秒。不到 100 毫秒的声音不算一轮。
  - *OpenAI 兼容上传*（`POST …/audio/transcriptions`，最多服务、中转站和自建服务器都支持的格式：OpenAI、Groq、硅基流动
    （它的 SenseVoice 中文很强）、Mistral、speaches、LocalAI、vLLM）：用户说话时先把录音留着，说完后作为一个 16 kHz 的
    WAV 文件上传，带模型（`gpt-transcribe`）、ISO 639-1 的语言和一段 prompt：中文时是一句所需字体的话，说明英文照原样写，
    再加上热词。说话时不出字。
  - *通义千问（阿里云）*（`wss://dashscope-intl.aliyuncs.com/api-ws/v1/realtime?model=qwen3-asr-flash-realtime`，百炼的
    密钥作 bearer token；北京的域名或业务空间的域名填在地址里）：`session.update` 带 `input_audio_format: "pcm"`、
    `sample_rate: 16000`、语言（`input_audio_transcription.language`）、热词作上下文（`…corpus.text`），以及由服务器自己
    判断句子在哪结束，静音 1200 毫秒（千问的手动模式在 commit 之前什么都不识别，就没有实时的字了）。每一句先以服务器确定的
    部分加一段草稿到来（`…text`：`text` + `stash`），再整句到来（`…completed`）；用户说完发 `session.finish`，
    `session.finished` 结束。客户端的每个事件都带 `event_id`。
  - *豆包（火山引擎）*（Seed-ASR，豆包流式语音识别，WebSocket 上它自己的二进制协议，文档是 2026-08-06 的）：每一帧一个
    4 字节的头（版本和头大小、消息类型和标志、序列化方式和压缩方式），大小用大端；先发 JSON 的请求，再把声音按 200 毫秒一包
    发出去，最后一包打上标记；每个回应都是到目前为止的全文，最后一包的回应就是最终结果。中文、英文和任意语言走
    `bigmodel_async` 加 `enable_nonstream`（二遍识别：先出实时的字，每句再用非流式重听一遍出最终的字）；豆包逐个列出的其他
    语言走 `bigmodel_nostream`，带它的语言代码（`ja-JP` 等），说完才出字。繁体由豆包自己转（`output_zh_variant: "tw"`）；
    热词放在 `corpus.context` 里。密钥是新版控制台的（`X-Api-Key`）；旧版控制台的 App ID 和 Access Token 以 `appid:token`
    的形式填进密钥框。模型就是计费的资源（`X-Api-Resource-Id`，默认是按小时计费的 Seed-ASR 2.0，`volc.seedasr.sauc.duration`）。
- **整理文字，按连接的格式。** 一个开关（默认开），然后是连接和模型：*OpenAI 的 Chat Completions*（几乎所有厂商、中转站和本地服务器都支持的格式；默认 `gpt-5-mini`，够聪明，能改对听错的
  字；用每个服务器都认的 `system` 角色；`store: false` 和 `reasoning_effort: "minimal"` 只发给该发的：OpenAI 自己和它的 GPT-5
  模型）、*Anthropic 的 Messages*（`claude-haiku-5-5`；`anthropic-version: 2023-06-01`，中转站和其他厂商的 Claude 格式接口都认；
  密钥放在 `x-api-key`，发往不是 Anthropic 的地址时也作 bearer token；支持的模型（Claude 4.6 及以后）带
  `output_config.effort: "low"`；不发 `temperature`，2026-10-21 起接口拒收）、*Google 的 Gemini*（`generateContent`、
  `x-goog-api-key`、`gemini-3.5-flash-lite`）。指令：去掉口头禅、结巴、重复和说错重来的部分；加标点；识别明显听错的字
  （同音字，把「再」写成「在」这类），在上下文能确定时改对，只改这种；保留说话人的原话、语气和每种语言原样；用主要语言的
  字体；绝不翻译、改写、回答或添加。回答比原话长出很多的不采用，出错也一样：字按听到的原样放进去。
- **地址。** 留空就是厂商自己的；粘贴进来的完整接口（`…/v1/audio/transcriptions`、`wss://…/api-ws/v1/realtime?model=…`）
  会当作它的 base。明文 http 只能连本机或局域网（自建服务器在那里）；地址里带用户名和密码的不收；从不跟随重定向，免得请求头
  里的密钥被带到别的主机。
- **密钥。** 每个连接一个，存在 app 数据库的 `credentials` 表里，封到它的名字上：`voice_input.connection.<id>`（表结构
  不变）；显示开头和最后几个字符，旁边是「移除」。密钥保存时、选了格式时、地址输完时都会检查一下连接：HTTP 格式读模型
  列表，实时格式做一次 WebSocket 握手（它们的服务器不一定有模型列表）；结果显示在下面。厂商自己的地址需要密钥；用户自己的
  服务器可以不要，那就不发。
- **准确度第一（2026-10-26）。** OpenAI 实时模型用 `xhigh`；每种格式都有热词；豆包二遍识别；千问的上下文；用 `gpt-5-mini`
  的整理改对听错的字；麦克风的声音做带限重采样（`audio.rs`：Blackman 窗下的加窗 sinc，16 个过零点，截止在较低采样率一半的
  92%）。原来的平均法会让 10 kHz 的嘶声以一半的响度折进 16 kHz 的频带，正好落在中文 s、sh、x、z、c、q 所在的地方（有个测试
  确认它被压下去 49 dB 以上）。
- **语音消息。** 语音消息下面的「转文字」（设计稿第五轮 B1，之前一直是灰的）：先用 Telegram 自己的转写（`recognizeSpeech`：
  Telegram Premium、没有 Premium 时每周几次、或为此助力过的群），字按 Telegram 给的进到消息里
  （`voiceNote.speech_recognition_result`：进行中带已有的字、完成、失败）；再点一下收起，再点又展开。Telegram 转不了，而用户
  在面板里打开了「用你自己的服务把语音消息转文字」（默认关）时，消息交给为语音消息选的连接（默认和当前主要语言的语音识别
  相同；单独选的 OpenAI 连接用上传）：上传格式直接传它的文件，
  实时格式先由 libmpv 的 PCM 输出（`src/player/decode.rs`，比实时快）解码成服务要的采样率；不整理。字只在这次运行的内存里，
  别处都不存。其余情况下，气泡里写明 Telegram 要 Premium，以及用户自己的服务在哪里。这等于把 Telegram 的内容（别人的声音）
  交给用户选的服务，碰到 API 条款第 1.5 条（见 [mcp.zh-Hans.md](mcp.zh-Hans.md)）：默认关、一次一条、由用户点击、什么都不存，
  和维护者 2026-10-12 为 MCP 服务器做的决定一样；按维护者 2026-10-26 的话做的（“语音信息转文字！可以一起做了”），留不留由
  维护者定。
- **设置文件。** `[voice_input]` 里有 `enabled`、`language`（正在用的主要语言）、`transcribe_messages`、
  `messages_connection` 和 `messages_model`（留空：和语音识别相同），每个连接一个 `[[voice_input.connections]]`（`id`、
  `label`、`format`、`url`），每种主要语言一个 `[[voice_input.profiles]]`：`language`、`connection`、`mode`（`realtime` 或
  `upload`）、`model`、`keywords`、`tidy`、`tidy_connection`、`tidy_model`，手动设过才有的 `delay`。旧文件会迁移：0.7.0 和
  0.7.1 的 `languages`、`tidy`、`model`、`tidy_model` 变成第一套设置和一个 OpenAI 连接（只选了一种语言的，那就是它的主要语言；
  选了几种或一种没选的，就是任意语言；等于 0.7 默认值的模型变成格式自己的默认）；0.7.2 每种语言的格式、地址和密钥变成连接，
  每个地址加密钥一个。它们的密钥在下次启动时搬进连接（在那之前 `key_from` 指着旧凭据，保存设置也不丢），格式、地址和密钥都
  一样的连接合成一个。
- **代码。** `src/speech/mod.rs`（UI 线程上的状态、面板、连接、各语言的设置和密钥）、`dictation.rs`（一次口述从第一个声音到出字）、
  `service.rs`（格式、地址、连接、检查）、`realtime.rs`（OpenAI 和千问的会话）、`doubao.rs`、`upload.rs`、`tidy.rs`、
  `messages.rs`（语音消息）、`audio.rs`；`src/player/decode.rs`；`src/telegram/voice.rs`（转文字）；`ui/voice.slint`、各主题的
  输入框和那一条、各主题百宝箱里的 `ui/pages/voice-input.slint`；截图场景 `voice-*`（其中有 `voice-upload`、
  `voice-transcripts`，面板的 `voice-input-doubao`（展开了一个连接）、`…-speech`、`…-tidy`、`voice-input-relay`、
  `voice-input-upload`）和 `toolbox-ai`。
- **和计划不一样的地方。** 没出设计稿就做了。设置是百宝箱里的一个面板，不是「输入」里的几行。说话时字显示在那一条里，说完
  整理好才一次放进输入框。字接在输入框内容的末尾，不是光标处（Slint 不告诉我们光标在哪）。报纸主题也加了 ✦，它那一轮设计稿
  里没有；语音输入和录语音消息不会同时进行。第 5 步的路由就是用户手动选主要语言。
- **实测情况。** OpenAI 的实时会话维护者在 0.7.0、0.7.1 上用过：英文好，中文远不如微信。千问、豆包、上传、Anthropic 和
  Gemini 的整理、语音消息走自己服务的那条路，照各自 2026 年 10 月的文档写，按那些格式测过；这台机器上没有它们的密钥。
  Opus 语音消息的解码用一条真的测过。

## 这是什么

- 听写。用户按下输入框里的麦克风，说话，字出现在框里；发不发仍由用户。它只转写用户自己的声音，不碰任何
  来自 Telegram 的东西，所以 Telegram API 条款第 1.5 条（见 [mcp.zh-Hans.md](mcp.zh-Hans.md)）不受影响。
  把收到的语音消息转成文字是另一回事：那是 Telegram 的内容，是 2026-10-06 调研报告留给维护者裁定的灰色
  地带。它先用 Telegram 自己的转写；用户自己的服务只在用户打开开关时才用（见上面「语音消息」）。
- 维护者的要求：界面的 11 种语言全要（英语、简繁中文、西班牙语、葡萄牙语、德语、法语、俄语、日语、韩语、
  阿拉伯语），一句话里混着说要扛得住，安装包要保持小（现在 60 多 MB）。单看中文准确率从来不是标准。
- 各管什么。app 录音（AVAudioEngine，走 objc2，16 位 PCM，这一步不需要 Swift）；一个引擎把声音变成字；
  一个小文本模型整理一下；字进到输入框的光标处，三套主题都有。`src/speech/` 是边界：引擎就是下面的几步，
  界面只知道一个语音输入和一台状态机（空闲、在听、收尾、整理中、失败并给出原因）。
- 权限。麦克风：`scripts/bundle.sh` 写的 Info.plist 里加 `NSMicrophoneUsageDescription`，再加 hardened
  runtime 的 entitlement `com.apple.security.device.audio-input`，2026-10-25 起在 `scripts/FinchGram.entitlements`；
  公证过的 app 少了这条，录到的是静音，也不弹窗。语音识别那个权限只有第 4 步用得着。
- 第 1 步上线时改过的话：CLAUDE.md 里的「FinchGram 自己不调用 AI」，mcp.md 里同一句，以及 README 11 种语言里的
  「除了 Telegram 和 GitHub 更新，什么都不离开你的机器」。用户口述时，声音去用户选的那个引擎，别的什么都不去。
- 今天一行代码不写就有的：macOS 自带听写（按两下 fn）能往任何文本框里打字，winit 实现了
  `NSTextInputClient`，所以应该能打进输入框（没实测）。走云端还是本机由 macOS 定，一次一种语言。

## 2026-10-21 排除了什么，为什么

把模型放进 app：

| 模型 | 11 种语言 | 混说 | 为什么不 |
|---|---|---|---|
| Apple SpeechTranscriber（macOS 26，本机） | 9 种：没有俄语、阿拉伯语 | 不行：一次一个 locale，不识别语种 | 见第 4 步；它作为众多引擎之一回来 |
| Whisper large-v3（whisper.cpp） | 99 种全有 | 每段话判一种主语言；夹杂的外语词通常保留 | 5 bit 量化 1.1 GB 的模型挨着 60 MB 的 app |
| Qwen3-ASR 0.6B / 1.7B | 30 种语言加 22 种中文方言，自带语种识别 | 纸面上最好 | 唯一的 C 运行时（antirez/qwen-asr）在 Mac 上只跑 CPU，常驻 2.8 / 6.9 GiB，才 8 个提交 |
| Cohere Transcribe（2B） | 14 种，俄语未确认 | 没有说法 | 同样的体积问题 |
| VibeVoice-ASR（微软，9B） | 50 多种，支持混说 | 行 | 权重 18 GB，只有 PyTorch，做的是一小时长的会议 |

原因就是体积：每一个都是 app 的十倍到百倍。它们在第 3 步回来，由用户自己装的服务跑，不由 FinchGram 发。

## 第 1 步：用户自己的密钥

用户贴一个 OpenAI API 密钥；FinchGram 不花一分钱，不带密钥，不开服务器。

- 引擎：OpenAI 的 Realtime API，转写会话（`v1/realtime/transcription_sessions`），模型
  `gpt-live-transcribe`，每分钟音频 $0.017。用户说的同时字就流回来
  （`conversation.item.input_audio_transcription.delta`）；app 提交这一轮后，最终稿到达（`...completed`）。
  会话接受 `languages`，预期语言的数组，用 `zh-cn` / `zh-tw` 这种写法，简繁直接分开；`keywords`，人名和
  术语的提示；`prompt`，自由的上下文；`delay`，从 `minimal` 到 `xhigh`，用速度换准确率。它没有服务端的
  断句检测，正合按住说：按下开会话并开始传，松开提交。只有 OpenAI 说这个协议；Groq 和自建服务在第 3 步。
- keywords 只来自用户在设置里填的东西。Telegram 里的联系人名字是 Telegram 的数据，没有 AI 访问页的允许
  不去任何 AI（mcp.md）。
- 传输：一个 WebSocket 客户端，`tungstenite` 的阻塞版，TLS 用树里已有的 rustls；每次听写一个线程；音频按
  24 kHz 16 位单声道 PCM、base64、大约每 100 毫秒一块；事件经 `upgrade_in_event_loop` 回到界面线程，和
  MCP 服务端一样。没有异步（architecture.md，线程）。
- 整理。拿到最终稿后，一个小文本模型去掉口水话（「嗯」「那个」「um」「uh」）、犹豫、重复的词和说了一半
  的开头，补上标点；保留意思、用户自己的措辞和每一种原样说出的语言，不翻译，不添加，只返回文字。走同一个
  密钥：`gpt-5-nano`（每百万 token 输入 $0.05、输出 $0.40）一条消息远不到一美分的百分之一；请求带
  `store: false`。一个开关可以关掉整理。是说话时先显示原话、松手后被整理稿替换，还是只出现整理稿，由
  设计那一轮定。
- 设置（先做成通用里单独的一节，2026-10-26 起是百宝箱里的一个面板）：密钥、语言、整理开关。密钥是凭据，放在 app 的数据库里，像服务器的
  密码一样封起来，绝不进 settings.toml（[architecture.zh-Hans.md](architecture.zh-Hans.md)，app 自己的数据库）。
- 用户的花费：每小时说话 $1.02，加整理可以忽略；聊天听写一个月典型是一到三小时，$1 到 $3；十小时 $10。
  OpenAI 的音频价格到目前为止年年在降。

## 第 2 步：FinchGram 卖的套餐

将来，有用户懒得贴密钥时：改成登录一个 FinchGram 套餐。维护者给的数是 $9.9 用 500 分钟，预付，永不透支。

- 按 2026-10-21 的价格算账。500 分钟用 `gpt-live-transcribe` 成本 $8.50，代理商户再抽 5% 到 8%：什么都不
  剩。用 `gpt-transcribe`（同一条实时管道，提交后出字，每分钟 $0.0045）成本 $2.25；用 `gpt-4o-transcribe`
  （松手后上传，每分钟 $0.006）$3.00。所以套餐要么用便宜的模型，要么少给分钟，要么卖贵些。听写软件的
  月价是 $8.49（Superwhisper）、$9.99（Spokenly，云端）和 $15（Wispr Flow）；讨论过 $50，远高于它们全部。
- 需要什么：一个中转，小的 Rust 服务，拿着密钥，把 WebSocket 原样转给 OpenAI，数秒数，管余额；账号或许可
  证密钥；一个代理商户（Paddle、Lemon Squeezy）替你处理全世界的销售税；隐私政策和服务条款，因为用户的声音
  经过维护者的服务器；客服和退款。每月几美元的 VPS。它把 FinchGram 从一个没有服务器的客户端变成一个有
  服务器的服务。
- 客户端里是同一份代码：语音后端有两种登录，用户自己的密钥或套餐，套餐只是另一个地址和令牌。
- 什么时候：真有用户要了再做，不提前。

## 第 3 步：本地模型，免费，给跑得动的 Mac

FinchGram 去连用户在自己 Mac 上跑的服务，什么都不离开机器；没有密钥，没有套餐，FinchGram 里没有模型。

- 语音：一个说 OpenAI `v1/audio/transcriptions` 格式的服务，比如 whisper.cpp 的 server、speaches 或
  vLLM。整理：一个说 chat completions 的服务，比如 Ollama、LM Studio 或 llama-server。设置里各填一个地址
  和一个模型名。
- 这就是通用路径：松手后上传，一两秒后出字，没有字随声出，因为这些服务不说实时协议。同一条路径也能连
  Groq 托管的 Whisper（large-v3-turbo 每小时 $0.04，large-v3 $0.111）和任何其他 OpenAI 格式的云端，用哪家
  由用户定。
- 值得这样跑的引擎，来自 2026-10-21 的调研：Whisper large-v3（99 种语言，5 bit 量化 1.1 GB，whisper.cpp
  走 Metal）；Qwen3-ASR 0.6B / 1.7B（30 种语言加 22 种中文方言，语种识别，Apache-2.0，经 vLLM 或 antirez 的
  C 运行时）；Cohere Transcribe（14 种语言，俄语未确认，Apache-2.0）。Apple 的本机模型走不了这条路（见第
  4 步）。跑得动的 Mac：Whisper large-v3 要 Apple 芯片加 16 GB 以上内存；以各服务自己的文档为准。

## 第 4 步：操作系统自带的语音识别

免费、离线、机器上本来就有。边界后面再多一个引擎；它在各平台覆盖哪些语言，就是第 5 步的路由要读的。

- macOS。macOS 26 起，Speech 框架里的 `SpeechAnalyzer` 配 `SpeechTranscriber`：本机识别，模型由系统下载和
  保管（AssetInventory）。API 只有 Swift，所以要一个小的 Swift 文件由 build.rs 编译；swiftc 随 Rust 本来就
  要的命令行工具来，CI 的 `macos-latest` 从 2026 年中起就是 macOS 26 配 Xcode 26。要麦克风和语音识别两个
  权限。2026-10-21 在维护者的 Mac（macOS 26.6.2）上核实：30 个 locale，其中有 `zh_CN`、`zh_TW`、`zh_HK`、
  `yue_CN`、英语、日语、韩语、德语、法语、西班牙语、意大利语、葡萄牙语；界面的 11 种语言里占 9 种，没有
  俄语和阿拉伯语。当时只装了英语模型；app 第一次用时要请系统装 `zh_CN`。一次一个 locale，不识别语种，不能
  混说。旧的 `DictationTranscriber`（26 之前的模型）覆盖 50 来个 locale，含 `ru_RU` 和 `ar_SA`，同样一次
  一种。iOS 26 是同一个 API，留给 FinchGram 有 iOS 版的那天。
- Android。`android.speech.SpeechRecognizer`：Android 12 起有本机识别（`createOnDeviceSpeechRecognizer`，
  API 31，`isOnDeviceRecognitionAvailable`，各语言模型用 `triggerModelDownload` 取）。Android 14（API 34）
  加了语种检测和一个会话内的自动语言切换（`EXTRA_ENABLE_LANGUAGE_DETECTION`、
  `EXTRA_ENABLE_LANGUAGE_SWITCH` 配允许的语言列表）：混说由系统自己处理。留给 FinchGram 有 Android 版的
  那天。
- Windows。2026 年新出：Windows AI APIs 里的本机语音识别 API，`Microsoft.Windows.AI.Speech`
  （`SpeechRecognitionModel`，`BatchRecognition` 转文件，`StreamingRecognition` 转麦克风并发 `Recognized`
  事件）。要 Windows 11 24H2（build 26100）或更新，Windows App SDK 2.2.2-experimental9（2026 年 6 月）或更新
  的实验版；Copilot+ PC 上跑 NPU、模型预装，其他 PC 上跑 CPU、第一次用时经 Windows Update 下载模型
  （`EnsureReadyAsync`，要用户同意，可在 设置 → 系统 → AI 组件 删除）。按微软 2026-10-08 那页，它还是实验
  性的，不能用于生产，不能上商店，页面也没列语言；媒体说预览版先只有英语。更老的：
  `Windows.Media.SpeechRecognition`（WinRT），离线只有几种语言；还有 Windows 自带的语音输入（Win+H），走
  Azure 云端，46 种语言，本来就能往任何文本框打字。FinchGram 还没有 Windows 版（architecture.md，平台）；
  有了以后，等这个 API 脱离实验状态、说清支持哪些语言，它就是那边的第 4 步。

## 第 5 步：路由

每次听写由 app 选引擎；用户不必自己挑，除非想挑。

- 它看什么：用户选的语言（一种、几种或任意）、是否预期混说、平台和系统版本、本地服务是否应答、有没有
  密钥或套餐、有没有网络，以及隐私设置「我的声音永远不离开这台 Mac」。
- 规则，按顺序：

| 如果 | 那么 |
|---|---|
| 用户说永远不离开这台 Mac | 系统引擎覆盖这种语言就用它，否则本地服务，再没有就按钮变灰并写明原因 |
| 预期混说，或在 Mac 上要俄语、阿拉伯语 | 本地服务，否则云端（系统引擎做不到） |
| 系统引擎覆盖这种语言且用户允许 | 系统引擎：免费，起得最快 |
| 本地服务应答 | 本地服务 |
| 有密钥或套餐 | 云端 |
| 什么都没有 | 按钮变灰，旁边写明原因 |

- 整理同样路由：本地服务、云端或关闭。macOS 26 上 Apple 的 Foundation Models 框架可以在本机整理（只有
  Swift，和第 4 步一样）；记下，没定。
- 规则看得见：正在听的状态写明用的引擎，设置里显示顺序，没有用户允许的云端就不会收到任何声音。路由
  最后做，等第 1、3、4 步都有了；之前没有什么要建的。

## 五步并排看

| 步骤 | 需要 | 用户花费 | 时间 |
|---|---|---|---|
| 1. 自己的密钥 | 设计一轮、WebSocket 客户端、麦克风 entitlement、设置项 | 每小时说话约 $1 | 最先 |
| 2. 套餐 | 中转服务器、账号、代理商户、政策、客服 | $9.9 用 500 分钟，模型按上面的算术选 | 有用户要时 |
| 3. 本地服务 | 通用的 OpenAI 格式路径，设置里两个地址 | 无 | 第 1 步之后，活不大 |
| 4. 系统引擎 | macOS 上一个 Swift 垫片和语音识别权限；别的平台各自的 API | 无 | 第 1 步之后 |
| 5. 路由 | 第 1、3、4 步 | | 最后 |

## 2026-10-21 的决定

1. 语音输入是第一个 AI 功能；标准是 11 种语言和混说，不是单看中文。
2. 模型不进 app；安装包保持小。
3. 不单靠 Apple 的框架：没有俄语、阿拉伯语，一次只能一种语言。
4. 先用用户自己的 OpenAI 密钥，走实时转写会话，再由小模型整理一遍。
5. 之后是套餐，价格按算术来；本地服务给跑得动的 Mac；有的平台用操作系统自带的识别；最后一层路由。
6. 今天不做。

2026-10-25 维护者说要做，“类似微信那样”，第 1 步做了。

## 来源

- OpenAI：[实时转写](https://developers.openai.com/api/docs/guides/realtime-transcription)、
  [gpt-live-transcribe](https://developers.openai.com/api/docs/models/gpt-live-transcribe)、
  [价格](https://developers.openai.com/api/docs/pricing)。
- Apple：2026-10-21 在维护者的 Mac 上跑 `SpeechTranscriber.supportedLocales`。
- Android：[SpeechRecognizer](https://developer.android.com/reference/android/speech/SpeechRecognizer)、
  [RecognizerIntent](https://developer.android.com/reference/android/speech/RecognizerIntent)。
- Windows：[Speech Recognition with Windows AI APIs](https://learn.microsoft.com/en-us/windows/ai/apis/speech-recognition)、
  [Build 2026](https://blogs.windows.com/windowsdeveloper/2026/06/02/build-2026-furthering-windows-as-the-trusted-platform-for-development/)。
- 模型：[Qwen3-ASR](https://github.com/QwenLM/Qwen3-ASR)、[antirez/qwen-asr](https://github.com/antirez/qwen-asr)、
  [Cohere Transcribe](https://the-decoder.com/cohere-releases-open-source-model-that-tops-speech-recognition-benchmarks/)、
  [VibeVoice-ASR](https://labs.ai.azure.com/projects/vibevoice-asr/)、[Groq 价格](https://groq.com/pricing)。
- 听写软件的价格：[Superwhisper](https://spokenly.app/blog/superwhisper-review)、
  [Wispr Flow](https://spokenly.app/blog/wispr-flow-pricing)。
