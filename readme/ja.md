# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · [Español](es.md) · [Português](pt.md) · [Deutsch](de.md) · [Français](fr.md) · [Русский](ru.md) · **日本語** · [한국어](ko.md) · [العربية](ar.md)

メディアセンターを備えた、オープンソースの Telegram デスクトップクライアントです。
Rust + [Slint](https://slint.dev) で書かれています。まず macOS（Apple silicon）向けで、Windows と Linux は後日対応します。

FinchGram は Telegram API を使っており、Telegram エコシステムの一部です。非公式のクライアントで、
Telegram が開発したものではありません。

状況：初期段階。新規登録とログイン、チャット一覧、テキストメッセージのやり取りが、デザインの 3 つのテーマで動きます：
ワークベンチ（既定）、ブロードシート、ターミナル。切り替えは「設定 › 外観」から。写真と動画はチャットに表示され、
ビューアで開けます（動画は mpv で再生）。2 段階認証は「設定 › プライバシーとセキュリティ」で管理できます。
次はファイル、複数アカウント、キーワードフィルター、予約メッセージです。

アプリはシェル（外殻）です。Telegram そのものは Telegram 公式のライブラリ TDLib が担い、実行ファイルの隣で
独立したプログラムとして動きます。それが `finchgram-tdlib` で、このリポジトリがバージョンを固定したソースから
ビルドしています（Coova Studio における ffmpeg と同じです）。シェルは `src/telegram/` を通してのみ、TDLib 独自の
JSON でこれと通信します。詳しくは [docs/architecture.md](../docs/architecture.md) と
[docs/conventions.md](../docs/conventions.md)（英語）を参照してください。

すべてがここで公開されています：ソースコード、依存関係のビルド（`vendor/`）、リリース。

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
docs/                    # architecture.md、conventions.md（+ zh-Hans）
lang/                    # 翻訳：lang/<コード>/LC_MESSAGES/finchgram.po、バイナリに組み込まれる
readme/                  # この README の他の言語版
release-signing.pub      # リリースへの署名を許された公開鍵。アプリに組み込まれる
scripts/
  bundle.sh              # dist/FinchGram.app をビルド（リリースのワークフローが実行するもの）
  fetch-fonts.sh         # 固定バージョンの UI フォントを vendor/fonts/ へ
  fetch-mpv.sh           # 固定バージョンの libmpv リリースを vendor/mpv/bin/ へ
  fetch-tdlib.sh         # 固定バージョンの finchgram-tdlib リリースを vendor/tdlib/bin/ へ
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
    account.rs           #   プロフィール、ログアウト
    password.rs          #   設定の 2 段階認証
    files.rs             #   ファイルのダウンロード
    viewer.rs            #   メディアビューア：写真、動画、「ダウンロード」への保存
  platform/              # OS ごとに異なる部分
  player/                # libmpv による動画再生。OpenGL でウィンドウに描画
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

## ライセンス

GPL-3.0（[LICENSE](../LICENSE)）。finchgram-tdlib には TDLib（Boost Software License 1.0）と
OpenSSL（Apache License 2.0）が含まれます。UI フォントは SIL Open Font License 1.1、アイコンは MIT
ライセンスです。それぞれのライセンス文はアプリに同梱されています。
