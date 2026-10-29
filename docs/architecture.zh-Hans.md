# 架构：app 是外壳，Telegram 交给 TDLib

[English](architecture.md)

状态：方向已确认（2026-09-29），照酷丸工具箱的架构来做。外壳、finchgram-tdlib 和 libmpv 已经在 macOS 上跑起来了，
登录、聊天列表、文字消息、图片和视频、发送照片视频和文件、消息操作（右键菜单：回复、编辑、拷贝、转发、举报、删除、多选），
以及新消息通知都能用了，界面是设计稿的三套主题。还没定的事列在最后。

## 为什么

- 一个 Telegram 客户端其实是两件事：一是协议和它的本地状态（MTProto、加密、聊天和消息的数据库、文件、
  和服务器保持同步），二是页面。TDLib 是 Telegram 官方的库，第一件事它全包了。FinchGram 要做的是第二件，
  再加上媒体中心。
- TDLib 是 C++ 写的。把它放进一个单独的程序里，它就不会混进 app 自己的代码：app 用普通的 `cargo build`
  就能编译，不需要 C++ 工具链，也没有 FFI；TDLib 崩了，窗口也不会跟着挂。
- FinchGram 先做 macOS，以后做 Windows 和 Linux，下面这些规则给它们留好了路。

## 思路

```
FinchGram（外壳：Rust + Slint）
│  页面 · app 的状态 · 设置 · 更新 · 通知
│  跟 Telegram 打交道只经过 src/telegram/
▼
finchgram-tdlib   可执行文件旁边的一个独立程序，就像酷丸工具箱的 ffmpeg：
                  锁定版本的 TDLib，加上一个很小的外壳程序，通过标准输入输出传递 TDLib 的 JSON
```

## 名词

- **外壳**：app 本身，包括页面、app 的状态、设置、更新。它知道聊天长什么样、用户能对它做什么，
  但不管 Telegram 的协议是怎么回事。
- **finchgram-tdlib**：由本仓库的 vendor 构建从锁定的源码编译出来的 TDLib，加上我们的外壳程序
  （`vendor/tdlib/host/main.cpp`）。只有它跟 Telegram 通信。
- **适配层**：`src/telegram/`，唯一跟 finchgram-tdlib 打交道的代码。
- **平台层**：外壳里随操作系统而不同的那几块：通知、托盘、Dock 角标、安全存储、打开链接、
  在文件管理器里显示文件、更新。

## 原则

1. 外壳从不自己讲 Telegram 的协议，也从不链接 TDLib。一切都经过 finchgram-tdlib 和适配层。
2. TDLib 保留自己的 JSON 接口，就像 FFmpeg 保留自己的命令行：请求、回复和更新都是 TDLib 的对象，
   跟它的 td_api.tl 定义的一模一样，一行一个。上面不再套我们自己的协议。
3. 一个程序跑一个 TDLib client，也就是一个账号。登录了几个账号，app 就跑几个，最多十个（跟 Telegram 官方应用一样），
   每个账号有自己的数据库目录和密钥（见下面“多账号”）。标准输入关闭时，它让 TDLib 关闭并把数据库写完，然后退出；
   退出登录之后它会自己退出。
4. 崩溃不会带垮窗口。finchgram-tdlib 意外退出时，还在等回复的请求都会失败，适配层再把它启动起来：
   TDLib 的数据库在磁盘上，所以能接着用。启动后 30 秒内就退出的，说明它本身坏了，app 会直接报错，
   而不是一遍遍地重启。
5. 版本是约定的一部分。`src/telegram/api.rs` 是照着某一个 TDLib 版本的 td_api.tl 写的，就是
   `vendor/tdlib/build.sh` 里锁定的那个。适配层的第一个请求就是问 finchgram-tdlib 的版本，
   对不上就是打包错误。这些类型是手写的，只写 FinchGram 用到的部分；TDLib 新加的字段会被忽略。
6. 文件只传路径。TDLib 把文件下载到它的文件目录，告诉我们路径，外壳从那里读。管道里不传大东西。
7. 开发规范照旧（[conventions.md](conventions.zh-Hans.md)）：所有东西都随 app 一起打包，由我们自己的 CI
   从锁定的源码构建，不用用户机器上的任何东西。
8. 平台相关的代码只放在平台层。其他全是共用的 Rust。
9. Telegram 对 API 客户端的条款是设计的一部分（core.telegram.org/api/terms）：用我们自己的 api_id；
   名字里不带 “Telegram”，也不用它的 logo；app 要说明自己用的是 Telegram API；频道里要显示官方的赞助消息；
   不干扰已读回执、正在输入、在线状态和阅后即焚；从 Telegram 拿到的任何数据都不用来训练或喂给 AI。

## 外壳怎么跟 finchgram-tdlib 通信

- 这个程序只在一个地方找：可执行文件旁边（.app 里的 `Contents/MacOS/`；`cargo run` 时是
  `target/<profile>/`，由 build.rs 从 `vendor/tdlib/bin/` 复制过去）。正式版由一个团队签名（scripts/sign.sh），
  这个程序也一起签；app 只在它带着同一团队的、完好的签名时才启动它（`src/telegram/process.rs`）：它会拿到
  Telegram 数据库的密钥（钥匙串只交给 app 本身的那把），机器上别的东西换进来的程序也会拿到。没有团队签名的
  FinchGram（`cargo run`、ad hoc 签名的包）没有可比对的东西，直接启动。
- 标准输入：请求，一行一个 JSON 对象。每个请求带一个 `"@extra"` 编号，TDLib 会把它原样放进回复里，
  回复再交给发请求时给的回调。
- 标准输出：回复和更新，一行一个。一个读取线程负责解析，只有在它的收件箱原本是空的时候才去叫醒 UI 线程，
  所以一连串更新会作为一批到达。现在还没人关心的更新直接在那里丢掉。
- 标准错误：TDLib 的日志（默认只输出错误，外壳需要时再调高）。
- `finchgram-tdlib --version` 输出 TDLib 的版本和 commit，vendor 构建会检查它。

## 线程

- UI 线程跑 Slint 的事件循环，并持有 app 的状态：TDLib 告诉我们的聊天、用户和消息，以及页面显示用的
  Slint model。状态只在这里、根据更新、成批地改，不用锁。
- 每个 finchgram-tdlib 有一个写线程和一个读线程。
- 耗时的活（解码图片、读文件）放到单独的线程里做，做完通过事件循环汇报，跟酷丸工具箱一样。
  图片在 UI 线程之外解码成 `SharedPixelBuffer`，到 UI 线程上再变成 `slint::Image`。
