# MCP 服务端：AI 助手与用户的 Telegram

[English](mcp.md)

状态：2026-10-12 建成，先是服务端（v0.3.20），然后是它的页面——AI 访问（2026-10-24 之前是设置页的一节，之后是百宝箱里的面板：⌘J → AI → AI 访问），以及应用第一次连接时请用户允许的
对话框，按设计里为它们出的那一轮（"FinchGram Desktop 页面" 里的 "AI 访问"），三个主题都有。

## 这是什么

AI 助手的应用（Claude Desktop、Claude Code、Cursor，凡是会说 Model Context Protocol 的）通过正在运行的
FinchGram、用用户自己的账号，读用户的聊天并在其中行动。这是 FinchGram 做了、而 Telegram 官方应用没有的唯一一件事。
FinchGram 自身不把 Telegram 的任何东西交给 AI，不为 AI 建索引，不为 AI 存任何东西：助手是用户自己的，它只能通过下面这些工具、在用户
允许的范围内触及 Telegram。（语音输入，见 [voice-input.zh-Hans.md](voice-input.zh-Hans.md)，在用户口述时以用户自己的密钥把用户自己的声音发给
用户选的语音识别服务，不含 Telegram 的任何东西；唯一的例外是 Telegram 自己转不了时、用户要用那个服务转写的语音消息：
用户打开之前是关的，一次一条、由用户点击，字哪里都不存。）

维护者于 2026-10-12 做出这个决定时，知道 Telegram API 条款第 1.5 条禁止把平台数据用于"人工智能的开发、增强或
部署"，也知道 Telegram《内容授权服务条款》里的例外：需要"明确、知情、肯定且持续的同意……严格限于特定内容和
特定聊天"。这个设计在一个客户端能做到的范围内尽量贴近那个例外：用户打开之前一直是关的；只有用户选定的聊天；每个
应用都由用户按名字允许；不为 AI 保留任何东西；读取别人看不见；发送由用户掌控。

## 连接一个助手

1. 百宝箱 → AI → AI 访问（⌘J）：打开"允许 AI 助手连接"。选助手能看到哪些聊天（除秘密聊天外的全部，或只限选定的），以及它能写什么
   （消息、只写草稿、或什么都不写）。

2. 让助手的应用把 FinchGram 的桥当作 MCP 服务端运行，通过标准输入输出（"stdio"传输）。页面上的"连接应用"
   备好了两行可以直接复制；桥就是应用自己的可执行文件加一个词 `mcp`：

   - Claude Desktop，在 `claude_desktop_config.json` 里：

     ```json
     { "mcpServers": { "finchgram": { "command": "/Applications/FinchGram.app/Contents/MacOS/finchgram", "args": ["mcp"] } } }
     ```

   - Claude Code：`claude mcp add finchgram -- /Applications/FinchGram.app/Contents/MacOS/finchgram mcp`

3. FinchGram 不必开着：桥会在后台启动它，并等它起来。

4. 应用第一次连接时，FinchGram 的窗口会弹出来问："允许「Claude」使用你的 Telegram 账号吗？"，带着 macOS 报告的
   程序名、它的位置，以及按当前设置它能看到什么、能写什么。允许，或不允许；两种答案都会记住，列在页面的"应用"
   里，之后可以再允许，或移除它让它下次重新问。答案属于一个 Telegram 账号：第一个登录的，而且只要它还在这里登录着
   就一直是它，不管前台是哪个账号（docs/architecture.zh-Hans.md 的“多账号”）；助手操作的就是这个账号。它退出登录之后，
   前台账号接替它：所有应用都重新问一遍，选定的聊天（那是另一个账号的）也清空。

服务端关着时连上来的助手，或用户没有允许的助手，会被明确告知，用户在自己应用的 MCP 日志里看得到那句话。

