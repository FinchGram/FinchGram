# The MCP server: AI assistants and the user's Telegram

[中文](mcp.zh-Hans.md)

Status: the server was built on 2026-10-12, without its page. Settings → AI access (the switch,
which chats, whether an assistant may send, who is connected, the log) waits for its round in the
design, and so does the dialog that asks the user to allow an assistant's app the first time it
connects. Until then the server is turned on in settings.toml.

## What it is

An AI assistant's app (Claude Desktop, Claude Code, Cursor: anything that speaks the Model Context
Protocol) reads the user's chats and acts in them through the running FinchGram, with the user's own
account. It is the one thing FinchGram does that Telegram's own apps do not. FinchGram itself calls
no AI, indexes nothing for one and stores nothing for one: the assistant is the user's, and it
reaches Telegram only through the tools below, with what the user allows.

The maintainer decided this on 2026-10-12 knowing clause 1.5 of Telegram's API terms, which forbids
using data from the platform in "the development, enhancement or deployment of artificial
intelligence", and the exception in Telegram's Terms of Service for Content Licensing, which wants
"explicit, informed, affirmative and continued consent ... strictly limited to the specific content
and chat". The design keeps as close to that exception as a client can: off until the user turns it
on, only the chats the user chooses, nothing kept for an AI, reading that nobody can see, sending
that the user controls.

## Connecting an assistant

1. Turn the server on: in `~/Library/Application Support/FinchGram/settings.toml`

   ```toml
   [mcp]
   enabled = true
   ```

   and start FinchGram again. (Settings → AI access will be the switch.)

2. Tell the assistant's app to run FinchGram's bridge as an MCP server over standard input and
   output (the "stdio" transport). The bridge is the app's own executable with the word `mcp`:

   - Claude Desktop, in `claude_desktop_config.json`:

     ```json
     { "mcpServers": { "finchgram": { "command": "/Applications/FinchGram.app/Contents/MacOS/finchgram", "args": ["mcp"] } } }
     ```

   - Claude Code: `claude mcp add finchgram -- /Applications/FinchGram.app/Contents/MacOS/finchgram mcp`

3. FinchGram need not be running: the bridge starts it, in the background, and waits for it.

An assistant that connects while the server is off is told so, in words the user sees in their
app's MCP log.

## The tools

What an assistant can ask for. Chats and messages go by their ids, which `list_chats` and
`search_chats` give. Text is Telegram's Markdown (`**bold**`, `__italic__`, `~~strikethrough~~`,
`` `code` ``, ```` ```pre``` ````, `[text](url)`); TDLib reads it into formatting (parseMarkdown).

| Tool | What it does | Telegram request |
|---|---|---|
| `list_chats` | The chats of the main list, newest activity first: id, title, kind, unread count, last message | the store |
| `search_chats` | Chats by title or @username, among the user's own | searchChats |
| `get_chat` | One chat: title, kind, username, member count, unread, muted, pinned | the store, or getChat |
| `get_messages` | A chat's newest messages, newest first, with sender, time, text, links, media; older ones before a given id | getChatHistory |
| `search_messages` | Messages by their words, in one chat or across the allowed ones | searchChatMessages, searchMessages |
| `get_unread` | The chats with unread messages, each with its newest unread ones | the store, getChatHistory |
| `download_media` | A message's photo or still sticker as the picture itself; a video, GIF or file downloaded and its path given | getMessage, getMessageProperties, downloadFile |
| `draft_message` | A draft in a chat, for the user to send from any Telegram app | parseMarkdown, setChatDraftMessage |
| `send_message` | A message, under the user's name, at once | parseMarkdown, sendMessage |
| `mark_read` | Messages marked as read | viewMessages |
| `get_me` | The account: id, name, username | the store |