- app 自己的活没有 async 运行时：Telegram 的网络由 TDLib 做。唯一的例外从 2026-10-16 起是下载引擎的 HTTP 客户端
  （`src/downloads/http.rs`）：hyper 跑在它自己的 tokio 运行时上，两个叫 "http" 的线程。原因是 HTTP/2（PikPak 这类
  服务器要靠它提速：一条连接上的多个流让它一直有活干）在 Rust 里只有异步实现。引擎在传输自己的线程上阻塞等待这个运行时，
  所以 `src/downloads/` 之外没有任何异步代码，UI 线程也碰不到它。
- MCP 服务端（`src/mcp/`）有一个监听 socket 的线程，每个连接再各一个。连接线程读一行、答一行；工具的活交给 UI 线程
  （`upgrade_in_event_loop`），store、TDLib 和页面都在那里，结果通过 channel 回来。一个程序能不能连也是这样问的：
  对话框问用户的时候，连接线程等着。
- 语音输入（`src/speech/`）有麦克风的 tap，在系统的一个音频线程上跑，只把采样交出去；每次口述一个“voice”线程：到
  用户所选语音识别服务的阻塞 WebSocket（rustls 上的 tungstenite；每次读最多等 20 毫秒，声音在两次读之间发出去），或者
  说完时上传一次（ureq），之后是整理文字的请求（ureq）。用用户自己的服务转写语音消息时有一个“voice-message”线程，
  先让 libmpv 解码它的文件。它们通过事件循环向 UI 线程汇报；都不是 async。
- UI 线程上的 panic 会结束整个 app：Slint 通过 `extern "C"` 函数调进 app 的代码，panic 不能穿过这样的函数，Rust 在那里
  直接 abort，macOS 的崩溃报告里只看得到 abort()，看不到原因。所以 panic 的内容和位置先写进 settings.toml 旁边的
  `panic.log`（`src/main.rs`）：从 Dock 启动的 app 没有终端可以打印。崩溃留下什么、报告怎么读，见
  docs/crash-reports.md。store（`store.rs`）是一个 RefCell：`store::with`
  里再调 `store::with` 就是这样一个 panic，所以调用方已经持有 store 时要用的辅助函数改为接收 `&Store`（contacts::matching）。

## 页面：三套主题

设计稿有三套主题：工作台（默认）、报纸和终端。每套有自己的聊天窗口（`ui/workbench/`、`ui/broadsheet/`、
`ui/terminal/`）；登录、设置和个人资料是三套共用的页面（`ui/pages/`），颜色、字体和形状跟着当前主题
（`ui/look.slint`）。

- 主题只是页面。三套读的是同一组 global（`ui/state.slint`、`ui/telegram.slint`），调用同一组回调，由同一份
  Rust 代码填充和响应。一个功能在 Rust 里只写一次，画三遍；设计稿的规矩是先做工作台，再同步到另外两套，
  保证三套始终一致。
- 切换立即生效（设置 › 外观，或“视图”菜单），不用重启：窗口只是换了页面，别的都不变，连打开的聊天都还在。
- 浅色还是深色，是每套主题各自的选择（settings.toml 里的 `[appearance]`）：工作台和报纸默认跟随系统，终端默认深色；换回一套主题时，它还是离开时的样子。
- 设计稿里有、FinchGram 还没做的功能一律灰掉，不装作能用。

## 外壳里的代码

- `src/telegram/`：适配层。`process.rs` 负责运行那个程序，`api.rs` 是我们用到的 TDLib 类型，`mod.rs` 负责发请求、
  分发回复、重新启动和转交更新。`store.rs` 保存 TDLib 告诉我们的东西（聊天、用户、群组、文件夹、打开的聊天的消息），
  每批更新之后把页面用的 model 更新到最新，只动真正变了的行。打开的聊天只显示最新的 100 条消息，往上翻时再增加：
  页面会把每一行都排出来，其中任何一行一变就要把所有行重新量一遍，所以行数就是每条新消息的代价。
  `login.rs`、`chats.rs`、`conversation.rs`、`actions.rs`（消息操作）、`bots.rs`（机器人在聊天里除消息之外的东西，以及置顶消息条）、`reactions.rs`、`topics.rs`、`drafts.rs`、`share.rs`、`admin.rs`（慢速模式，见下；`errors.rs` 把 Telegram 的拒绝换成页面的话）、`chat_info.rs`（聊天资料面板，见下）、`manage.rs`（管理群组 / 频道，见下）、`account.rs` 和 `password.rs`（两步验证）负责页面要做的事；`accounts.rs` 给账号菜单、百宝箱和 Broadsheet 的窄栏列出登录的账号，并在它们之间切换（见下面“多账号”）；`files.rs` 负责下载文件，`avatars.rs` 负责给要显示头像的行取来聊天和联系人的头像（取到之前是首字母色块，没有头像的一直是色块），`viewer.rs` 给媒体查看器提供内容并记住它的音量（喇叭旁的音量托盘、↑ ↓ 和滚轮；settings.toml 里的 `video_volume` 和 `video_muted`，下一个视频和下次启动沿用），`rich_text.rs` 把消息里带格式的文字（粗体、链接等）转换成 Slint 的富文本。
  菜单也能做 Telegram 官方应用对聊天能做的事：拉黑或取消拉黑用户（setMessageSenderBlockList）、举报聊天（reportChat，和举报消息走同样的步骤）、
  退出群组或频道（leaveChat）、删除和某个用户或机器人的聊天（deleteChatHistory，TDLib 允许时也可以同时删掉对方那边的）；
  拉黑、退出和删除都会先问一句，退出或删除后聊天的标签页随即关闭。
  文件夹的菜单（对它的标签页点右键，Workbench 里是对树里的标题行）把里面的聊天全部标为已读：
  TDLib 把整个列表走一遍（readChatList），已读回执是真的。菜单里的"排序方式"决定所有文件夹里聊天的顺序，
  置顶的不参与，照 Telegram 的顺序留在最前：按最新消息（Telegram 自己的顺序）、按名称（按界面语言的习惯比较，
  用 macOS 自带的比较，中文按拼音、数字按大小，`platform::NameOrder`）、按未读（静音的聊天按没有未读算，
  和它灰色的角标一致）；所有文件夹共用一个选择，settings.toml 里的 `chat_order`，排序在 `store.rs` 的 chats_in。
- 通知（`notifications.rs`，设置 → 通知与声音）用的是 TDLib 自己的：用它的 `notification_group_count_max` 选项打开后，
  该不该通知由 TDLib 决定。它遵循每个聊天的免打扰和每类聊天的设置（Telegram 自己的、跟着账号走的设置），
  账号的另一台设备正在用时会稍等一下，消息在任何地方读过之后会把通知收回。外壳只负责显示 TDLib 加的通知
  （用户正看着的不显示：窗口在前台时打开着的那个聊天），收回 TDLib 去掉的通知；点通知会打开那个聊天。
