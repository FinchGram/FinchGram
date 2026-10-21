# 开发规范

[English](conventions.md)

照酷丸工具箱的规范来，只有一点不同：FinchGram 是开源的，所以源码、vendor 构建和发布全都放在这一个仓库里。

## 1. 所有第三方依赖随 app 一起打包，从锁定的源码构建

### 规则

- app 只通过自己带的 finchgram-tdlib 跟 Telegram 通信，永远不找 PATH、Homebrew 或用户机器上的任何东西。
  找不到自带的就是打包错误，界面直接报错，没有任何退路。
- 用户拿到的 app 已经包含全部依赖，不需要安装、下载任何东西。
- finchgram-tdlib 由本仓库的 vendor 构建从源码编译：`vendor/tdlib/build.sh`。TDLib 的 commit、
  OpenSSL 的版本和 SHA-256、最低支持的 macOS 版本都锁定在脚本顶部。所有第三方库都是静态链接的：
  程序只依赖 macOS 自带的东西，脚本会检查这一点（`otool -L`），也会检查版本，再跑一遍冒烟测试。
- CI 在干净的机器上构建它，并从 `tdlib-1.8.67-1` 这样的 tag 发布成本仓库的一个 release
  （`.github/workflows/tdlib.yml`）。不用任何第三方预编译的二进制。
- 二进制不进 git。每个开发者在本地跑一次 `scripts/fetch-tdlib.sh`，它把锁定的 release 下载到
  `vendor/tdlib/bin/`（已在 .gitignore 里）并校验 SHA-256，对不上就失败。
- 升级时改 `vendor/tdlib/build.sh` 顶部锁定的那一块，推一个新的 tdlib-* tag，然后改
  `scripts/fetch-tdlib.sh` 里的 release tag 和 SHA-256，以及 `src/telegram/mod.rs` 里的 `TDLIB_VERSION`
  （td_api.tl 里我们读的东西有变化时，还要改 `src/telegram/api.rs`）。提交之后所有人重新跑一次脚本，
  就都在同一个版本上；回退也一样。
- `build.rs` 把 `vendor/tdlib/bin/finchgram-tdlib` 复制到可执行文件旁边（`target/debug/` 或 `target/release/`），
  所以 `cargo run` 和打包出来的 app 用的是完全同一个程序。它不存在时编译直接失败，并提示先跑脚本。
- 要试改 vendor 构建时，就在本地构建：先 `vendor/tdlib/build.sh`，再 `vendor/tdlib/build.sh install`。
  发布的版本里永远不会有在开发者机器上编译的程序。

### 为什么

- **版本可控**：所有开发者、所有构建、所有用户用的都是同一个 TDLib，行为一致，问题可复现。
  `api.rs` 里的类型也正好对应这个版本。
- **随时升级**：改一块就能升到新版本，回退也一样简单。
- **协作方便**：clone 之后跑一次脚本就行。除非要改 TDLib，否则谁都不用编译它。
- **用户省心**：跟用户机器上装了什么无关。

### 以后所有外部依赖都照这个模式

不管是另一个程序、一个库、字体还是数据文件：都在 `vendor/<名字>/` 下占一个目录，用一个锁定了版本和哈希的脚本
构建或下载它，再随 app 一起发布。脚本进 git，它构建或下载出来的东西不进。永远不依赖用户机器上已有的东西，也不在代码里留“自带的没有就用系统的”这种后门。
受支持的操作系统每一份都自带的东西可以用（比如 macOS 上的 zlib 和系统框架）。

界面字体就是照这个做的：`scripts/fetch-fonts.sh` 从锁定的 google/fonts commit 下载到 `vendor/fonts/` 并校验
SHA-256，`src/fonts.rs` 把它们编译进可执行文件，缺了的时候 `build.rs` 会提前报错。图标是一些很小的 SVG 文件
（Phosphor，MIT），原样放在 git 的 `ui/icons/` 里，连同它们的许可证，跟酷丸工具箱放图标的做法一样。

libmpv 跟 finchgram-tdlib 一样照这个做：`vendor/mpv/build.sh` 按锁定的版本和 SHA-256 构建 mpv 和它需要的库
（FFmpeg、libplacebo，以及 libass 和它用的 FreeType、FriBidi、HarfBuzz），全部静态链接进一个只依赖 macOS 的
`libmpv.2.dylib`；脚本会检查这一点（`otool -L`），并播放一小段原始视频做冒烟测试。CI 从 `mpv-<版本号>-<n>`
这样的 tag 构建并发布它（`.github/workflows/mpv.yml`），`scripts/fetch-mpv.sh` 把锁定的 release 下载到
`vendor/mpv/bin/`。`build.rs` 从那里链接它，并把它复制到可执行文件旁边；app 包把它放在 `Contents/Frameworks/`
里，可执行文件的 run path 只在那里找它。

## 2. 一个仓库

- 源码、vendor 构建（`vendor/`）和发布全在这里，都公开。酷丸工具箱拆成三个仓库，是因为它的源码不公开；
  FinchGram 没有理由这样拆。
