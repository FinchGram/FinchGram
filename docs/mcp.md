# The MCP server: AI assistants and the user's Telegram

[中文](mcp.zh-Hans.md)

Status: built on 2026-10-12, the server first (v0.3.20) and then its page, Settings → AI access,
and the dialog that asks the user to allow an app the first time it connects, from the design's
round for them ("AI 访问" in "FinchGram Desktop 页面"), in every theme.

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
on, only the chats the user chooses, each app allowed by the user by name, nothing kept for an AI,
reading that nobody can see, sending that the user controls.

## Connecting an assistant

1. Settings → AI access: turn on "Let AI assistants connect". Choose which chats an assistant may
   see (all but secret chats, or only chosen ones) and what it may write (messages, drafts only, or
   nothing).

2. Tell the assistant's app to run FinchGram's bridge as an MCP server over standard input and
   output (the "stdio" transport). The page's "Connect an app" has both lines ready to copy; the
   bridge is the app's own executable with the word `mcp`:

   - Claude Desktop, in `claude_desktop_config.json`:

     ```json
     { "mcpServers": { "finchgram": { "command": "/Applications/FinchGram.app/Contents/MacOS/finchgram", "args": ["mcp"] } } }
     ```

   - Claude Code: `claude mcp add finchgram -- /Applications/FinchGram.app/Contents/MacOS/finchgram mcp`

3. FinchGram need not be running: the bridge starts it, in the background, and waits for it.

4. The first time an app connects, FinchGram's window comes up with the question: "Allow “Claude”
   to use your Telegram account?", with the program as macOS reports it, where it is, and what the
   settings would let it see and write. Allow, or Don't Allow; either is remembered, under Apps on
   the page, where an app can be allowed later or removed so that it asks again. The answer is for
   one Telegram account: the first that signed in, and that one as long as it is signed in here,
   whichever account is in front (docs/architecture.md, Several accounts); assistants work with
   that account. Once it is gone (logged out), the account in front takes its place: every app asks
   again, and the chosen chats, which were the other account's, start empty.

An assistant that connects while the server is off, or that the user has not allowed, is told so,
in words the user sees in their app's MCP log.

The same, by hand: `[mcp]` in `~/Library/Application Support/FinchGram/settings.toml`
(`enabled`, `scope = "all" | "chosen"`, `chats = [ids]`, `sending = "on" | "drafts" | "off"`,
`account`, the id of the Telegram account the chats are for), read when FinchGram starts. The
programs allowed or turned away are in the app's database (docs/architecture.md), not in the file.

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
| `send_message` | A message, under the user's name, at once; answered with the message's id once Telegram has confirmed it | parseMarkdown, sendMessage |
| `delete_message` | A message the account sent (the user's own, or an assistant's under their name), deleted for everyone where Telegram allows it: any time in private chats, within 48 hours in groups; never someone else's | getMessage, getMessageProperties, deleteMessages |
| `mark_read` | Messages marked as read | viewMessages |
| `get_me` | The account: id, name, username | the store |