- Telegram 官方应用让别人看到的，FinchGram 也一样（API 条款）：消息看到了才算已读（`conversation.rs`，viewMessages），
  用户写字时对方能看到“正在输入”（sendChatAction），窗口在前台并且有人在用时账号是在线状态（`online.rs`，
  TDLib 的 `online` 选项）。Telegram 据在线状态压下用户其他设备上的通知，TDLib 据此决定 FinchGram 的通知什么时候出。
  阅后即焚的照片和视频（看一次，或限时）在聊天里只显示模糊的小图，用户在媒体查看器里打开它时才告诉
  TDLib（`viewer.rs`，openMessageContent），之后它按 Telegram 官方应用的做法失效。查看器只在 TDLib 说内容
  可以保存时才提供保存（聊天可以限制保存）。
- 发送照片、视频和文件（`attachments.rs`，设计稿第六轮）：回形针菜单打开系统的打开面板；拖到窗口上的文件（`mod.rs`，
  winit 的 HoveredFile 和 DroppedFile，[drag-and-drop.zh-Hans.md](drag-and-drop.zh-Hans.md)）和 ⌘V 粘贴的图片或文件走同一条路。
  发出去之前先在一张卡片里显示，可以加说明文字，选项和 Telegram 官方应用一样：按照片发还是按文件发、多个合成最多十个一组的相册、
  私聊里的自毁计时器、静默发送。只有 Telegram 认作照片和视频的文件（JPEG、PNG、WebP；MP4、M4V、MOV）才按照片和视频发，
  其他一律按文件原样发。视频的时长、尺寸和封面由另一个线程上独立的 libmpv 读出来（`player/probe.rs`）。发送用的是 TDLib 的
  sendMessage 和 sendMessageAlbum；消息根据 updateFile 显示进度（`store.rs`），文件消息不论是谁发的都显示成设计稿里的卡片，
  点一下下载，或在访达中显示。能发什么由聊天的权限和我们在其中的权限决定（`store.rs`，send_rights）。说明文字的长度和文件大小
  遵守 Telegram 的限制（TDLib 的 `message_caption_length_max` 和 `is_premium` 选项）。
- 截图工具（`screenshot/`，设计稿第七轮）：输入栏里的剪刀或 ⌘⇧A（设置 › 通用 › 截图，可以改按键）先按设置决定要不要把窗口藏起来
  （默认不藏），让系统截下每个显示器（`platform/`，macOS 自带的 screencapture，前提是已允许“屏幕录制”；没允许时弹一张卡片说明去哪里允许），
  再用一个无边框窗口（`ui/shot.slint`）盖住每个显示器，显示定住的屏幕，整屏压暗。按住拖动拖出一个选区，选区内不压暗；单击则整个取下
  指针下的窗口（系统列出的窗口，没藏起来的话 FinchGram 自己的窗口也在内）。选区可以用把手和方向键调整，然后在上面画：
  方框、椭圆、箭头、画笔、文字、马赛克，三种粗细、六种颜色，可以撤销。
  状态和标注都在 Rust 里，覆盖层只负责画出来并上报指针和按键。Done 用 resvg 把标注画进图里（`screenshot/export.rs`，
  用内置字体，三套主题画出来的图一样），然后打开发送卡片，标题是 “Send Screenshot”；Copy 复制到剪贴板；Save 存到“下载”文件夹，
  文件名和 macOS 自己的截图一样。
- 在消息上任何地方点右键都会弹出它的菜单。消息里带格式的文字（Slint 的 StyledText）会把所有点击留给自己，
  所以右键是在窗口自己的事件里看到的（`mod.rs`，通过 winit），记进一个全局属性，再由指针下的那一行去要菜单。
  菜单里有哪些项，看 TDLib 说这条消息能做什么（getMessageProperties）。
- 聊天的菜单（在列表里对它点右键，`chats.rs`）可以免打扰、置顶、复制它的 ID（TDLib 的 chat id，
  机器人和脚本认的就是这个数字；账号自己的 ID 在个人资料页），以及把它加进账号的某个文件夹或从中移出。
  最后这项 TDLib 没有单独的请求：先把整个文件夹取下来（getChatFolder），改它手选的聊天，再整个发回去（editChatFolder）。
  按类别收聊天的文件夹（联系人、群组……）会把移出的聊天照样留着，所以在那种文件夹里还要把它按名字排除掉。
- 在打开的聊天里搜索（`search.rs`：头部的放大镜、Terminal 的"[search]"、⌘F）由 TDLib 查找带这些词的消息
  （searchChatMessages，每次一百条，最新的在前；回答来自 Telegram 的服务器，所以是他们按整词的搜索），再一条条跳过去，
  ⏎ 往上、⇧⏎ 往下，每条都像回复的原消息那样亮一下。离已加载消息很远的匹配不是一页页翻过去的：围绕它的那一页取代已加载
  的消息（`conversation.rs`，`History::has_newer`），往上照旧一页页加载，往下滚时再加载更新的，直到聊天末尾；从那里
  发出消息，或再次打开这个聊天，都从末尾重新开始。回复的原消息在两页之外时也这样到达。
- 全局搜索（`global_search.rs`，设计稿的 compose.js 一轮：标题栏的搜索框、⌘K、Terminal 的 /search）是盖在聊天上的一个
  窗口。什么都没输入时显示最近用过的聊天；输入了词，先列本地名字里带这些词的聊天和联系人（`contacts.rs`：TDLib 的
  getContacts，按名字、用户名匹配，输入是手机号时按号码匹配），停顿一下后再列 Telegram 服务器找到的：按用户名和标题
  找到的公开用户、群组和频道（searchPublicChats，去掉已加入的和已有的联系人）、按完整手机号找到的人
  （searchUserByPhoneNumber，只能找到允许别人用手机号找到自己的用户；谁都没有的号码可以直接添加为联系人），以及消息
  （searchMessages）。公开群组或频道先显示资料卡，带简介（getSupergroupFullInfo），从那里加入（joinChat）；人直接打开
  私聊（createPrivateChat）；消息打开它所在的聊天并定位到它。↑↓ 选择，⏎ 打开，Esc 关闭。
