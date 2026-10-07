# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · [Español](es.md) · [Português](pt.md) · [Deutsch](de.md) · [Français](fr.md) · [Русский](ru.md) · **日本語** · [한국어](ko.md) · [العربية](ar.md)

<p align="center"><img src="../ui/logo/png/FinchGram-icon-128.png" width="96" alt="FinchGram"></p>

**メディアセンターを備えた Telegram デスクトップクライアント。Rust と [Slint](https://slint.dev) 製。** GPL-3.0 の
オープンソースです。いまは Apple silicon の macOS 向け、Windows と Linux は後から。

FinchGram は Telegram 公式のライブラリ [TDLib](https://core.telegram.org/tdlib) を通して Telegram と通信し、
[Telegram API 利用規約](https://core.telegram.org/api/terms)を守ります。非公式クライアントであり、Telegram 製ではありません。

<p align="center"><img src="../docs/screenshots/workbench-light-chats.png" width="800" alt="Workbench テーマのチャットウィンドウ"></p>

## できること

- **チャット。** 電話番号、認証コード、2 段階認証のパスワードでログイン、または QR コードをスキャンしてログイン。新規登録も
  できます。チャット一覧には Telegram のフォルダ、ピン留め、未読数、メンションが表示され、検索ボックス（⌘K）、チャンネル
  一覧、ボット一覧があります。
- **メッセージ。** 書式付きテキスト（太字、斜体、コード、リンク）、返信と転送、写真、動画、GIF、ステッカー、ファイル、
  リンクのプレビューカード。メッセージを右クリックするとメニューが開きます：返信、編集、コピー、リンクのコピー、転送、報告、
  削除、複数選択。自動消滅の写真と動画はぼかして表示され、一度だけ開けます。Telegram 公式アプリと同じです。
- **メディア。** 写真と動画はウィンドウ全体のビューアで開き、同じアルバムの他の写真も見られます。動画はソースから
  ビルドした mpv で再生します。チャットが許可していれば「ダウンロード」に保存できます。
- **送信。** 写真、動画、ファイルはクリップから送信できるほか、ウィンドウにドラッグするか貼り付けても送れます。送る前に
  カードで確認：キャプションを付ける、写真として送るかファイルとして送るか、最大 10 個をアルバムにまとめる、個人チャットでは
  自動消滅タイマー、サイレント送信。
- **スクリーンショット。** 入力欄のはさみ、または ⌘⇧A で画面を固定：ウィンドウを選ぶかドラッグで範囲を指定し、四角、楕円、
  矢印、ペン、文字、モザイクを描き込んで、そのまま送信、コピー、保存できます。WeChat と同じように、どのチャットからでも。
- **チャットのメニュー。** ミュート、ピン留め、既読にする、フォルダに入れる。相手のブロックと解除、報告、グループや
  チャンネルからの退出、チャットの削除。
- **通知。** 新着メッセージは macOS の通知で届き、その数が Dock アイコンに表示されます。ウィンドウを閉じても FinchGram は
  Dock に残り、メニューバーに置くこともできます。
- **3 つのテーマ。** Workbench、Broadsheet、Terminal。それぞれライトとダークがあり、再起動なしで切り替えられます。
- **設定。** ログイン時に起動、Enter で送信、スクリーンショットのショートカット、通知のサウンドとプレビュー、2 段階認証、
  表示言語、そして更新：アプリは GitHub Releases から自動更新し、検証できないものは一切インストールしません。
- **言語。** インターフェースは英語と簡体字中国語、この README は 11 言語。

まだないもの：秘密のチャット、音声メッセージと通話、複数アカウント、投票と予約送信、キーワードフィルター、Windows と Linux。
[次に来るもの](../docs/architecture.md#not-now)も参照してください。

## ダウンロード

[最新のリリース](https://github.com/FinchGram/FinchGram/releases/latest)から `FinchGram-<バージョン>-macos-arm64.zip` を
取得し、解凍して FinchGram を「アプリケーション」に移動します。Apple silicon の macOS 12 以降が必要です。アプリはまだ公証
されていないため、初回起動時に macOS が一度確認します：システム設定 → プライバシーとセキュリティ → 「このまま開く」。
以降は自動で更新されます。

すべてのリリースに `SHA256SUMS` とその Ed25519 署名が付きます。アプリは更新をインストールする前に両方を検証します。
[release-signing.pub](../release-signing.pub) の公開鍵で自分でも検証できます。

## 3 つのテーマ

| Workbench | Broadsheet | Terminal |
|---|---|---|
| ![Workbench](../docs/screenshots/workbench-light-chats.png) | ![Broadsheet](../docs/screenshots/broadsheet-light-chats.png) | ![Terminal](../docs/screenshots/terminal-light-chats.png) |

デフォルトの Workbench はタブ付きの作業ウィンドウ。Broadsheet は新聞のような読み心地で、アカウントごとのアクセントカラー。
Terminal は等幅フォントの画面でコマンドが使えます。どれもライトとダークがあり、システムに従うか自分で選べます
（設定 › 外観）。

<p align="center"><img src="../docs/screenshots/workbench-dark-chats.png" width="800" alt="Workbench、ダーク"></p>

## スクリーンショット機能

<p align="center"><img src="../docs/screenshots/workbench-light-shot-annotated.png" width="800" alt="書き込み付きのスクリーンショットオーバーレイ"></p>

⌘⇧A を押すか、クリップの隣のはさみをクリックします。画面がオーバーレイの下で固定され、ポインタの下のウィンドウがそのまま
選べるほか、ドラッグで範囲を指定できます。拡大鏡とピクセル単位のサイズ付き。ツールバーで四角、楕円、矢印、ペン、文字、
モザイクを 3 種類の太さと 6 色で描け、取り消しもできます。Done で画像が送信カードに入り、キャプションを添えられます。
⌘C でコピー、⌘S で「ダウンロード」に保存。設定 › 一般 › スクリーンショットでショートカットと、撮影中に FinchGram の
ウィンドウを隠すかどうかを変えられます。

<p align="center"><img src="../docs/screenshots/workbench-light-shot-send-card.png" width="800" alt="スクリーンショットを載せた送信カード"></p>

## 仕組み

アプリはシェルです。Telegram そのものは TDLib が担い、実行ファイルの隣で別プログラム `finchgram-tdlib` として動きます。
これはこのリポジトリがピン留めしたソースからビルドします。シェルは `src/telegram/` を通してのみ、TDLib 自身の JSON で
やり取りします。動画は同じ方法でビルドした libmpv で再生します。利用者のマシンから何も取らず、ビルド時にはこのピン留め済み・
チェックサム付きの成果物以外は何もダウンロードしません。ソース、vendor ビルド（`vendor/`）、リリース、すべてが公開です。
[docs/architecture.md](../docs/architecture.md) と [docs/conventions.md](../docs/conventions.md) を参照してください。

## Telegram のルール

FinchGram は Telegram 公式アプリがすることだけをし、禁じられていることはしません。メッセージは見た時点で既読になり、
相手には他の Telegram アプリと同じように入力中やオンラインが見え、チャンネルのスポンサーメッセージは表示され、自動消滅の
メディアは一度だけ開け、Telegram から得たものはどの AI にも渡しません。Telegram と、更新のための GitHub 以外に、
あなたのマシンから出ていくものはありません。

## 開発

現時点では、開発には Apple silicon の macOS が必要です。finchgram-tdlib はまだそれ向けにしかビルドしていません。
Linux と Windows は後日対応します。

finchgram-tdlib、libmpv、UI フォントは git に入っていません。`vendor/tdlib/` と `vendor/mpv/` にあるのは
それらをビルドするスクリプトだけで、フォントは Google Fonts から取得します。clone したら、まず一度取得してください：

```sh
scripts/fetch-tdlib.sh    # 固定バージョンの finchgram-tdlib リリースを vendor/tdlib/bin/ にダウンロードし、SHA-256 を検証
scripts/fetch-mpv.sh      # 固定バージョンの libmpv リリースを vendor/mpv/bin/ にダウンロードし、SHA-256 を検証
scripts/fetch-fonts.sh    # 固定バージョンの UI フォントを vendor/fonts/ にダウンロードし、SHA-256 を検証
FINCHGRAM_API_ID=… FINCHGRAM_API_HASH=… cargo run
```

- finchgram-tdlib 自体を変更するときは、ここでビルドしてください（数分かかります。Xcode または
  Command Line Tools と、`brew install cmake ninja gperf` が必要です。いずれもビルド用のツールです）：

  ```sh
  vendor/tdlib/build.sh && vendor/tdlib/build.sh install
  ```

- `FINCHGRAM_API_ID` と `FINCHGRAM_API_HASH`：自分のものを
  [my.telegram.org](https://my.telegram.org) → API development tools で取得します。コンパイル時に読み込まれ、
  リポジトリには決して入れません。これがなくてもアプリは起動し、API ID がないことを表示します。
- `FINCHGRAM_TEST_DC=1 cargo run` は Telegram のテストサーバーを使います。テストサーバーには専用のアカウントがあり、
  アプリはそのために別のデータベースを持ちます。
- `cargo test screenshots -- --ignored` は、すべてのテーマのすべてのページを、ライトとダークで、架空の
  チャットとともに `target/screenshots/` に描き出します。アカウントなしで UI を確認する方法です。

`build.rs` は `vendor/tdlib/bin/finchgram-tdlib` を、コンパイルされた実行ファイルの隣にコピーします。そのため
`cargo run` は、パッケージ化されたアプリとまったく同じプログラムを使います。libmpv は `vendor/mpv/bin/` から
リンクし、同じく隣にコピーします。`vendor/fonts/` のフォントは実行ファイルに組み込まれます。どれかが欠けていると、
ビルドはその旨のメッセージを出して失敗します。Rust 1.92 以降が必要です。

TDLib のデータベースとダウンロードしたファイルは `~/Library/Application Support/FinchGram/tdlib/` に、
設定は `~/Library/Application Support/FinchGram/settings.toml` にあります。

## 構成

```
.github/workflows/
  mpv.yml                # クリーンなマシンで libmpv をビルドし、mpv-* タグをリリースとして公開
  release.yml            # main への push ごとにアプリをビルドし、v* タグをリリースとして公開
  tdlib.yml              # クリーンなマシンで finchgram-tdlib をビルドし、tdlib-* タグをリリースとして公開
Cargo.toml
build.rs                 # ui/app.slint をコンパイルし、lang/ を同梱し、vendor/tdlib/bin/ を実行ファイルの隣にコピー
docs/                    # architecture.md、conventions.md、drag-and-drop.md（+ zh-Hans）
  screenshots/           #   README に載せる画像。スクリーンショットテストから（scripts/readme-pictures.sh）
lang/                    # 翻訳：lang/<コード>/LC_MESSAGES/finchgram.po、バイナリに組み込まれる
readme/                  # この README の他の言語版
release-signing.pub      # リリースへの署名を許された公開鍵。アプリに組み込まれる
scripts/
  bundle.sh              # dist/FinchGram.app をビルド（リリースのワークフローが実行するもの）
  fetch-fonts.sh         # 固定バージョンの UI フォントを vendor/fonts/ へ
  fetch-mpv.sh           # 固定バージョンの libmpv リリースを vendor/mpv/bin/ へ
  fetch-tdlib.sh         # 固定バージョンの finchgram-tdlib リリースを vendor/tdlib/bin/ へ
  readme-pictures.sh     # README の画像を target/screenshots/ から docs/screenshots/ にコピー
  release.sh             # リリースを開始：バージョン、タグ、push。残りは GitHub Actions が行う
src/
  main.rs                # ウィンドウ、設定、言語とテーマ、アップデート。Telegram を起動
  fonts.rs               # UI フォント。実行ファイルに組み込まれる
  images.rs              # メッセージの画像。UI スレッドの外でデコード
  telegram/              # finchgram-tdlib と通信する唯一のコード
    process.rs           #   プログラムを実行：標準入出力で TDLib の JSON をやり取り
    api.rs               #   FinchGram が使う TDLib の型（固定バージョンの td_api.tl に準拠）
    mod.rs               #   リクエストと応答、再起動。アップデートは store へ
    store.rs             #   TDLib から届いたチャット・ユーザー・メッセージ。ページのモデル
    login.rs             #   ログイン、新規登録
    chats.rs             #   チャット一覧
    conversation.rs      #   開いているチャット：メッセージ、送信
    actions.rs           #   メッセージの操作：右クリックメニュー、返信、転送など
    account.rs           #   プロフィール、ログアウト
    password.rs          #   設定の 2 段階認証
    files.rs             #   ファイルのダウンロード
    avatars.rs           #   チャットと相手の写真。頭文字の代わりに表示
    viewer.rs            #   メディアビューア：写真、動画、「ダウンロード」への保存
    rich_text.rs         #   書式付きテキスト：太字、斜体、リンクなど
    notifications.rs     #   新着メッセージの通知、Dock アイコンの未読数
    online.rs            #   ウィンドウが前面で使われている間、アカウントはオンライン
  platform/              # OS ごとに異なる部分
  player/                # libmpv による動画再生。OpenGL でウィンドウに描画
  screenshot/            # スクリーンショット機能：撮影、オーバーレイ、書き込み、画像
  update.rs              # 自動アップデート：GitHub Releases、署名の検証、入れ替え、再起動
  settings.rs            # ユーザーの設定（settings.toml）
  i18n.rs                # UI の言語：保存された選択、なければシステムの言語、それもなければ英語
  screenshots.rs         # すべてのページを PNG に描画（cargo test screenshots -- --ignored）
  bin/                   # finchgram-release-sign.rs、リリースを支える Ed25519 署名ツール
ui/
  app.slint              # メインウィンドウ：メニューと、どのページを表示するか
  state.slint            # アプリの状態。Rust とページで共有
  telegram.slint         # Telegram の内容：アカウント、ログイン、チャット、メッセージ
  look.slint             # テーマの色・書体・形。共有ページ用
  format.slint           # UI の言語で書く日付・数・メッセージの種類
  widgets.slint          # 共有の小さな部品。chat.slint はチャットウィンドウの共有部分
  viewer.slint           # ウィンドウ全体を覆うメディアビューア。テーマごとのスタイル
  pages/                 # 3 つのテーマで共有するページ：ログイン、設定、プロフィール
  workbench/             # ワークベンチテーマのチャットウィンドウ（既定）
  broadsheet/            # ブロードシートテーマのチャットウィンドウ
  terminal/              # ターミナルテーマのチャットウィンドウ
  icons/                 # Phosphor アイコン（MIT）、通常とデュオトーン。icons.slint が一覧
  logo/                  # FinchGram のロゴ（svg、png）と使用ルール
vendor/fonts/            # git には入らない：UI フォント（scripts/fetch-fonts.sh）
vendor/mpv/              # libmpv：動画再生用の mpv と FFmpeg
  build.sh               #   固定したソースからビルド：バージョンと SHA-256 は冒頭で固定
  bin/                   #   git 管理外：ライブラリ本体（scripts/fetch-mpv.sh または build.sh install）
vendor/tdlib/            # finchgram-tdlib
  build.sh               #   固定したソースからビルド：TDLib のコミット、OpenSSL のバージョンと SHA-256 は冒頭で固定
  host/                  #   小さなホストプログラム（main.cpp）とその CMakeLists.txt
  bin/                   #   git 管理外：アプリが使うプログラム（scripts/fetch-tdlib.sh または build.sh install）
  work/, dist/           #   git 管理外：ローカルビルドの中間ファイルとパッケージ
```

## 翻訳

UI のテキストはすべて `@tr("English text")` と書きます。同じ文字列は、どこに出てきても訳は一つです（build.rs が
Slint のデフォルトコンテキストを無効にしています）。同じ英文でも別の場所で違う訳が必要なら、コンテキストを付けます：
`@tr("menu" => "Open")`。公式ツールで抽出し、各言語にマージします：

```sh
cargo install slint-tr-extractor                                          # 一度だけ
find ui -name '*.slint' | sort | xargs slint-tr-extractor --no-default-translation-context -o lang/finchgram.pot
for po in lang/*/LC_MESSAGES/finchgram.po; do msgmerge --update "$po" lang/finchgram.pot; done   # brew install gettext が必要
```

その後 `msgstr` を埋めて、もう一度 `cargo build` します。原文の言語は英語で、現在は簡体字中国語を同梱しています。

## リリース

リリースは GitHub Actions だけが、タグの付いたコミットからクリーンなマシンでビルドし
（`.github/workflows/release.yml`）、このリポジトリで公開します：zip にしたアプリ、`SHA256SUMS`、
その Ed25519 署名。インストール済みのアプリはここから自動でアップデートし、検証できないものは
一切インストールしません。メンテナーはコマンド一つでリリースを始めます：

```sh
scripts/release.sh patch    # 0.1.0 -> 0.1.1。minor、major、または正確なバージョンも可
```

今のところ署名は ad hoc です。ダウンロードしたアプリを初めて開くとき、macOS が一度だけ確認します
（システム設定 → プライバシーとセキュリティ）。アプリ自身がインストールしたアップデートは、この手順なしで起動します。

## 貢献

Issue と pull request を歓迎します。構造に関わる変更の前に [docs/architecture.md](../docs/architecture.md) と
[docs/conventions.md](../docs/conventions.md) を読んでください：すべての依存はこのリポジトリがピン留めしたソースから
ビルドし、UI は 3 つのテーマすべてでデザインに従い、アプリに Telegram API 利用規約に反するものは入れません。`cargo test`、
`cargo clippy --all-targets`、`cargo test screenshots -- --ignored` をクリーンに保ち、描かれた画像を確認してください。
README の画像もこのテストから来ています：`scripts/readme-pictures.sh` が `docs/screenshots/` を更新します。

## ライセンス

GPL-3.0（[LICENSE](../LICENSE)）。finchgram-tdlib には TDLib（Boost Software License 1.0）と
OpenSSL（Apache License 2.0）が含まれます。UI フォントは SIL Open Font License 1.1、アイコンは MIT
ライセンスです。それぞれのライセンス文はアプリに同梱されています。
