# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · **繁體中文** · [Español](es.md) · [Português](pt.md) · [Deutsch](de.md) · [Français](fr.md) · [Русский](ru.md) · [日本語](ja.md) · [한국어](ko.md) · [العربية](ar.md)

<p align="center"><img src="../ui/logo/png/FinchGram-icon-128.png" width="96" alt="FinchGram"></p>

**一個有超能力的 Telegram 桌面客戶端，用 Rust 和 [Slint](https://slint.dev) 寫成。** 開源，GPL-3.0 授權。
現在支援 Apple silicon 的 macOS，Windows 和 Linux 之後再做。

FinchGram 透過 Telegram 官方的函式庫 [TDLib](https://core.telegram.org/tdlib) 和 Telegram 通訊，並遵守
[Telegram API 條款](https://core.telegram.org/api/terms)。它是非官方客戶端，不是 Telegram 出品的。

<p align="center"><img src="../docs/screenshots/workbench-light-chats.png" width="800" alt="工作台主題的聊天視窗"></p>

## 能做什麼

- **Telegram 該有的都有。** 聊天、資料夾、搜尋，最多十個帳號；訊息的回覆、回應、機器人、話題和草稿；照片、影片、檔案和語音訊息；管理自己的群組和頻道。
- **截圖工具。** ⌘⇧A 定住畫面：取一個視窗或拖一個選區，在上面標註，傳送出去。在每個聊天裡都能用，像微信那樣。
- **下載。** 一個下載用戶端，連你自己的伺服器，走 WebDAV、FTP、FTPS 或 SFTP：NAS、seedbox、PikPak。[詳見](../docs/downloads.zh-Hans.md)
- **音樂。** 播放你自己伺服器上的音樂，只要它支援 Subsonic API（Navidrome 等）。[詳見](../docs/music.zh-Hans.md)
- **直播。** Owncast、PeerTube、Icecast 和 Telegram 自己的直播：發現、追蹤，就在 app 裡看。[詳見](../docs/live-streams.zh-Hans.md)
- **語音輸入。** 對著輸入框說話，走你自己選的 AI 服務商、用你自己的金鑰。[詳見](../docs/voice-input.zh-Hans.md)
- **AI 助理。** Claude 等支援 MCP 的應用程式，在你允許之後，透過 FinchGram 讀你的聊天並在其中操作。[詳見](../docs/mcp.zh-Hans.md)
- **三套主題。** 工作台、報紙和終端，各有淺色和深色。終端有一條真能用的指令列。[怎麼用](../docs/terminal.zh-Hans.md)
- **十一種語言**，還會自己更新：先驗證，再安裝。

[全部功能，附圖](../docs/features.zh-Hans.md)。還沒有的：秘密聊天和通話、投票和排程訊息、關鍵字過濾、Windows 和 Linux。見 [接下來做什麼](../docs/architecture.md#not-now)。

## 下載

從[最新的 release](https://github.com/FinchGram/FinchGram/releases/latest) 下載
`FinchGram-<版本>-macos-arm64.zip`，解壓縮後把 FinchGram 拖進「應用程式」。需要 Apple silicon 的 macOS 12
或更新。app 用 North Brookville LLC 的 Developer ID 簽名並經 Apple 公證，和其它 app 一樣直接打開。之後它會自動更新。

每個 release 都帶 `SHA256SUMS` 和它的 Ed25519 簽章。app 安裝更新之前會先驗證這兩樣；你也可以自己驗證，公鑰在
[release-signing.pub](../release-signing.pub)。

## 三套主題

| 工作台 | 報紙 | 終端 |
|---|---|---|
| ![工作台](../docs/screenshots/workbench-light-chats.png) | ![報紙](../docs/screenshots/broadsheet-light-chats.png) | ![終端](../docs/screenshots/terminal-light-chats.png) |

工作台是預設主題，一個帶分頁的工作視窗。報紙讀起來像報紙，每個帳號有自己的強調色。終端是等寬字型的畫面，
帶指令。每套都有淺色和深色，跟隨系統或自己選，在「設定 › 外觀」裡。

終端主題在最下面那一列執行指令：按 ⌘K，或者輸入 `/`。指令和按鍵見[終端主題](../docs/terminal.zh-Hans.md)。

<p align="center"><img src="../docs/screenshots/workbench-dark-chats.png" width="800" alt="工作台，深色"></p>

## Telegram 的規矩

FinchGram 只做 Telegram 官方應用程式做的事，不做它們禁止的事。訊息在你看到時標為已讀，對方像在任何 Telegram 應用程式裡一樣
能看到你正在輸入和在線上，頻道裡的贊助訊息照常顯示，自毀媒體只能打開一次，來自 Telegram 的內容不會交給任何 AI，除非你在百寶箱 → AI 存取裡放一個助理進來，或者打開了用你自己的 AI 服務商把語音訊息轉文字。
除了 Telegram、用來更新的 GitHub，以及你用語音輸入時以你自己的金鑰傳給你所選 AI 服務商的聲音，什麼都不會離開你的機器。

## 文件

文件有英文和簡體中文兩種。

- [FinchGram 能做什麼](../docs/features.zh-Hans.md)：每一項功能，附圖。
- [終端主題](../docs/terminal.zh-Hans.md)：它的指令列和全部指令。
- [從原始碼建置](../docs/development.zh-Hans.md)、[目錄結構](../docs/layout.zh-Hans.md)、[翻譯](../docs/translations.zh-Hans.md)、[發佈](../docs/releases.zh-Hans.md)。
- [架構](../docs/architecture.zh-Hans.md)和[慣例](../docs/conventions.zh-Hans.md)：它是怎麼做的，為什麼這麼做。

## 開發

app 是一個外殼：Telegram 本身交給 TDLib，它作為獨立程式跑在旁邊，由本儲存庫從鎖定的原始碼建置，libmpv 和 Skia 也一樣。建置需要 Apple silicon 的 macOS 和你自己的 Telegram API ID；步驟見 [docs/development.zh-Hans.md](../docs/development.zh-Hans.md)。

## 參與

歡迎提 issue 和 pull request。改動東西之間的關係之前，先讀 [docs/architecture.md](../docs/architecture.md)
和 [docs/conventions.md](../docs/conventions.md)：每個依賴都由本倉庫從鎖定的原始碼建置，介面在三套主題裡
都照設計稿做，app 裡沒有任何違反 Telegram API 條款的東西。保持 `cargo test`、`cargo clippy --all-targets` 和
`cargo test screenshots -- --ignored` 乾淨，並且看一眼畫出來的圖。README 裡的圖也來自這個測試：
`scripts/readme-pictures.sh` 會更新 `docs/screenshots/`。

## 授權條款

GPL-3.0（[LICENSE](../LICENSE)）。finchgram-tdlib 包含 TDLib（Boost Software License 1.0）和
OpenSSL（Apache License 2.0）；介面字型採用 SIL Open Font License 1.1，圖示採用 MIT 授權。
它們的授權條款文字都隨 app 一起發佈。
