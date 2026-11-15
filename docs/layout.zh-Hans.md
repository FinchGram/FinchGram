# 仓库目录

[English](layout.md)

东西都放在哪里。它们怎么配合，见 [architecture.zh-Hans.md](architecture.zh-Hans.md)。

```
.github/workflows/
  mpv.yml                # 在干净的机器上构建 libmpv；mpv-* tag 发布成 release
  check.yml              # 每次 push 到 main 跑 clippy 和测试
  release.yml            # Release v* 提交时构建并发布 app
  skia.yml               # 在干净的机器上构建 Skia；skia-* tag 发布成 release
  tdlib.yml              # 在干净的机器上构建 finchgram-tdlib；tdlib-* tag 发布成 release
Cargo.toml
build.rs                 # 把 vendor/tdlib/bin/ 复制到可执行文件旁边
crates/
  finchgram-release-sign/ # release 背后的 Ed25519 签名工具，单独一个包
  finchgram-ui/          # 界面：编译 ui/app.slint，打包 lang/，单独一个包
docs/                    # 文档：一件事一个小文件，各有英文和简体中文两版
  screenshots/           #   README 里的图，来自截图测试（scripts/readme-pictures.sh）
lang/                    # 翻译：lang/<代码>/LC_MESSAGES/finchgram.po，编译进二进制
readme/                  # README 的其他语言版本
release-signing.pub      # 允许给 release 签名的公钥，编译进 app
scripts/
  bundle.sh              # 构建 dist/FinchGram.app（release 流程跑的就是它）
  fetch-fonts.sh         # 把锁定的界面字体下载到 vendor/fonts/
  fetch-mpv.sh           # 把锁定的 libmpv release 下载到 vendor/mpv/bin/
  fetch-skia.sh          # 把锁定的 Skia release 下载到 vendor/skia/bin/，并告诉 cargo 它在哪里
  fetch-tdlib.sh         # 把锁定的 finchgram-tdlib release 下载到 vendor/tdlib/bin/
  readme-pictures.sh     # 把 README 用的图从 target/screenshots/ 复制到 docs/screenshots/
  release.sh             # 发起一次 release：改版本、打 tag、push，其余交给 GitHub Actions
  sign.sh                # 给 app 签名：默认临时签名，release 流程里用 Developer ID
src/
  main.rs                # 窗口、设置、语言和主题、更新；启动 Telegram
  fonts.rs               # 界面字体，编译进可执行文件
  images.rs              # 消息里的图片，在界面线程之外解码
  command_line.rs        # Terminal 的命令行：已输入的字能对上哪些命令
  telegram/              # 唯一跟 finchgram-tdlib 打交道的代码
    process.rs           #   运行这个程序：通过标准输入输出传递 TDLib 的 JSON
    api.rs               #   FinchGram 用到的 TDLib 类型（照锁定版本的 td_api.tl 写）
    mod.rs               #   请求和回复、重新启动；更新交给 store
    store.rs             #   TDLib 告诉我们的聊天、用户和消息；页面用的 model
    login.rs             #   登录、注册
    chats.rs             #   聊天列表
    conversation.rs      #   打开的聊天：消息、发送
    actions.rs           #   消息操作：右键菜单、回复、转发等
    account.rs           #   个人资料、退出登录
    password.rs          #   设置里的两步验证
    files.rs             #   下载文件
    avatars.rs           #   聊天和联系人的头像，代替首字母色块
    viewer.rs            #   媒体查看器：照片、视频、保存到“下载”
    rich_text.rs         #   带格式的文字：粗体、斜体、链接……
    notifications.rs     #   新消息通知、Dock 图标上的未读数
    online.rs            #   窗口在前台、有人在用时，账号是在线状态
  platform/              # 随操作系统而不同的部分
  player/                # 用 libmpv 播放视频，画进一块由窗口显示的 surface
  screenshot/            # 截图工具：截取、覆盖层、标注、成图
  downloads/             # 下载页面：WebDAV、FTP、FTPS 和 SFTP，带多连接下载引擎
  music/                 # 音乐页面：Subsonic API 客户端、边播边下的缓存、队列与播放条
    remote.rs            #   三个客户端的同一张脸：列目录、检查服务器、取文件
    webdav.rs            #   PROPFIND 列目录、地址、Basic 认证
    ftp.rs               #   FTP 和 FTPS，用 suppaftp：单流，REST 接着下
    sftp.rs              #   SFTP，用 russh：多个会话分块下载，记住服务器密钥
    transfer.rs          #   一个文件分块多连接下载，停了接着来
    mod.rs               #   页面状态、线程、队列、服务器表单
  update.rs              # 自动更新：GitHub Releases、校验签名、替换、重启
  settings.rs            # 用户偏好（settings.toml）
  i18n.rs                # 界面语言：保存的选择，否则跟系统，否则英文
  screenshots.rs         # 把每个页面画成 PNG（cargo test screenshots -- --ignored）
ui/
  app.slint              # 主窗口：菜单，以及显示哪个页面
  state.slint            # app 的状态，Rust 和页面共用
  telegram.slint         # Telegram 的内容：账号、登录、聊天、消息
  downloads.slint        # 下载页面的状态：服务器、文件夹、队列
  look.slint             # 当前主题的颜色、字体和形状，给共用页面用
  format.slint           # 按界面语言写出的日期、数量和消息类型
  widgets.slint          # 共用的小部件；chat.slint 是聊天窗口共用的部分
  viewer.slint           # 覆盖整个窗口的媒体查看器，三套主题各有样式
  pages/                 # 三套主题共用的页面：登录、设置、个人资料、下载
  workbench/             # 工作台主题的聊天窗口（默认）
  broadsheet/            # 报纸主题的聊天窗口
  terminal/              # 终端主题的聊天窗口
  icons/                 # Phosphor 图标（MIT），常规和双色两种；icons.slint 列出它们
  logo/                  # FinchGram 的 logo（svg、png）和使用规则
vendor/fonts/            # 不进 git：界面字体（scripts/fetch-fonts.sh）
vendor/mpv/              # libmpv：mpv 和 FFmpeg，用来播放视频
  build.sh               #   从锁定的源码构建它：版本和 SHA-256 都锁定在脚本顶部
  bin/                   #   不进 git：库本身（scripts/fetch-mpv.sh 下载，或 build.sh install）
vendor/skia/             # Skia，用来画窗口
  build.sh               #   从锁定的源码构建它：各个 commit 和包的名字都锁定在脚本顶部
  Cargo.lock             #   锁定这次构建用到的 crate
  bin/                   #   不进 git：包本身（scripts/fetch-skia.sh 下载，或 build.sh install）
vendor/tdlib/            # finchgram-tdlib
  build.sh               #   从锁定的源码构建它：TDLib commit、OpenSSL 版本和 SHA-256 都锁定在脚本顶部
  host/                  #   我们的小外壳程序（main.cpp）和它的 CMakeLists.txt
  bin/                   #   不进 git：app 用的程序（scripts/fetch-tdlib.sh 下载，或 build.sh install）
  work/, dist/           #   不进 git：本地构建的中间文件和打好的包
```