The assistant is told at the start (the protocol's `instructions`) that reading marks nothing as
read, that `send_message` sends under the user's name at once and is for when the user asked to send, and that
`draft_message` is for everything else.

## What the user controls

Settings → AI access, from the design:

- **Access**: the switch. Off (the default), FinchGram turns every assistant away. Off, the page
  shows only the switch and the rules at its end.
- **Chats assistants can see**: all chats (never a secret one), or only chats the user chooses,
  listed on the page with Remove, and a picker (Choose chats) with a search box, where secret
  chats are shown dimmed and cannot be ticked.
- **Writing**: send messages (at once, under the user's name; and delete messages the account
  sent), drafts only (the assistant leaves a draft, the user sends it), or read only.
- **Apps**: every program that has connected, as macOS named it and where it was last seen; connected now,
  or when it was last used; Remove (it asks again next time), or Allow for one that was turned
  away. A program macOS would not name ("Unidentified program", by its process id) is remembered
  only while FinchGram runs.
- **Connect an app**: the Claude Code line and the Claude Desktop JSON, with FinchGram's own path
  in them, to copy.
- **Log**: the last five things assistants asked for ("Read 20 messages in Keyboard Lab", "Turned
  away from Tech Morning: not a chosen chat"), by app and time, and Show log in Finder for the
  whole log.
- **Always, whatever the settings**: the rules below, said on the page.

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
   user can open it. No port is opened. At most 32 connections are open at once, and one that
   has not said `initialize` within two minutes is closed; after that it may stay quiet as long as
   it likes, as an assistant's app does.
2. **The system says who connected, and the user says whether it may.** For each connection
   FinchGram asks the kernel for the peer process (`LOCAL_PEERPID`) and libproc for its program;
   when that is FinchGram's own bridge, or a shell that started it, it looks at the parent until
   it finds the assistant's app ("Claude", /Applications/Claude.app; "claude",
   ~/.local/share/claude/versions/2.1.306). Nothing is taken from what the connection says about
   itself, so nothing can claim to be something else. The first time a program connects, the
   dialog asks the user; the answer is kept by the program's code signature, the way macOS itself
   tells apps apart: the Security framework validates the running process's signature against
   `anchor apple generic` (a certificate Apple issued: Developer ID, the App Store, or Apple's own),
   and the answer is remembered by the Team ID and the signing identifier in it
   ("Q6L2SF6YDW/com.anthropic.claude-code" for Claude Code,
   "Q6L2SF6YDW/com.anthropic.claudefordesktop" for Claude Desktop), which stay the same through
   every update and for every copy of the program: Claude Code installed on its own and the one
   inside the VS Code extension are one app, allowed once. A program without such a signature
   (unsigned, signed ad hoc, or under a certificate Apple did not issue; Apple's own programs carry
   no team) is remembered by where it is instead, its app bundle or its path. The answer is also
   for one Telegram account, the one the settings are for: when that account is gone and another
   takes its place, every answer and the chosen chats are dropped, so that nothing allowed for one
   account carries over to the next. When the system cannot
   name the program, the
   dialog says so and makes Don't Allow the safe choice. Such a program's answer lasts for the run
   and for that process only (its pid and when it started); one the system cannot place at all, not
   even its process, is turned away without a question. A line over 1 MB from a connection ends it,
   and what a client says of itself goes into the log without its control characters.
3. **Settings decide what it may do**: the switch, the chats, the sending choice. They are read
   again before each side effect of a call under way (a send, a draft, a deletion, marking read, a
   file handed over): a change takes effect on calls already waiting on Telegram.

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
  tools/call), protocol version 2025-06-18 (and the two before it). At initialize the backend is
  asked whether the program may connect (`admit`, which waits for the user's answer); each call
  checks the user's standing answer again. Tested against a made-up backend.
- `src/mcp/tools.rs`: the tools as the assistant sees them (names, descriptions, JSON Schemas,
  annotations), the typed call each becomes, its arguments checked and within bounds, and what a
  call did (`Activity`) for the page's log.
- `src/telegram/tools.rs`: what each call does with the store and TDLib, on the UI thread, and
  where what the user allows is enforced. Also the chats the page and its picker list.
- `src/mcp/access.rs`: the page and the dialog (`AiAccess` in ui/ai-access.slint,
  ui/pages/ai-access.slint): the settings shown and changed, the apps allowed or turned away,
  the questions queued for the user, the log on the page, who is connected now.
- `src/mcp/peer.rs`: who connected. `src/mcp/bridge.rs`: `finchgram mcp`.
- `settings::Mcp`: the `[mcp]` table, with the apps.
- The log, `mcp.log` next to the socket: who connected, which tool was called with which chat and
  how much, how it went. Never a message's words. Starts over past 1 MB.

## To do

- The README's section in its 11 languages says what this is; the pictures of the page could
  join the README's gallery.
- Streamable HTTP on 127.0.0.1, with a token, for hosts without stdio.
- Resources (`telegram://chat/<id>`) if hosts make use of them.