The assistant is told at the start (the protocol's `instructions`) that reading marks nothing as
read, that `send_message` cannot be undone and is for when the user asked to send, and that
`draft_message` is for everything else.

## What the user controls

`[mcp]` in settings.toml, until Settings → AI access:

```toml
[mcp]
enabled = true          # false: the server turns every assistant away
scope = "chosen"        # "all" (the default) or "chosen": only the chats below
chats = [-1001234567890, 123456789]   # TDLib's chat ids (list_chats shows them)
sending = "drafts"      # "on" (the default): send_message works; "drafts": only draft_message; "off": neither
```

And always, whatever the settings:

- A secret chat is never shown to an assistant, nor searched, written to or marked.
- Reading never marks a message as read: `get_messages`, `get_unread` and the searches use
  getChatHistory and the search requests, never viewMessages. Only `mark_read` does, when asked.
- Self-destructing media is described ("a photo, self-destructing") but never handed out, and its
  file ids are kept back. The media of a chat that restricts saving its content stays in FinchGram
  (getMessageProperties: `can_be_saved`), as the media viewer's save button follows the same rule.
- A message sent by an assistant leaves the user's own draft in that chat alone.
- Nothing an assistant reads is kept anywhere: no index, no store, no cache beyond TDLib's own.

## Who may connect

The protocol has no authentication for the stdio transport: its authorization specification is
for HTTP, and says a stdio server "SHOULD NOT follow this specification, and instead retrieve
credentials from the environment". FinchGram's answer has three layers:

1. **The socket is the user's.** FinchGram listens on a Unix socket in its own folder
   (`~/Library/Application Support/FinchGram/mcp.sock`, mode 0600; when that path is too long for
   a socket, `finchgram-mcp.sock` in the user's temporary folder): only processes of the same
   user can open it. No port is opened.
2. **The system says who connected.** For each connection FinchGram asks the kernel for the peer
   process (`LOCAL_PEERPID`) and libproc for its program; when that is FinchGram's own bridge, or a
   shell that started it, it looks at the parent until it finds the assistant's app ("Claude
   (/Applications/Claude.app)"). Nothing is taken from what the connection says about itself, so
   nothing can claim to be something else. This name is in the log now, and will be what the
   dialog asking the user to allow an app shows.
3. **Settings decide what it may do**: the switch, the chats, the sending choice.

The assistant needs no Telegram credentials: it works through the session FinchGram is signed in
with, and FinchGram's api_id is nowhere in this.

A Streamable HTTP transport (for hosts that cannot run a program) would come with a token the user
copies from Settings, bound to 127.0.0.1 and checking the Origin header, as the specification asks.

## How it is built

- `src/mcp/mod.rs`: the socket, a thread listening on it and one per connection, the log. A
  connection's thread reads a line, has the protocol answer it and writes the answer back. A
  tool's work is handed to the UI thread (`upgrade_in_event_loop`), where the store and TDLib are,
  and its outcome comes back on a channel. No async runtime, as the architecture has it.
- `src/mcp/protocol.rs`: JSON-RPC 2.0 lines and MCP's methods (initialize, ping, tools/list,
  tools/call), protocol version 2025-06-18 (and the two before it). Tested against a made-up
  backend.
- `src/mcp/tools.rs`: the tools as the assistant sees them (names, descriptions, JSON Schemas,
  annotations) and the typed call each becomes, its arguments checked and within bounds.
- `src/telegram/tools.rs`: what each call does with the store and TDLib, on the UI thread, and
  where what the user allows is enforced.
- `src/mcp/peer.rs`: who connected. `src/mcp/bridge.rs`: `finchgram mcp`.
- `settings::Mcp`: the `[mcp]` table.
- The log, `mcp.log` next to the socket: who connected, which tool was called with which chat and
  how much, how it went. Never a message's words. Starts over past 1 MB.

## To do

- Settings → AI access, and the dialog that asks the user to allow an app the first time, once
  the design has them (every theme). Then the README's section in its 11 languages.
- Approving apps by name (the second layer above) and remembering them.
- Streamable HTTP on 127.0.0.1, with a token, for hosts without stdio.
- Resources (`telegram://chat/<id>`) if hosts make use of them.