手改也行：`~/Library/Application Support/FinchGram/settings.toml` 里的 `[mcp]`（`enabled`、`scope = "all" |
"chosen"`、`chats = [ids]`、`sending = "on" | "drafts" | "off"`、`account`（这些聊天属于哪个 Telegram 账号，按它的
id）），FinchGram 启动时读。允许过或拒绝过的程序在 app 自己的数据库里（docs/architecture.md），不在这个文件里。

## 工具

助手能要什么。聊天和消息按 id 来，`list_chats` 和 `search_chats` 给出 id。文字是 Telegram 的 Markdown（`**粗体**`、
`__斜体__`、`~~删除线~~`、`` `代码` ``、```` ```预格式``` ````、`[文字](url)`），由 TDLib 读成格式（parseMarkdown）。

| 工具 | 做什么 | Telegram 请求 |
|---|---|---|
| `list_chats` | 主列表里的聊天，最近活跃的在前：id、标题、类型、未读数、最后一条消息 | store |
| `search_chats` | 按标题或 @用户名在用户自己的聊天里找 | searchChats |
| `get_chat` | 一个聊天：标题、类型、用户名、成员数、未读、是否静音、是否置顶 | store，或 getChat |
| `get_messages` | 一个聊天最新的消息，新的在前，带发送者、时间、文字、链接、媒体；给定 id 之前的更早消息 | getChatHistory |
| `search_messages` | 按词找消息，在一个聊天里或在允许的聊天范围内 | searchChatMessages、searchMessages |
| `get_unread` | 有未读的聊天，各带最新的未读消息 | store、getChatHistory |
| `download_media` | 一条消息的照片或静态贴纸，直接给图；视频、GIF 或文件下载后给路径 | getMessage、getMessageProperties、downloadFile |
| `draft_message` | 在一个聊天里留草稿，由用户在任意 Telegram 应用里发送 | parseMarkdown、setChatDraftMessage |
| `send_message` | 以用户的名义立即发一条消息；等 Telegram 确认后再回答，带上消息的最终 id | parseMarkdown、sendMessage |
| `delete_message` | 删掉这个账号发过的一条消息（用户自己发的，或助手以用户名义发的），在 Telegram 允许的范围内为所有人删除：私聊随时，群里 48 小时内；别人的消息不行 | getMessage、getMessageProperties、deleteMessages |
| `mark_read` | 把消息标为已读 | viewMessages |
| `get_me` | 账号：id、名字、用户名 | store |

一开始（协议的 `instructions`）就告诉助手：读取不会标已读；`send_message` 以用户的名义立即发出，在用户要求发送时用；其他情况
都用 `draft_message`。

## 用户能控制什么

AI 访问，按设计（设计画的是设置 → AI 访问；2026-10-24 起是百宝箱里的面板）：

- **访问**：总开关。关着（默认）时，FinchGram 拒绝所有助手。关着时页面只有这个开关和末尾的那几条规则。
- **助手能看到的聊天**：所有聊天（秘密聊天永远不算），或只限用户选的；选定的列在页面上，可以移除；选聊天的面板
  （选择聊天）带搜索框，秘密聊天灰显、不能勾。
- **发消息**：可以发消息（以用户名义立即发出；也能删这个账号发过的消息）、只写草稿（助手留草稿，用户发）、或只读。
- **应用**：每个连接过的程序，按 macOS 给出的名字和上次出现的位置；当前是否连着，或上次使用时间；移除（下次重新问），或对
  被拒绝的那个点允许。macOS 说不出名字的程序（"无法识别的程序"，按进程号）只在 FinchGram 这次运行期间记住。
- **连接应用**：Claude Code 那一行和 Claude Desktop 的 JSON，里面是 FinchGram 自己的路径，可复制。
- **记录**：助手最近要过的五件事（"读取了机械键盘研究所的 20 条消息"、"被拒绝：科技早报不在所选聊天中"），带
  应用和时间；"在访达中显示记录"打开完整日志。
- **无论怎么设置**：下面这些规则，写在页面上。

无论怎么设置，始终如此：