- tag：app 用 `v<版本号>`，finchgram-tdlib 用 `tdlib-<版本号>-<n>`，libmpv 用 `mpv-<版本号>-<n>`（`<n>` 是同一个
  版本的第几次构建）。
- vendor 的 release 发布时加 `--latest=false`，这样本仓库的 “Latest release” 永远是最新的 app，
  已安装的 app 就靠它来自动更新。
- 二进制只放在 release 里，永远不进 git 历史：`vendor/*/bin/`、`vendor/*/work/`、`vendor/*/dist/`、
  `dist/` 和 `target/` 都在 .gitignore 里。

## 3. 先发布，再测装好的 app

跟酷丸工具箱一样。

- 按用户拿到 app 的方式来测：装上 release 直接用。发布很便宜，一天可以发很多次。
- 每次发布都走 `scripts/release.sh`，不手动发布任何东西。`Cargo.toml` 里的版本、git tag `v<版本号>`
  和 GitHub release 是同一回事。
- release 由 GitHub Actions（`.github/workflows/release.yml`）在干净的机器上从打了 tag 的 commit 构建，
  永远不在开发者的机器上构建，用的是所有人都在用的同一个脚本拿到的锁定版 finchgram-tdlib。流程里跑的是
  `scripts/bundle.sh`，本地想试试打包后的 app 也用它；本地打出来的包永远不发布。流程用到的每个 Action（checkout、
  Rust 工具链、缓存、上传产物）都按完整的 commit SHA 锁定，旁边注明版本，不用可以被挪动的 tag 或分支：在给 release
  签名的机器上跑的，必须是审过的那份代码。要升级就改 SHA。
- 每个 release 都签名：CI 用只有 CI 才有的 Ed25519 私钥（`RELEASE_SIGNING_KEY` secret，另在发布负责人的
  钥匙串里以 "FinchGram release signing key" 备份一份）给 `SHA256SUMS` 签名，app 用编译进去的公钥
  （`release-signing.pub`）验证，验证不过的更新一律不装。密钥对由 `src/bin/finchgram-release-sign.rs`
  生成（`keygen`），CI 签名和验证用的也是它。换密钥：把新公钥加到 `release-signing.pub` 第二行，发一个版本，
  换掉 secret，再删掉旧的那行。
- 装好的 app 自己更新（`src/update.rs`）：每天在随机时刻检查一次（settings.toml 里的 `check_for_updates`
  可以关掉），也可以从 帮助 → 检查更新… 手动检查。先验证签名，再用签过名的清单校验 zip 的 SHA-256；
  它要找的 zip 名字由版本号推出来，所以旧的签名清单没法挪到新 tag 上冒充。
- 更新流程是产品的一部分，必须保证从每一个发布过的版本都能更新到下一个版本。改 asset 名字、`SHA256SUMS`
  格式、签名密钥或仓库之前，必须先发一个能认新布局的版本。
- CI 用 North Brookville LLC 的 Developer ID Application 证书（团队 S7W3MG9276；证书 2027-11-20 到期，到时换同一
  团队的新证书，对用户没有影响）给 app 签名，开 Hardened Runtime 并带时间戳。macOS 把用户给 app 的权限（截图工具
  要的“屏幕录制”）绑在代码签名的团队上，所以更新、换证书都不会丢；ad-hoc 签名每次构建都不一样，以前每次更新丢权限
  就是这个原因。证书和私钥只存在于 secrets `CODESIGN_CERTIFICATE`（base64 的 .p12，里面带着 Apple 的 Developer ID
  G2 中间证书）和 `CODESIGN_CERTIFICATE_PASSWORD` 里，另在维护者的登录钥匙串里备份一份；没有这两个 secret（比如
  fork）时流程退回 ad-hoc 签名并给出警告。v0.3.2 到 v0.4.16 用的是自签名证书（"FinchGram"），换过来时每个用户重新
  授权了一次屏幕录制。
- 每个 release 都做公证：流程用 `notarytool` 把签好名的 app 交给 Apple（App Store Connect 的 API 密钥，secrets
  `NOTARY_KEY`、`NOTARY_KEY_ID`、`NOTARY_ISSUER_ID`），把票据钉进 app 包，再用 `spctl` 问一遍 Gatekeeper 的结论，
  然后才打 zip，所以下载的 app 直接能开。没有这几个 secret（比如 fork）时 app 只签名不公证，第一次打开 macOS 会问
  一次。本地 `SIGN_IDENTITY="Developer ID Application: …" scripts/bundle.sh` 签法相同，只是不公证。

## 4. Telegram API 凭据

- FinchGram 有自己在 my.telegram.org 申请的 api_id 和 api_hash。它们永远不进仓库：发布流程从 secrets
  （`FINCHGRAM_API_ID`、`FINCHGRAM_API_HASH`）里取出来交给编译器；开发者编译时把这两个变量设成自己的。
- 没有它们的构建也能运行，窗口上会说明这个版本没有 API ID。永远不会退而用别人的。
- 开发时用 Telegram 的测试服务器，避免动到真实账号：启动 app 时加 `FINCHGRAM_TEST_DC=1`
  （它有自己单独的数据库 `tdlib-test`）。
