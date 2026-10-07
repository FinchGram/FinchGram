# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · **繁體中文** · [Español](es.md) · [Português](pt.md) · [Deutsch](de.md) · [Français](fr.md) · [Русский](ru.md) · [日本語](ja.md) · [한국어](ko.md) · [العربية](ar.md)

一個開源的 Telegram 桌面用戶端，附帶媒體中心，以 Rust + [Slint](https://slint.dev) 編寫。
先做 macOS（Apple silicon），之後再做 Windows 和 Linux。

FinchGram 使用 Telegram API，是 Telegram 生態系的一部分。它是非官方用戶端，並非 Telegram 出品。

狀態：早期。註冊和登入、聊天列表和文字訊息的收發已經可用，介面是設計稿的三套主題：工作台（預設）、報紙和
終端，在「設定 › 外觀」裡切換。聊天裡的圖片和影片可以直接看，點開後在檢視器裡瀏覽，影片用 mpv 播放。照片、影片和檔案可以從迴紋針選單傳送，也可以拖進視窗或直接貼上。輸入列裡的剪刀（或 ⌘⇧A）可以截取螢幕的一部分，畫上方框、箭頭、文字和馬賽克後直接傳送。
在訊息上按右鍵可以回覆、編輯、複製、複製連結、轉傳、檢舉、刪除，或選取多則訊息。兩步驟
驗證在「設定 › 隱私與安全」裡管理。新訊息會以 macOS 通知提醒，未讀數顯示在 Dock 圖示上（「設定 › 通知與音效」）。
接下來做檔案、多帳號、關鍵字隱藏和定時訊息。

app 是一個外殼。Telegram 本身交給 TDLib（Telegram 官方的程式庫），它以獨立程式的形式在執行檔旁邊執行：
`finchgram-tdlib`，由本儲存庫從鎖定版本的原始碼建置（就像 Coova Studio 的 ffmpeg）。外殼只透過 `src/telegram/`
與它溝通，使用的是 TDLib 自己的 JSON。請見 [docs/architecture.zh-Hans.md](../docs/architecture.zh-Hans.md) 和
[docs/conventions.zh-Hans.md](../docs/conventions.zh-Hans.md)（簡體中文）。

所有東西都在這裡、全部公開：原始碼、vendor 建置（`vendor/`）和發佈版本。

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
lang/                    # 翻譯：lang/<代碼>/LC_MESSAGES/finchgram.po，編譯進二進位檔
readme/                  # 這份 README 的其他語言版本
release-signing.pub      # 允許為 release 簽名的公鑰，編譯進 app
scripts/
  bundle.sh              # 建置 dist/FinchGram.app（release 流程執行的就是它）
  fetch-fonts.sh         # 把鎖定的介面字型下載到 vendor/fonts/
  fetch-mpv.sh           # 把鎖定的 libmpv release 下載到 vendor/mpv/bin/
  fetch-tdlib.sh         # 把鎖定的 finchgram-tdlib release 下載到 vendor/tdlib/bin/
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

目前是 ad-hoc 簽名：下載的 app 第一次開啟時，macOS 會攔一次（系統設定 → 隱私權與安全性）。
app 自己安裝的更新不需要這一步。

## 授權條款

GPL-3.0（[LICENSE](../LICENSE)）。finchgram-tdlib 包含 TDLib（Boost Software License 1.0）和
OpenSSL（Apache License 2.0）；介面字型採用 SIL Open Font License 1.1，圖示採用 MIT 授權。
它們的授權條款文字都隨 app 一起發佈。