- 铅笔菜单（`new_chat.rs`；Terminal 的 /group、/channel、/dm、/contact）：新建群组先选成员，再填名称和头像，有成员时是
  普通群（createNewBasicGroupChat），没有成员或人数超过普通群上限时是超级群（createNewSupergroupChat，成员随后添加）；
  新建频道在第一步就建出来，和 Telegram 官方应用一样，这样第二步才能检查并设置公开链接（先按 Telegram 自己的规则，再
  checkChatUsername，然后 setSupergroupUsername）或显示邀请链接（主邀请链接，没有就 replacePrimaryChatInviteLink，旁边
  有复制按钮），第三步邀请订阅者（addChatMembers）；新建私聊是选一个联系人；添加联系人按手机号（importContacts），没注册
  的号码会说明。选好的头像（macOS 能读的图片都行，`images::read`）在聊天建出后上传（setChatPhoto，先裁成正方形、
  缩到 640 像素）。Telegram 拒绝的都显示在对话框里，
  对话框不关；新聊天里的系统消息是服务器自己发的。
- 机器人（`bots.rs`；还没有设计稿，所以各页面用自己的颜色朴素地画）：机器人消息下面的按钮（replyMarkupInlineKeyboard）
  在气泡下面按行平分宽度，按它的类型做 Telegram 官方应用做的事。callback 问机器人（getCallbackQueryAnswer，等待时按钮
  转圈），回的话作为轻提示显示，机器人要求弹窗时作为对话框显示，回链接就打开；链接按钮（↗）先确认、显示它的站点，
  和文字不是地址的链接一样；登录按钮先问 Telegram 站点要什么（getLoginUrlInfo），再问用户，同意后登录（getLoginUrl）；
  小程序在浏览器里打开（地址来自 openWebApp 或 getWebAppUrl：FinchGram 没有 web view）；switch-inline 把"@bot 词"写进
  当前聊天，或写进从转发选择器里挑的聊天；copy 复制；用户按钮打开那个人的聊天。按钮跟着消息的编辑走
  （updateMessageEdited 带着新的 markup），消息没了按钮就不显示。机器人的键盘（replyMarkupShowKeyboard）在输入框上方，
  用键盘按钮收起，机器人要求用一次就收起的用过就收起，机器人收回时（updateChatReplyMarkup）消失；按键发出它的文字，
  要手机号的先确认再发一张联系人名片，要位置、投票、选聊天的说明 FinchGram 还做不了，付款也是。输入框里只有"/"或
  "/词"时（Terminal：在命令行上，混在它自己的命令里）列出这个机器人或群里机器人的命令（botInfo 来自 getUserFullInfo
  和群的完整信息），机器人的菜单按钮在输入框工具的左边：它的小程序，或它的命令。聊天最新的置顶消息
  （getChatPinnedMessage，updateMessageIsPinned 时再问一次）是标题下面的一条，点它跳过去。保护内容的聊天
  （has_protected_content：不能转发、不能保存，Telegram 的应用截图时也不显示它）在前台时窗口不参与屏幕捕捉：
  窗口的 sharing type 设为 none（`platform::set_window_capturable`），截图和录屏里它是空白，和 macOS 对这类窗口的
  做法一样。
