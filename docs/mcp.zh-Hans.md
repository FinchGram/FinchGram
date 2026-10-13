# MCP 服务端：AI 助手与用户的 Telegram

[English](mcp.md)

状态：服务端于 2026-10-12 建成，页面还没有。设置 → AI 访问（开关、哪些聊天、助手能不能发消息、谁连着、日志）
等设计出一轮再做，第一次连接时请用户允许某个助手应用的对话框也一并等着。在此之前，服务端在
settings.toml 里打开。

## 这是什么

AI 助手的应用（Claude Desktop、Claude Code、Cursor，凡是会说 Model Context Protocol 的）通过正在运行的
FinchGram、用用户自己的账号，读用户的聊天并在其中行动。这是 FinchGram 做了、而 Telegram 官方应用没有的唯一一件事。
FinchGram 自身不调用任何 AI，不为 AI 建索引，不为 AI 存任何东西：助手是用户自己的，它只能通过下面这些工具、在用户
允许的范围内触及 Telegram。

维护者于 2026-10-12 做出这个决定时，知道 Telegram API 条款第 1.5 条禁止把平台数据用于“人工智能的开发、增强或
部署”，也知道 Telegram《内容授权服务条款》里的例外：需要“明确、知情、肯定且持续的同意……严格限于特定内容和
特定聊天”。这个设计在一个客户端能做到的范围内尽量贴近那个例外：用户打开之前一直是关的；只有用户选定的聊天；
不为 AI 保留任何东西；读取别人看不见；发送由用户掌控。

## 连接一个助手

1. 打开服务端：在 `~/Library/Application Support/FinchGram/settings.toml` 里写

   ```toml
   [mcp]
   enabled = true
   ```

   然后重新启动 FinchGram。（将来设置 → AI 访问就是这个开关。）

2. 让助手的应用把 FinchGram 的桥当作 MCP 服务端运行，通过标准输入输出（“stdio”传输）。桥就是应用自己的可执行
   文件加一个词 `mcp`：

   - Claude Desktop，在 `claude_desktop_config.json` 里：

     ```json
     { "mcpServers": { "finchgram": { "command": "/Applications/FinchGram.app/Contents/MacOS/finchgram", "args": ["mcp"] } } }
     ```

   - Claude Code：`claude mcp add finchgram -- /Applications/FinchGram.app/Contents/MacOS/finchgram mcp`

3. FinchGram 不必已经在运行：桥会在后台把它启动起来，然后等它。

服务端关着的时候连进来的助手会被告知，用户在自己应用的 MCP 日志里能看到这句话。

## 工具

助手能要什么。聊天和消息都按 id 来，id 由 `list_chats` 和 `search_chats` 给出。文字用 Telegram 的 Markdown
（`**粗体**`、`__斜体__`、`~~删除线~~`、`` `代码` ``、```` ```预格式``` ````、`[文字](url)`），由 TDLib 读成格式
（parseMarkdown）。

| 工具 | 做什么 | Telegram 请求 |
|---|---|---|
| `list_chats` | 主列表里的聊天，最近活跃的在前：id、标题、类型、未读数、最后一条消息 | store |
| `search_chats` | 在用户自己的聊天里按标题或 @用户名找 | searchChats |
| `get_chat` | 一个聊天：标题、类型、用户名、成员数、未读、免打扰、置顶 | store，或 getChat |
| `get_messages` | 一个聊天最新的消息，新的在前，带发送者、时间、文字、链接、媒体；给一个 id 可以往前翻 | getChatHistory |
| `search_messages` | 按文字找消息，在一个聊天里或在允许的聊天之间 | searchChatMessages、searchMessages |
| `get_unread` | 有未读消息的聊天，各带最新的几条未读 | store、getChatHistory |
| `download_media` | 消息的照片或静态贴纸直接给图片本身；视频、GIF 或文件下载后给出路径 | getMessage、getMessageProperties、downloadFile |
| `draft_message` | 在聊天里写一份草稿，由用户在任何 Telegram 应用里发送 | parseMarkdown、setChatDraftMessage |
| `send_message` | 以用户的名字立刻发出一条消息 | parseMarkdown、sendMessage |
| `mark_read` | 把消息标为已读 | viewMessages |
| `get_me` | 账号：id、名字、用户名 | store |

一开始（协议的 `instructions`）就告诉助手：读取不会标已读；`send_message` 不可撤回，只在用户要求发送时用；
其余都用 `draft_message`。

