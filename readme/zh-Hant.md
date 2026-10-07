# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · **繁體中文** · [Español](es.md) · [Português](pt.md) · [Deutsch](de.md) · [Français](fr.md) · [Русский](ru.md) · [日本語](ja.md) · [한국어](ko.md) · [العربية](ar.md)

<p align="center"><img src="../ui/logo/png/FinchGram-icon-128.png" width="96" alt="FinchGram"></p>

**一個帶媒體中心的 Telegram 桌面客戶端，用 Rust 和 [Slint](https://slint.dev) 寫成。** 開源，GPL-3.0 授權。
現在支援 Apple silicon 的 macOS，Windows 和 Linux 之後再做。

FinchGram 透過 Telegram 官方的函式庫 [TDLib](https://core.telegram.org/tdlib) 和 Telegram 通訊，並遵守
[Telegram API 條款](https://core.telegram.org/api/terms)。它是非官方客戶端，不是 Telegram 出品的。

<p align="center"><img src="../docs/screenshots/workbench-light-chats.png" width="800" alt="工作台主題的聊天視窗"></p>

## 能做什麼

- **聊天。** 用手機號碼、驗證碼和兩步驟驗證密碼登入，或者掃 QR code 登入；也可以註冊新帳號。聊天列表帶 Telegram
  的資料夾、置頂、未讀數和 @ 提醒；有搜尋框（⌘K）；還有頻道檢視和機器人檢視。
- **訊息。** 帶格式的文字（粗體、斜體、程式碼、連結）、回覆和轉傳、照片、影片、GIF、貼圖、檔案，以及連結預覽卡片。
  在訊息上按右鍵打開它的選單：回覆、編輯、複製、複製連結、轉傳、檢舉、刪除，或選取多則。設為自毀的照片和影片
  以模糊顯示、只能打開一次，和 Telegram 官方應用程式一樣。
- **媒體。** 照片和影片在蓋住整個視窗的檢視器裡打開，可以翻看同一相簿的其他圖片；影片用從原始碼建置的 mpv
  播放。聊天允許時可以儲存到「下載項目」。
- **傳送。** 照片、影片和檔案可以從迴紋針選單傳送，也可以拖進視窗或直接貼上。送出之前先在一張卡片裡顯示：
  可以加說明文字，按照片傳或按檔案傳，最多十個合成一個相簿，私訊裡可以設自毀計時器，也可以靜音傳送。
- **截圖。** 輸入列裡的剪刀或 ⌘⇧A 會定住螢幕：取一個視窗或拖一個選取範圍，畫上方框、橢圓、箭頭、畫筆、文字和馬賽克，
  然後直接傳送、複製或儲存。和微信一樣，每個聊天裡都有。
- **聊天選單。** 靜音、置頂、標為已讀、放進資料夾；封鎖或解除封鎖對方、檢舉、退出群組或頻道、刪除聊天。
- **通知。** 新訊息以 macOS 通知提醒，未讀數顯示在 Dock 圖示上；關閉視窗後 FinchGram 留在 Dock 裡，也可以住進選單列。
- **三套主題。** 工作台、報紙和終端，各有淺色和深色，切換不用重新啟動。
- **設定。** 開機時啟動、按 Enter 傳送、截圖快捷鍵、通知的聲音和預覽、兩步驟驗證、介面語言，還有更新：app 從
  GitHub Releases 自動更新，驗證不過的一律不裝。
- **語言。** 介面有英文和簡體中文；這份 README 有十一種語言。

還沒有的：秘密聊天、語音訊息和通話、多帳號、投票和排程訊息、關鍵字過濾、Windows 和 Linux。見
[接下來做什麼](../docs/architecture.md#not-now)。

## 下載

從[最新的 release](https://github.com/FinchGram/FinchGram/releases/latest) 下載
`FinchGram-<版本>-macos-arm64.zip`，解壓縮後把 FinchGram 拖進「應用程式」。需要 Apple silicon 的 macOS 12
或更新。app 還沒有公證，所以第一次打開時 macOS 會攔一次：系統設定 → 隱私權與安全性 → 「強制打開」。之後它會自動更新。

每個 release 都帶 `SHA256SUMS` 和它的 Ed25519 簽章。app 安裝更新之前會先驗證這兩樣；你也可以自己驗證，公鑰在
[release-signing.pub](../release-signing.pub)。

## 三套主題

| 工作台 | 報紙 | 終端 |
|---|---|---|
| ![工作台](../docs/screenshots/workbench-light-chats.png) | ![報紙](../docs/screenshots/broadsheet-light-chats.png) | ![終端](../docs/screenshots/terminal-light-chats.png) |

工作台是預設主題，一個帶分頁的工作視窗。報紙讀起來像報紙，每個帳號有自己的強調色。終端是等寬字型的畫面，
帶指令。每套都有淺色和深色，跟隨系統或自己選，在「設定 › 外觀」裡。

<p align="center"><img src="../docs/screenshots/workbench-dark-chats.png" width="800" alt="工作台，深色"></p>

## 截圖工具

<p align="center"><img src="../docs/screenshots/workbench-light-shot-annotated.png" width="800" alt="截圖覆蓋層，帶標註"></p>

按 ⌘⇧A，或者點迴紋針旁邊的剪刀。螢幕定住，整個畫面壓暗：拖一個選取範圍，範圍內不壓暗，帶放大鏡和像素尺寸；
點一下某個視窗則整個取下。工具列可以畫方框、橢圓、箭頭、畫筆、文字和馬賽克，三種粗細、六種顏色，可以復原。
按 Done 把圖放進傳送卡片，可以配上說明文字；⌘C 複製；⌘S 存到「下載項目」。「設定 › 一般 › 截圖」裡可以改快捷鍵，
也可以決定截圖時要不要先把 FinchGram 的視窗藏起來。

<p align="center"><img src="../docs/screenshots/workbench-light-shot-send-card.png" width="800" alt="帶截圖的傳送卡片"></p>

## 怎麼做的

app 是一個外殼。Telegram 本身交給 TDLib，它作為一個獨立程式跑在執行檔旁邊：`finchgram-tdlib`，由本倉庫從鎖定的
原始碼建置；外殼只透過 `src/telegram/` 跟它通訊，用的是 TDLib 自己的 JSON。影片用同樣方式建置的 libmpv 播放。
不從使用者的機器上拿任何東西，建置時除了這些鎖定且帶校驗和的產物之外什麼也不下載。所有東西都公開：原始碼、vendor 建置
（`vendor/`）和發佈。見 [docs/architecture.md](../docs/architecture.md) 和
[docs/conventions.md](../docs/conventions.md)。

## Telegram 的規矩

FinchGram 只做 Telegram 官方應用程式做的事，不做它們禁止的事。訊息在你看到時標為已讀，對方像在任何 Telegram 應用程式裡一樣
能看到你正在輸入和在線上，頻道裡的贊助訊息照常顯示，自毀媒體只能打開一次，來自 Telegram 的內容不會交給任何 AI。
除了 Telegram 和用來更新的 GitHub，什麼都不會離開你的機器。

## 開發

目前開發需要 Apple silicon 的 macOS：finchgram-tdlib 暫時只為它建置。Linux 和 Windows 之後再支援。

finchgram-tdlib、libmpv 和介面字型都不進 git：`vendor/tdlib/` 和 `vendor/mpv/` 裡只放建置它們的腳本，
字型來自 Google Fonts。clone 之後先把它們下載一次：

```sh
scripts/fetch-tdlib.sh    # 把鎖定的 finchgram-tdlib release 下載到 vendor/tdlib/bin/ 並驗證 SHA-256
scripts/fetch-mpv.sh      # 把鎖定的 libmpv release 下載到 vendor/mpv/bin/ 並驗證 SHA-256
scripts/fetch-fonts.sh    # 把鎖定的介面字型下載到 vendor/fonts/ 並驗證 SHA-256
FINCHGRAM_API_ID=… FINCHGRAM_API_HASH=… cargo run
```

- 要修改 finchgram-tdlib 本身時，請在本機建置（幾分鐘；需要 Xcode 或 Command Line Tools，以及
  `brew install cmake ninja gperf`，都只是建置工具）：

  ```sh
  vendor/tdlib/build.sh && vendor/tdlib/build.sh install
  ```

- `FINCHGRAM_API_ID` 和 `FINCHGRAM_API_HASH`：使用你自己的，在 [my.telegram.org](https://my.telegram.org)
  → API development tools 申請。它們在編譯時讀取，永遠不進儲存庫。沒有它們 app 也能啟動，會提示沒有 API ID。
- `FINCHGRAM_TEST_DC=1 cargo run` 連線到 Telegram 的測試伺服器，那裡有獨立的帳號；app 會為它另外保存一個資料庫。
- `cargo test screenshots -- --ignored` 用假資料把每套主題的每個頁面（淺色和深色）畫成圖片，存到
  `target/screenshots/`：不用登入帳號也能看介面。

`build.rs` 把 `vendor/tdlib/bin/finchgram-tdlib` 複製到編譯出來的執行檔旁邊，所以 `cargo run`
用的和打包後的 app 是完全相同的程式；它也從 `vendor/mpv/bin/` 連結 libmpv 並同樣複製過去，`vendor/fonts/`
裡的字型則編譯進執行檔。缺少任何一樣，編譯都會直接失敗並說明原因。需要 Rust 1.92 或更新版本。

TDLib 的資料庫和下載的檔案在 `~/Library/Application Support/FinchGram/tdlib/`，設定在
`~/Library/Application Support/FinchGram/settings.toml`。

## 目錄結構

```
.github/workflows/
  mpv.yml                # 在乾淨的機器上建置 libmpv；mpv-* tag 發佈為 release
  release.yml            # 每次 push 到 main 都建置 app；v* tag 發佈為 release
  tdlib.yml              # 在乾淨的機器上建置 finchgram-tdlib；tdlib-* tag 發佈為 release
Cargo.toml
build.rs                 # 編譯 ui/app.slint，打包 lang/，把 vendor/tdlib/bin/ 複製到執行檔旁邊
docs/                    # architecture.md、conventions.md、drag-and-drop.md（以及簡體中文版）
  screenshots/           #   README 裡的圖，來自截圖測試（scripts/readme-pictures.sh）
lang/                    # 翻譯：lang/<代碼>/LC_MESSAGES/finchgram.po，編譯進二進位檔
readme/                  # 這份 README 的其他語言版本
release-signing.pub      # 允許為 release 簽名的公鑰，編譯進 app
scripts/
  bundle.sh              # 建置 dist/FinchGram.app（release 流程執行的就是它）
  fetch-fonts.sh         # 把鎖定的介面字型下載到 vendor/fonts/
  fetch-mpv.sh           # 把鎖定的 libmpv release 下載到 vendor/mpv/bin/
  fetch-tdlib.sh         # 把鎖定的 finchgram-tdlib release 下載到 vendor/tdlib/bin/
  readme-pictures.sh     # 把 README 用的圖從 target/screenshots/ 複製到 docs/screenshots/
  release.sh             # 發起一次 release：改版本、打 tag、push，其餘交給 GitHub Actions
src/
  main.rs                # 視窗、設定、語言和主題、更新；啟動 Telegram
  fonts.rs               # 介面字型，編譯進執行檔
  images.rs              # 訊息裡的圖片，在介面執行緒之外解碼
  telegram/              # 唯一與 finchgram-tdlib 打交道的程式碼
    process.rs           #   執行這個程式：透過標準輸入輸出傳遞 TDLib 的 JSON
    api.rs               #   FinchGram 用到的 TDLib 型別（依照鎖定版本的 td_api.tl 撰寫）
    mod.rs               #   請求和回覆、重新啟動；更新交給 store
    store.rs             #   TDLib 告訴我們的聊天、使用者和訊息；頁面用的 model
    login.rs             #   登入、註冊
    chats.rs             #   聊天列表
    conversation.rs      #   開啟的聊天：訊息、傳送
    actions.rs           #   訊息操作：右鍵選單、回覆、轉傳等
    account.rs           #   個人資料、登出
    password.rs          #   設定裡的兩步驟驗證
    files.rs             #   下載檔案
    avatars.rs           #   聊天和聯絡人的頭像，代替首字母色塊
    viewer.rs            #   媒體檢視器：照片、影片、儲存到「下載」
    rich_text.rs         #   帶格式的文字：粗體、斜體、連結……
    notifications.rs     #   新訊息通知、Dock 圖示上的未讀數
    online.rs            #   視窗在前景、有人在用時，帳號是上線狀態
  platform/              # 隨作業系統而不同的部分
  player/                # 用 libmpv 播放影片，經 OpenGL 畫進視窗
  screenshot/            # 截圖工具：截取、覆蓋層、標註、成圖
  update.rs              # 自動更新：GitHub Releases、驗證簽名、替換、重新啟動
  settings.rs            # 使用者偏好（settings.toml）
  i18n.rs                # 介面語言：已儲存的選擇，否則跟隨系統，否則英文
  screenshots.rs         # 把每個頁面畫成 PNG（cargo test screenshots -- --ignored）
  bin/                   # finchgram-release-sign.rs，release 背後的 Ed25519 簽名工具
ui/
  app.slint              # 主視窗：選單，以及顯示哪個頁面
  state.slint            # app 的狀態，Rust 和頁面共用
  telegram.slint         # Telegram 的內容：帳號、登入、聊天、訊息
  look.slint             # 目前主題的顏色、字型和形狀，給共用頁面用
  format.slint           # 依介面語言寫出的日期、數量和訊息類型
  widgets.slint          # 共用的小元件；chat.slint 是聊天視窗共用的部分
  viewer.slint           # 覆蓋整個視窗的媒體檢視器，三套主題各有樣式
  pages/                 # 三套主題共用的頁面：登入、設定、個人資料
  workbench/             # 工作台主題的聊天視窗（預設）
  broadsheet/            # 報紙主題的聊天視窗
  terminal/              # 終端主題的聊天視窗
  icons/                 # Phosphor 圖示（MIT），一般和雙色兩種；icons.slint 列出它們
  logo/                  # FinchGram 的 logo（svg、png）和使用規則
vendor/fonts/            # 不進 git：介面字型（scripts/fetch-fonts.sh）
vendor/mpv/              # libmpv：mpv 和 FFmpeg，用來播放影片
  build.sh               #   從鎖定的原始碼建置它：版本和 SHA-256 都鎖定在腳本開頭
  bin/                   #   不進 git：函式庫本身（scripts/fetch-mpv.sh 下載，或 build.sh install）
vendor/tdlib/            # finchgram-tdlib
  build.sh               #   從鎖定的原始碼建置它：TDLib commit、OpenSSL 版本和 SHA-256 都鎖定在腳本開頭
  host/                  #   我們的小型外殼程式（main.cpp）和它的 CMakeLists.txt
  bin/                   #   不進 git：app 使用的程式（scripts/fetch-tdlib.sh 下載，或 build.sh install）
  work/, dist/           #   不進 git：本機建置的中間檔案和打包好的套件
```

## 翻譯

介面上的每一段文字都寫成 `@tr("English text")`。同一個字串無論出現在哪裡都只有一種譯法（build.rs 關閉了
Slint 的預設 context）；同一句英文在別處需要不同譯法時，給它一個 context：`@tr("menu" => "Open")`。
用官方工具擷取，再合併到每種語言：

```sh
cargo install slint-tr-extractor                                          # 安裝一次
find ui -name '*.slint' | sort | xargs slint-tr-extractor --no-default-translation-context -o lang/finchgram.pot
for po in lang/*/LC_MESSAGES/finchgram.po; do msgmerge --update "$po" lang/finchgram.pot; done   # 需要 brew install gettext
```

然後填好 `msgstr`，重新 `cargo build`。英文是來源語言；目前附帶簡體中文。

## 發佈

release 只由 GitHub Actions 在乾淨的機器上、從打了 tag 的 commit 建置（`.github/workflows/release.yml`），
發佈在本儲存庫裡：壓縮好的 app、`SHA256SUMS` 和它的 Ed25519 簽名。已安裝的 app 從這裡自動更新，
驗證不通過的一律不安裝。維護者用一條指令發起 release：

```sh
scripts/release.sh patch    # 0.1.0 -> 0.1.1；也可以是 minor、major 或一個具體版本號
```

app 用專案自己的憑證簽名，不是 Apple Developer ID，所以下載的 app 第一次開啟時，macOS 會攔一次
（系統設定 → 隱私權與安全性）。app 自己安裝的更新不需要這一步。

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