- 设计稿第五轮（ui/extras.slint 放三套主题共用的部分：回应条、表情选择器、圆形视频消息、名片、话题、分享面板，以及它们的文字）：
  - 表情回应（`reactions.rs`）：消息下面是回应条，每个表情一个小标签带数字，自己点过的高亮；消息旁的笑脸打开选择器，
    列的是这个聊天允许的表情，按 Telegram 给这条消息排的顺序（getMessageAvailableReactions）；双击消息用账号的默认表情
    （updateDefaultReactionType）。addMessageReaction 和 removeMessageReaction；每条消息能回应几个由 TDLib 把关
    （chatAvailableReactions.max_reaction_count：1 个，Premium 3 个，聊天另有限制时更少），选择器里写着。自定义表情的
    回应显示成它代表的 emoji（getCustomEmojiStickers，问一次），付费回应显示成星星。频道帖子也能回应。
  - 话题（`topics.rs`）：开了话题的群（supergroup.is_forum）打开先是话题列表（getForumTopics）：图标、名称、最后一条
    消息、未读数、置顶和关闭。进入话题只显示这个话题的消息（getForumTopicHistory；`History::topic_id` 把别的话题的消息
    挡在外面），标题下面是「全部话题 / 话题名」；在里面发的消息带着话题（messageTopicForum）。关闭的话题不能发消息，
    输入框的位置是一行说明。新建话题填名称、选 Telegram 的图标之一（getForumTopicDefaultIcons，createForumTopic）。
    跟着 updateForumTopicInfo 和 updateForumTopic 走，变了的话题整行重新取（getForumTopic），因为更新里没有未读数。
    话题的图标是它自定义表情对应的 emoji，综合话题是「#」。
  - 圆形视频消息（messageVideoNote）：180 px 的圆，里面是静帧、播放按钮和时长；点击就地播放，用的是和查看器同一个
    播放器（`viewer.rs`：Conversation.round-playing 指着这条消息时画面是 Viewer.frame），播放时外圈是强调色；
    会告诉 TDLib 看过了（openMessageContent）。录圆形视频消息 FinchGram 还没有：没有摄像头采集。
  - 语音消息（messageVoiceNote；`voice.rs`，来自设计稿「秘密聊天与通话」那一轮的 B1 和 B2）：气泡里有播放按钮、28 根柱子
    的波形（TDLib 的 5 位波形），波形同时就是播放进度，点一下跳到那里；还有时长、没听过时的小圆点、播放时的倍速
    （1×、1.5×、2×），以及灰掉的「转文字」；终端主题写成一行方块字符。它用一个单独的、不出画面的 mpv 播放
    （`src/player/audio.rs`），所以和媒体查看器互不抢播放器；正在播哪条、播到哪，是 Conversation 的属性，不在每一行里，
    这样播放时各行保持不变；别人发来的语音开始播放时会告诉 TDLib 听过了（openMessageContent），和 Telegram 官方应用
    一样，发送者能看到（updateMessageContentOpened 在每台设备上把它标为已听）。播完就停，换到别的聊天也停。录新的：按住
    输入框的麦克风说话，松开发送，向左拖取消，向上拖锁定（之后可以删除、停止并试听、发送）；只点一下会提示要按住。录音用
    AVAudioRecorder，存成缓存目录里的 M4A（AAC）文件（TDLib 接受 M4A 的语音消息，"audio/mp4"，不需要 Opus 编码器），
    以 inputMessageVoiceNote 发出，带上时长和录音时测到的响度做成的波形；超过一天的录音在启动时删掉。麦克风不被允许时，
    显示设计稿里的那条提示和去系统设置的入口。这一轮里没做的：第一次按麦克风时、系统问之前的那张说明卡片，以及正在播放的
    语音滚出视野时出现在标题栏下的迷你播放器。
  - 联系人名片（messageContact；`contacts.rs`）：谁、号码、是否注册了 Telegram；「发消息」打开和这个人的聊天
    （没注册的换成「邀请」，把 Telegram 的邀请语复制到剪贴板：桌面发不了短信），「添加联系人」（addContact，没注册的
    按号码 importContacts），「分享」（分享面板）。
  - 分享（`share.rs`）：帖子的「分享」或名片的「分享」打开面板：要分享的内容、能写消息的聊天的搜索（频道和开了话题
    的群不列）、可以多选、先发一句自己的话、「发送给 N 个聊天」。帖子和消息转发（forwardMessages），名片作为名片发
    （inputMessageContact）。保护内容的聊天（has_protected_content）不能从里面分享，页面会说明；Telegram 拒绝的
    （CHAT_FORWARDS_RESTRICTED、FLOOD_WAIT_N）留在面板里。
  - 草稿（`drafts.rs`）：输入框只有一个；切到别的聊天时，里面的字替离开的聊天存起来（setChatDraftMessage），
    来的聊天把自己的放回去；打字停一会儿也会存。updateChatDraftMessage 带来别的设备上写的。聊天列表里有草稿的聊天用
    草稿代替最后一条消息，前面标「草稿」（Terminal 的列表是「✎」）。
  - 百宝箱（`src/toolbox.rs`、ui/toolbox.slint、ui/<theme>/toolbox.slint；设计的第七轮）：聊天之外的东西都在这一处。Workbench
    点活动栏顶部的 logo，面板从左边滑出盖住列表和会话区，780px，其余压遮罩；Broadsheet 从窄栏的 logo 滑出，820px，排成报纸的
    索引；Terminal 不用抽屉，/tools 把清单放在窗格的位置（命令行和底栏留着；“/tools 传输”或“/tools transfers”直接跳到一类，
    再输一次同一个命令或按 Esc 关，/accounts 就是账号这一类）。三套主题都可以按 ⌘J（显示 → 百宝箱）。顶部一个搜索框按名字
    过滤（⏎ 打开找到的第一个工具），左边是分类（全部，然后账号、传输、媒体、消息、AI；toolbox.rs 里一类是一张表的一行，
    一个工具是另一张表的一行），右边是各段：账号排成一格一格（⌘1–⌘0；添加账号），然后每一类的每个工具，各带一句说明和
    正在发生的事：点卡片打开工具并关上百宝箱，能关掉的工具（AI 访问、语音输入）在卡片上有自己的开关，和它面板上的是同一个。钉到栏上
    单独一类，叫活动栏（Broadsheet 叫窄栏，Terminal 叫底栏），排在最后：一个工具一个开关，活动栏、窄栏和 Terminal 的底栏只
    显示钉住的工具，按百宝箱的顺序，三套主题共用 settings.toml 里的 `[toolbox] pinned`，默认什么都不钉（维护者 2026-10-24
    定的：钉什么由用户自己配）；正在下的文件数显示在钉住的下载按钮上。下载是一个家族：服务器能说的协议列在它下面（WebDAV、FTP、FTPS、SFTP，各带默认端口和说这种协议的服务器
    数）；协议是服务器的属性，添加服务器时选（docs/downloads.zh-Hans.md），不是单独的工具。“AI”一类里有两个工具：
    AI 访问和语音输入，各是百宝箱自己的一个面板：点它的卡片，或者它在栏上的按钮，右边直接是配置本身，上面有“‹ AI”回到这一类
    的卡片（ui/pages/ai-access.slint 的 AiPanel：开关、聊天范围、应用、日志；ui/pages/voice-input.slint 的 VoicePanel：
    开关，然后是密钥、语言和整理）。两个都不是设置页的一节，设置页只放 FinchGram 自己的偏好（AI 访问是维护者 2026-10-24
    定的，语音输入是 2026-10-26：它不是 Telegram 的功能，所以是一个工具）。设计画了而 FinchGram 还没有的工具（直播、Jellyfin、关键词隐藏、定时消息）灰着列出、不能钉；Emby
    也是，它是维护者 2026-10-24 在 Jellyfin 旁边加的，不在设计那一轮里（片库页还没设计）。底部是
    设置和我的资料（设计里灰着的“AI 设置”按钮同一天按维护者的话去掉了）。维护者 2026-10-23 定的两条照旧：截图不进百宝箱（剪刀留在输入框下面，快捷键照旧），齿轮
    不回到各栏上（设置在百宝箱、菜单栏和 ⌘, 里）。没钉住的工具有动静时（下载在进行）或别的账号有未读时（Broadsheet 除外，
    它的窄栏本来就列着）logo 上有一个点；钉住的工具的点画在它自己的图标上。
  - 聊天资料面板（`chat_info.rs`，设计的“聊天资料”一轮）：⌘I 或头部的侧栏图标（Terminal 是「[chat info]」）在消息
    右侧展开一栏，并排挤压消息、不盖住（左侧列表先让位到 220px），三套主题都有，开关状态跨聊天、跨重启记住
    （settings.toml 里的 `chat_info_open`）。开头是头像、名字和状态，然后是资料（用户的手机号、用户名和简介；
    机器人或群的描述；公开群的链接；getUserFullInfo、getSupergroupFullInfo、getBasicGroupFullInfo）、设置（通知、
    置顶、所在文件夹、标为已读：带 force_read 的 viewMessages）和操作（屏蔽、举报、退出、删除，和聊天菜单一样会先问）。
    共享内容、成员（getSupergroupMembers，前五位）和邀请链接按设计这一版灰着画出来；群主和管理员多一行「管理群组 /
    管理频道」，打开下面的管理页。行在 Rust 里生成、各主题负责画，所以 Terminal 的键盘能走：↑ ↓、⏎、行的字母、Esc。
  - 管理群组 / 频道（`manage.rs`，设计的“管理群组 / 频道”一轮，ChatManage）：「管理」那一行把资料面板换成这一页，
    还在同一栏里；返回或 Esc 换回面板。这一页三套主题只画一次（ui/chat-manage-panel.slint），各主题把自己的一组令牌
    （ManageLook，在 ui/<theme>/chat-info.slint 里）交给它，和设计稿一份标记配三套令牌的做法一样。页上有：头像（从系统的
    文件选择框选一张图，macOS 能读的都行：image crate 读不了的 HEIC、TIFF、GIF 等交给 ImageIO，并按 EXIF 方向转正
    （`images::read`）；在裁剪页里拖动、缩放后裁成正方形，缩到 640 像素，setChatPhoto；新头像立刻显示，直到 Telegram
    那边的到了；读不了的图片会提示）、名字和简介，由右上的“保存”
    一起保存（setChatTitle、setChatDescription）；公开或私有（边打用户名边查，停顿后 checkChatUsername，被占用时给三个
    可以试的；setSupergroupUsername），私有时显示主邀请链接（确认后 replacePrimaryChatInviteLink）和另建的链接，带名称、
    过期时间和人数上限（getChatInviteLinks、createChatInviteLink、revokeChatInviteLink）；成员（getSupergroupMembers，
    基础群用它的 full info；从联系人里添加，addChatMembers；行内确认后移出，setChatMemberStatus banned；设为管理员，
    给设计稿里起步的几项权限）和管理员，每人一页权限和头衔（chatAdministratorRights 每拨一下整个发一次；头衔停顿后保存；
    撤销回普通成员）；群组的设置（成员权限，setChatPermissions，折成一行；慢速模式，setChatSlowModeDelay；入群审批，
    toggleSupergroupJoinByRequest，带等待的申请数；话题，toggleSupergroupIsForum；以及第五轮的保护内容、新成员可见
    历史、隐藏成员、强力反垃圾）或频道的（签名，toggleSupergroupSignMessages；评论群，在 getSuitableDiscussionChats
    里选，setChatDiscussionGroup）。没有权限做的事画成淡的带锁（提示写缺哪一项）；Telegram 拒绝的写在被拒的那一组顶上，
    点「知道了」收起。基础群被要求做只有超级群才有的事（用户名、慢速模式……）时先升级成超级群
    （upgradeBasicGroupChatToSupergroupChat），和 Telegram 官方应用一样，对话跟着进新的聊天。
  - 慢速模式（`admin.rs`）：群开了慢速模式时，发一条之后输入区倒计时
    （supergroupFullInfo.slow_mode_delay_expires_in，以及 Telegram 拒绝时的 SLOWMODE_WAIT_N），结束前不收消息；
    管理员不受限。（设计稿第五轮那个规则与控制项的栏已由上面的管理页取代；那个栏的其余部分：文件、定时消息、单聊天的
    隐私开关，FinchGram 还没有。）
  - 这一轮没做的：付款（付款按钮会说明）、投票、位置、替机器人选聊天；手机三主题（FinchGram 没有手机版）。