- 秘密聊天永远不会给助手看，也不会被搜索、写入或标记。
- 读取永远不会把消息标为已读：`get_messages`、`get_unread` 和搜索都用 getChatHistory 和搜索请求，从不用
  viewMessages。只有 `mark_read` 会，而且只在被要求时。
- 阅后即焚的媒体只描述（"一张照片，阅后即焚"），从不交出，文件 id 也不给。禁止保存内容的聊天里的媒体留在
  FinchGram 里（getMessageProperties：`can_be_saved`），和媒体查看器的保存按钮同一条规则。
- 助手发出的消息不动用户自己在那个聊天里的草稿。
- 助手读到的东西不存在任何地方：不建索引，不存储，除 TDLib 自己的之外没有缓存。

## 谁可以连接

协议对 stdio 传输没有认证：它的授权规范是给 HTTP 的，而且写明 stdio 服务端"不应遵循本规范，而应从环境中获取
凭据"。FinchGram 的答案有三层：

1. **socket 是用户自己的。** FinchGram 在自己的文件夹里监听一个 Unix socket（`~/Library/Application
   Support/FinchGram/mcp.sock`，权限 0600；路径对 socket 来说太长时，用用户临时文件夹里的 `finchgram-mcp.sock`）：
   只有同一个用户的进程能打开它。不开任何端口。最多同时开 32 个连接，其中等着 `initialize` 的最多 8 个；连上两分钟还没
   初始化的会被断开，不管它说了别的什么，三十秒不读应答的也断开；`initialize` 之前只回答 initialize 和 ping。之后想安静
   多久都行，助手的 app 就是这样。桥这一边也不认错人：转发任何一行之前，它先向系统问清楚是哪个进程在 socket 上监听，
   要求它的代码签名和自己一致（开发时要求是同一个可执行文件），所以用户名下别的程序抢占了 socket 的位置，只会被说破，
   什么也收不到。
2. **系统说是谁连的，用户说它能不能。** 每个连接，FinchGram 向内核问对端进程（`LOCAL_PEERPID`），向 libproc 问它
   的程序；如果那是 FinchGram 自己的桥、或启动桥的 shell，就往父进程找，直到找到助手的应用（"Claude"，
   /Applications/Claude.app；"claude"，~/.local/share/claude/versions/2.1.306）。不取连接自报的任何东西，所以没有
   东西能冒充别的。程序第一次连接时，对话框问用户；答案按程序的代码签名记住，和 macOS 自己区分 app 的办法一样：
   Security 框架按 `anchor apple generic`（Apple 签发的证书：Developer ID、App Store 或 Apple 自己的）验证运行中进程的
   签名，答案记在签名里的 Team ID 和签名标识之下（Claude Code 是 "Q6L2SF6YDW/com.anthropic.claude-code"，Claude
   Desktop 是 "Q6L2SF6YDW/com.anthropic.claudefordesktop"），升级、换个地方安装都不变：单独安装的 Claude Code 和
   VS Code 扩展里的那个是同一个 app，允许一次就够。没有这种签名的程序（未签名、ad hoc 签名、或证书不是 Apple
   签发的；Apple 自己的程序没有 team）改按路径记：程序本身（链接解析过的路径），而不是它的 app 包，因为一个包里
   可能还装着别的程序。解释器（node、bun、deno、python、ruby、perl、php、java、osascript……）什么都不记：系统报的是
   解释器，不是它在跑的脚本，脚本只是一个参数，而且脚本能借解释器自己的开关和环境变量把参数伪装成别的；给 node 记一个
   答案，等于给任何人用 node 跑的任何脚本都记了。对话框会说明这一点，把“不允许”做成稳妥的按钮，答案只在本次运行、
   只对那个进程有效，和系统说不出名字的程序一样。记下来的答案是 app 自己数据库里的行（docs/architecture.zh-Hans.md，“app 自己的数据库”），每一行带着
   一个标签，用只有本应用能从钥匙串拿到的那把密钥、算在程序、答案和当时登录的 Telegram 账号上：别的东西写进文件的行、
   事后被改过答案的行、或者从另一个账号那时候留下又放回来的行都不算数，那个
   程序会像新来的一样再被问一次。答案只属于一个
   Telegram 账号，就是这些设置所属的那个：它退出登录、由别的账号接替时，所有答案和选定的聊天都会丢掉，一个账号
   允许的东西不会带到下一个账号。系统说不出程序名时，
   对话框会说明，并把"不允许"做成稳妥的那个按钮。这种程序的答案只在本次运行、且只对那个进程（按 pid 和启动时间）有效；
   系统连进程都说不出的，不问直接拒绝。一个连接发来超过 1 MB 的一行就断开；客户端自报的名字进日志前去掉控制字符。