## 用户掌控什么

settings.toml 里的 `[mcp]`，直到设置 → AI 访问做好：

```toml
[mcp]
enabled = true          # false：服务端拒绝所有助手
scope = "chosen"        # "all"（默认）或 "chosen"：只有下面这些聊天
chats = [-1001234567890, 123456789]   # TDLib 的聊天 id（list_chats 会显示）
sending = "drafts"      # "on"（默认）：send_message 可用；"drafts"：只有 draft_message；"off"：都不行
```

还有不论设置如何都成立的：

- 秘密聊天永远不给助手看，也不搜、不写、不标。
- 读取永远不标已读：`get_messages`、`get_unread` 和搜索用的是 getChatHistory 和搜索请求，从不 viewMessages。
  只有 `mark_read` 标，而且只在被要求时。
- 自毁媒体只描述（“一张照片，会自毁”），从不交出，文件 id 也不给。限制保存内容的聊天，其媒体留在 FinchGram 里
  （getMessageProperties：`can_be_saved`），和媒体查看器的保存按钮遵循同一条规则。
- 助手发出的消息不会动用户自己在那个聊天里的草稿。
- 助手读到的东西不会留在任何地方：没有索引，没有存储，除了 TDLib 自己的缓存没有别的缓存。

## 谁可以连

协议本身对 stdio 传输没有鉴权：它的授权规范是给 HTTP 用的，并且写明 stdio 服务端“SHOULD NOT follow this
specification, and instead retrieve credentials from the environment”。FinchGram 的做法分三层：

1. **socket 是用户自己的。** FinchGram 监听自己文件夹里的一个 Unix socket
   （`~/Library/Application Support/FinchGram/mcp.sock`，权限 0600；这个路径长到 socket 放不下时，改用用户临时文件夹里的
   `finchgram-mcp.sock`）：只有同一个用户的进程能打开它。不开任何端口。
2. **由系统说出是谁连的。** 每个连接 FinchGram 都向内核问对端进程（`LOCAL_PEERPID`），向 libproc 问它的程序；
   如果那是 FinchGram 自己的桥，或启动桥的 shell，就一路看父进程，直到找到助手的应用（“Claude
   (/Applications/Claude.app)”）。连接自己说什么都不采信，所以谁也冒充不了别人。这个名字现在写进日志，将来就是
   请用户允许某个应用的对话框里显示的身份。
3. **设置决定它能做什么**：开关、聊天范围、发送档位。

助手不需要任何 Telegram 凭据：它用的是 FinchGram 已登录的会话，FinchGram 的 api_id 完全不在这件事里。

Streamable HTTP 传输（给不能运行程序的宿主）将来做的话，会带一个用户从设置里复制的令牌，只绑定 127.0.0.1，并按
规范检查 Origin 头。

## 怎么搭的

- `src/mcp/mod.rs`：socket、一个监听线程、每个连接一个线程、日志。连接线程读一行，让协议回答，再把回答写回去。
  工具的活交给 UI 线程（`upgrade_in_event_loop`），store 和 TDLib 都在那里，结果通过 channel 回来。没有 async
  运行时，和架构一致。
- `src/mcp/protocol.rs`：JSON-RPC 2.0 的行和 MCP 的方法（initialize、ping、tools/list、tools/call），协议版本
  2025-06-18（以及之前两个）。用一个假的后端测试。
- `src/mcp/tools.rs`：助手眼中的工具（名字、说明、JSON Schema、注解），以及每个调用对应的类型化调用，参数检查
  过、在边界内。
- `src/telegram/tools.rs`：每个调用对 store 和 TDLib 做的事，在 UI 线程上；用户允许的范围在这里执行。
- `src/mcp/peer.rs`：谁连的。`src/mcp/bridge.rs`：`finchgram mcp`。
- `settings::Mcp`：`[mcp]` 这张表。
- 日志，socket 旁边的 `mcp.log`：谁连了，调用了哪个工具、哪个聊天、多少条，结果如何。从不记消息的文字。超过 1 MB
  重新开始。

## 待办

- 设置 → AI 访问，以及第一次连接时请用户允许某个应用的对话框，等设计出来（三个主题都要）。然后是 README 11 个语言
  里的一节。
- 按名字批准应用（上面的第二层）并记住。
- 127.0.0.1 上带令牌的 Streamable HTTP，给没有 stdio 的宿主。
- 资源（`telegram://chat/<id>`），如果宿主用得上。