- Saved Messages（和自己的聊天）在三个主题里都是聊天列表上方固定的一行，账号还没用过它也在：点这一行时由 TDLib
  建出这个聊天（createPrivateChat）。它不会在下面的文件夹里重复出现；全局搜索匹配的是列表显示的名字（Saved Messages、
  Deleted Account），不是 TDLib 的标题。
- MCP 服务端（`src/mcp/`，[mcp.zh-Hans.md](mcp.zh-Hans.md)）：AI 助手的应用（Claude Desktop、Claude Code 等）通过
  正在运行的 app、用 Model Context Protocol 读用户的聊天并在其中行动。app 监听自己文件夹里的一个 Unix socket；
  `finchgram mcp`（同一个可执行文件）是助手应用运行的 stdio 桥。`protocol.rs` 是 JSON-RPC 和 MCP 的方法，`tools.rs`
  是助手眼中的工具，`src/telegram/tools.rs` 是每个工具对 store 和 TDLib 做的事，在 UI 线程上，范围由百宝箱里的 AI 访问
  决定（默认关）。`access.rs` 就是那个页面和应用第一次连接时请用户允许的对话框（ui/ai-access.slint、
  ui/pages/ai-access.slint，三个主题都有）：开关、哪些聊天、助手能写什么、允许或拒绝过的应用、记录。读取不标已读；
  秘密聊天、自毁媒体和限制保存的聊天的媒体永远不交出去。
- 下载页面（`src/downloads/`，[downloads.zh-Hans.md](downloads.zh-Hans.md)）：WebDAV、FTP、FTPS 或 SFTP 客户端，对着
  NAS、seedbox、PikPak、Alist/OpenList 或 `vget serve` 共享的文件夹，默认只读，用户为某台服务器允许改动（表单里的开关）后
  也能在那台上删除、移动、重命名、上传。`webdav.rs` 列目录（PROPFIND），`transfer.rs` 把一个文件
  切成 8 MB 的块、最多 12 条连接并发取进 `.part` 文件，停了接着来，照 vget 的做法；`mod.rs` 是 UI 线程上的页面状态、
  一个负责列目录的“servers”线程、每个在传的文件一个线程（同时两个），以及每批改动一个“changes”线程。密码进 macOS 钥匙串，服务器和下载目录进
  settings.toml；下好的照片或视频在媒体查看器里打开（`viewer.rs` 的 `view_file`）。这个页面不需要账号。
- 语音输入（`src/speech/`，[voice-input.zh-Hans.md](voice-input.zh-Hans.md)）：百宝箱“AI”一类里的一个工具，用户打开之前是关的；
  打开时输入框里有 ✦（终端里是 `/voice`，关着的时候它打开语音输入的面板）。用户添加 AI 服务商（名字、API 格式、地址、
  密钥），任何支持其中一种格式的厂商、中转站或用户自己的服务器都行；三个功能各有自己的开关，服务商和模型都从列表里选（模型
  是服务商自己列出的），不替用户选任何东西：语音输入、整理文字、语音消息转文字。`mod.rs` 是 UI 线程上一次口述的状态
  （空闲、在听、收尾、整理、或带原因的失败）、那一条里的文字、最后放进输入框的文字、服务商和它们的模型、密钥，以及百宝箱里
  语音输入的面板；`dictation.rs` 是一次口述从第一个声音到出字；`service.rs` 是格式、地址、模型列表和连接；`realtime.rs` 是 OpenAI 和千问的
  实时会话，`doubao.rs` 是豆包的二进制协议，`upload.rs` 是 OpenAI 的 `/audio/transcriptions`，`tidy.rs` 是三种格式的整理，
  `messages.rs` 是用用户自己的服务转写语音消息；`audio.rs` 把麦克风的声音带限重采样成服务要的采样率的 16 位 PCM。
  麦克风本身归平台层（`src/platform/microphone.rs`，AVAudioEngine）；密钥是 app 数据库里的凭据（见下），其余是
  settings.toml 里的 `[voice_input]`（0.7 的文件变成第一套）。语音消息下面的“转文字”在 `src/telegram/voice.rs`：先用
  Telegram 自己的转写，用户允许时才用用户自己的服务。