3. **设置决定它能做什么**：开关、聊天范围、发送选项。进行中的调用在每个副作用之前（发送、写草稿、删除、标记已读、交出
   文件）会再读一次设置，交出读到的内容之前也会：Telegram 应答期间被用户移出范围的聊天，它的消息不会交出去；从多个聊天
   汇总来的结果按那一刻的范围过滤。改动对已经在等 Telegram 的调用也生效。这些设置存在 settings.toml 里，带着一个只有
   本应用能算出的标签（`seal`；docs/architecture.zh-Hans.md，“app 自己的数据库”）：不是 FinchGram 改的文件，比如某个获准的
   程序自己把范围改大，不算数，下次启动这些设置回到默认、服务端关闭。要改，去 AI 访问页，不要改文件。

助手不需要任何 Telegram 凭据：它通过 FinchGram 已登录的会话工作，FinchGram 的 api_id 完全不在其中。

Streamable HTTP 传输（给跑不了程序的宿主）会配一个用户从设置里复制的令牌，绑定 127.0.0.1 并检查 Origin 头，
按规范的要求。

## 怎么做的

- `src/mcp/mod.rs`：socket，一个监听线程，每个连接一个线程，日志。连接线程读一行，让协议答一行，写回去。工具的
  活交给 UI 线程（`upgrade_in_event_loop`），store 和 TDLib 在那里，结果经一个 channel 回来。没有异步运行时，
  和架构一致。
- `src/mcp/protocol.rs`：JSON-RPC 2.0 行和 MCP 的方法（initialize、ping、tools/list、tools/call），协议版本
  2025-06-18（以及之前两个）。initialize 时问后端这个程序能不能连（`admit`，会等用户的答案）；每次调用再查一遍用户
  的既定答案。用一个假后端测试。
- `src/mcp/tools.rs`：助手看到的工具（名字、说明、JSON Schema、注解），每个工具对应的带类型调用，参数检查并限定
  范围；以及调用做了什么（`Activity`），给页面的记录。
- `src/telegram/tools.rs`：每个调用在 UI 线程上对 store 和 TDLib 做什么，用户的允许范围在这里执行。页面和选聊天
  面板列的聊天也从这里来。
- `src/mcp/access.rs`：页面和对话框（ui/ai-access.slint 里的 `AiAccess`，ui/pages/ai-access.slint）：显示和修改
  设置，允许或拒绝的应用，排队等用户回答的提问，页面上的记录，谁当前连着。
- `src/mcp/peer.rs`：是谁连的。`src/mcp/bridge.rs`：`finchgram mcp`。
- `settings::Mcp`：`[mcp]` 表，含应用。
- 日志 `mcp.log`，在 socket 旁边：谁连过，调了哪个工具、哪个聊天、多少条，结果如何。从不记消息的字。超过 1 MB
  从头开始。

## 待办

- README 的 11 种语言里说清这是什么；页面的截图可以进 README 的图库。
- 127.0.0.1 上的 Streamable HTTP，带令牌，给没有 stdio 的宿主。
- Resources（`telegram://chat/<id>`），如果宿主用得上。