- `src/player/`：用 libmpv 播放视频，画进窗口（媒体查看器的播放器）；语音消息用另一个不出画面的 mpv 播放（`audio.rs`）。
- `src/platform/`：平台层。目前有：账号的会话列表里怎么称呼这台设备、macOS 上的透明标题栏、打开链接、剪贴板（文字和图片）；以及 macOS 上
  （设置 → 通用）关闭窗口后留在 Dock 里，即设计稿的“关闭窗口时：最小化到托盘”（点 Dock 图标重新打开窗口，退出时先让
  TDLib 关好）、菜单栏图标、开机时启动（SMAppService，macOS 13 及以上）；通过系统的 UserNotifications 框架发通知
  （只有打包成 .app 才有，`cargo run` 时没有）、Dock 图标上的未读数，以及让 Dock 图标跳一下（设置 → 通知与声音）；
  下载页面用的选目录面板和钥匙串（WebDAV 服务器的密码），以及麦克风和能不能用它的那一问：语音输入用 AVAudioEngine
  的 tap，语音消息用 AVAudioRecorder（`microphone.rs`）；选一张图片的面板（什么格式都行），以及 image crate 读不了的图片
  （HEIC、TIFF、GIF、AVIF 等）交给 ImageIO 读，并按拍摄时的方向转正。
- `src/update.rs`：自动更新（[conventions.md](conventions.zh-Hans.md) 第 3 节）。
- `src/settings.rs`、`src/i18n.rs`、`src/fonts.rs`（界面字体，编译进可执行文件）。
- `src/images.rs`：图片（照片、视频封面、聊天和联系人的头像、消息里自带的小预览图）在界面线程之外解码，并缓存最近的；
  照片按 EXIF 方向转正，image crate 读不了的交给平台层读（`read`）。文件头声称的像素超过 256 MiB 的图片，两边都不解码：
  几十个字节的文件头就能要走几个 GB 的内存。
- `src/screenshots.rs`：用 Slint 的软件渲染器和假数据，把每套主题的每个页面（浅色和深色）画成 PNG
  （`cargo test screenshots -- --ignored`）。
- `src/telegram/timing.rs`：用同样的方式、很多假聊天和假消息，量页面跟上 store 要多久
  （`cargo test --release refresh_timing -- --ignored --nocapture`）。
- `ui/`：`app.slint`（窗口：菜单，以及显示哪个页面）、`state.slint`、`telegram.slint` 和 `downloads.slint`（Rust 和页面共用的 global）、
  `look.slint`、`format.slint`（按界面语言写出的日期、数量和消息类型）、`widgets.slint` 和 `chat.slint`（共用的部件）、
  `viewer.slint`（覆盖整个窗口的媒体查看器）、`voice.slint`（语音输入的状态和文字）、`pages/`，以及三套主题各自的目录。

## 多账号

最多可以同时登录十个账号（`src/telegram/mod.rs`、`accounts.rs`），跟 Telegram 官方应用一样。每个账号有自己的
finchgram-tdlib，一直跑着，所以每个账号的消息都会到、都会通知；其中一个在前台，页面显示的就是它，其他账号的聊天、
标签页和草稿都留着，等它回到前台。

- **槽位。** 账号在磁盘上的位置叫槽位，1 … 10：槽位 1 用 `tdlib` 和 `media_cache`，槽位 2 用 `tdlib-2` 和
  `media_cache-2`，以此类推，钥匙串里的密钥名跟数据库目录同名（"tdlib"、"tdlib-2"）。启动时，凡是有目录的槽位都会
  启动；前台账号是上次运行时在前台的那个（settings.toml 里的 `account`）。“添加账号”占用第一个空槽位，并显示它的
  登录页；没登录完就离开的账号会被丢掉，启动时发现是空的槽位（而且还有别的账号）也会被丢掉。还有别的账号时退出
  登录，这个账号就连目录一起删掉（TDLib 已经清空了它们）；只剩一个账号时，它的槽位照旧留着给下次登录。
- **上下文。** 一段代码指的是哪个账号，由上下文决定（`telegram::context`）：正在处理某个账号的 finchgram-tdlib
  发来的一批消息时，`send` 和 `store::with` 就是这个账号的；其他时候（点击、定时器）就是前台账号的。账号自己
  稍后要做的事（它设的定时器、UI 线程之外解码的头像）用 `in_context` 带着自己的账号。页面只跟着前台账号：
  对别的账号，`store::refresh` 什么都不做，它的 store 把脏标记留着，等它回到前台，把它的 model 交给页面，
  全部重建一遍。store 之外按账号区分的东西（哪些聊天列表已加载、联系人、等待中的下载、未读数、登录到哪一步）
  按槽位做键，或者跟账号一起放在 `mod.rs` 里。
- **切换。** 在账号菜单、百宝箱或 Broadsheet 的窄栏里点一个账号；按 ⌘1 … ⌘0（“账号”菜单；Terminal 里也可以
  ⌃1 … ⌃0，它的分栏上写着）；或者点别的账号的通知。离开的账号把输入框里的字留给它的聊天（`drafts.rs`），
  关掉自己的菜单、对话框和查看器，并下线；进来的账号显示自己的 store，还没登录的显示登录到哪一步，窗口在用时上线。
- **通知和 Dock。** 每个账号都会通知。通知的名字以账号的槽位开头，点它会把那个账号带到前台并打开聊天。Dock 上的
  数字是所有账号的未读加起来；某个不在前台的账号有未读时，活动栏和窄栏的 logo 上有一个点。
- **AI 访问。** MCP 服务端的设置（[mcp.zh-Hans.md](mcp.zh-Hans.md)）属于一个账号：第一个登录的，而且只要它还在
  这里登录着就一直是它，不管前台是谁；助手操作的就是这个账号。它退出登录之后，前台账号接替它，允许过的应用要
  重新问一遍，跟单账号时换账号登录一样。
- **还没做。** 每个账号一个窗口。

## app 自己的数据库

FinchGram 自己要留的、既不是偏好设置也不是 Telegram 的东西，进它自己的数据库：settings.toml 旁边的 `finchgram.db`
（`src/database.rs`），一个普通的 SQLite 文件，任何 SQLite 工具都能打开，因为用户的数据是用户的（维护者 2026-10-24 的
决定；0.5.9 及以前整库用 SQLCipher 加密）。只有密码那一列（以及凭据的值，见下）是封起来的：AES-256-GCM，密钥随机生成、第一次就存进钥匙串
"FinchGram" 名下（account 是“database”），只有本应用能读；封的时候绑定它所属的服务器（协议、地址、端口和账号是封印的
附加认证数据，密文挪到写着另一台服务器的行里打不开）；app 在服务器的表单里显示密码，用户要看在那儿看。AI 访问里允许过
的程序，每行带一个用同一把密钥算出的标签，算在程序、答案和当时登录的 Telegram 账号上：不是 FinchGram 写的行、事后被改过的行、
或者从另一个账号那时候留下来的行，都不算数；SSH 服务器的密钥、以及
settings.toml 里的 AI 访问设置（`settings::Mcp::seal`，启动时由 src/mcp/access.rs 核对）同样带标签。TDLib 的
数据库从 0.4.18 起也有一把自己的密钥，同样存法（account 是“tdlib”）。规矩：偏好设置进 settings.toml，它要小、要能手改；
记录进数据库；Telegram 自己的状态归 TDLib。现在数据库里放的是下载页的服务器、它们的密码和用户是否允许改动每一台（docs/downloads.zh-Hans.md）、
AI 访问里允许过或拒绝过的程序（docs/mcp.zh-Hans.md）、你信任过的 SSH 服务器密钥和信任的日期
（docs/downloads.zh-Hans.md），以及 app 自己的、不属于哪台服务器的凭据：语音输入的密钥，每个 AI 服务商一个
（`voice_input.connection.<id>`，docs/voice-input.zh-Hans.md），每个都封到自己的名字、再加上服务商的 API 格式和地址
（照 settings.toml 里写的）上，所以服务商的地址被 FinchGram 自己的对话框以外的东西改过之后，密钥就打不开了，要用户重新输入；
以后的东西（重启后还在的下载队列、日志、每个聊天的设置）建的时候再加表。

从 0.6.0 起表结构有编号：SQLite 自带的 `PRAGMA user_version` 存着 `LAYOUT`，每改一次表就加一，每一步在
`database::upgrade` 里有对应的迁移。旧版本的文件先原样复制一份到 `~/Documents/FinchGram/backup/`（副本里密码仍是封着的），
再一步一步升上来，每步一个事务，自动进行、不弹框；新版本的文件拒绝打开并说明，不碰它。能加列就只加列、带默认值，
这样旧版本的 app 也能打开新文件。0.5.9 及以前的文件不读：搬进那个备份文件夹，另建新文件，这是维护者定的（那天 app
有六颗星）。表结构 2（0.6.2 之后）把密码重新封到各自的服务器上、给程序的行加了标签（之前的行没有东西能担保，一律丢掉，
每个程序再被问一次），并给 SSH 服务器的密钥记下信任的日期。表结构 3（0.6.3 之后）给 SSH 服务器的密钥也加了标签，之前的
行丢掉，每台 SFTP 服务器再出示一次密钥。表结构 4 给服务器加了 `allow_changes`，之前的服务器一律关；表结构 5 加了
`credentials`，按名字存 app 自己的秘密（两个都在 0.6.3 之后）。表结构 6（0.7.5 之后）把凭据连同服务商的格式和地址一起封起来
（之前的密钥在那次首次启动时由语音输入重新封一遍，也只有那一次还能打开），并给程序的行标上当时允许的账号；之前的行没写账号，
一律丢掉，每个程序再被问一次。settings 的键名改动必须带 serde alias，钥匙串条目的名字永远不改。

## 平台

| | macOS | Windows | Linux |
|---|---|---|---|
| 时间 | 现在 | 计划中 | 以后 |
| finchgram-tdlib | 我们的 CI 构建（Apple silicon） | 计划中 | 计划中 |
| 更新 | 我们自己的更新器 | 我们自己的更新器 | 我们自己的更新器或系统包 |

## 现在不做

- **通话**。TDLib 只管信令；媒体部分（tgcalls）本身就是一个大项目。
- **别人写的插件**。先得有信任模型：谁来签名、用户批准什么、怎么隔离。
- **用户自己的密钥之外的语音输入**。[voice-input.zh-Hans.md](voice-input.zh-Hans.md) 的五步里第一步已经做了
  （2026-10-25），2026-10-26 起向所有厂商和用户自己跑的服务开放；其余的没有：FinchGram 卖的套餐、操作系统自带的识别，
  以及在它们之间选择的路由（现在由用户自己选主要语言）。

## 还没定的事

1. **数据库加密**：0.4.18 定了。TDLib 的数据库和 app 自己的数据库都用随机密钥加密，密钥放钥匙串（见上面
   “app 自己的数据库”）。
2. **下载的文件放哪**：2026-10-20 定了：`~/Documents/FinchGram/media_cache`，用户看得见、可以自己清（清掉的看的时候会重新取）；不设大小上限，维护者的选择。什么时候取什么、还有什么要做（提前下载、边下边播），见 [media-files.zh-Hans.md](media-files.zh-Hans.md)。
3. **媒体播放**（媒体中心）：语音消息（mpv 能读的格式都行）、视频、GIF 和动态贴纸（WebM、Lottie）。2026-09-30 定了：用 mpv。
   它以 libmpv 的形式出现，跟 finchgram-tdlib 一样由 `vendor/mpv/build.sh` 从锁定的源码构建：mpv 加上 FFmpeg、
   libplacebo 和 libass（FreeType、FriBidi、HarfBuzz），全部静态链接进一个只依赖 macOS 的库；app 链接它，
   并把它放在 app 包的 Frameworks 目录里。窗口每次渲染时，mpv 在 Slint 自己的 OpenGL 上下文里把当前帧画进我们的
   一张纹理（`src/player/`），能硬解的用 VideoToolbox；媒体查看器显示这张纹理。代价是 libmpv 在 app 的进程里
   解码，一个故意构造的视频如果弄坏了解码器，窗口会跟着一起崩。FFmpeg 构建时
   去掉了网络、编码器和设备，并跟着上游升级；如果还不够，就把播放挪进一个沙箱里的辅助进程，由它把画面交给 app
   （macOS 上用 IOSurface）。Lottie 贴纸要另想办法（rlottie），WebM 贴纸要能保留透明通道的 VP9 解码（libvpx；
   FFmpeg 自带的 VP9 解码器会丢掉透明通道）。在那之前，动态贴纸只显示它的静态缩略图；静态贴纸照原样显示。
4. **渲染**：暂时用 Slint 的默认渲染器（FemtoVG）。设计稿的字体都已打包，界面用到的中文、日文、韩文和阿拉伯文由
   Noto Sans SC、JP、KR、Arabic 提供，不归任何文字的符号区块（广告里的花体字母、带圈数字、箭头）由 Noto Sans Math 和
   Symbols 2 提供，排版引擎对这些字符从不去查系统字体（回退顺序里界面语言自己的字体排在前面，`fonts.rs`）；emoji 用系统的彩色 emoji
   字体，`fonts.rs` 把它紧排在正文字体后面，免得 CJK 字体里黑白的 emoji 抢先（自己带心形、箭头字形的字体仍会画
   自己的，Source Sans 3 的 ❤ 就是）；消息里的其他文字暂时用系统字体。Skia 得从源码构建才行：它的 Rust 绑定默认会
   下载预编译的库，这是 conventions.md 不允许的。改用由我们自己的 CI 构建的 Skia 2026-10-11 列入计划、次日暂停：
   计划、原因，以及它对 tiny-skia 和 resvg 意味着什么，见 [skia.zh-Hans.md](skia.zh-Hans.md)。
