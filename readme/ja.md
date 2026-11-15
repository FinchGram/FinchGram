# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · [Español](es.md) · [Português](pt.md) · [Deutsch](de.md) · [Français](fr.md) · [Русский](ru.md) · **日本語** · [한국어](ko.md) · [العربية](ar.md)

<p align="center"><img src="../ui/logo/png/FinchGram-icon-128.png" width="96" alt="FinchGram"></p>

**超能力を備えた Telegram デスクトップクライアント。Rust と [Slint](https://slint.dev) 製。** GPL-3.0 の
オープンソースです。いまは Apple silicon の macOS 向け、Windows と Linux は後から。

FinchGram は Telegram 公式のライブラリ [TDLib](https://core.telegram.org/tdlib) を通して Telegram と通信し、
[Telegram API 利用規約](https://core.telegram.org/api/terms)を守ります。非公式クライアントであり、Telegram 製ではありません。

<p align="center"><img src="../docs/screenshots/workbench-light-chats.png" width="800" alt="Workbench テーマのチャットウィンドウ"></p>

## できること

- **Telegram 公式アプリと同じことが、ひととおり。** フォルダ・検索・最大 10 アカウントのチャット。返信・リアクション・ボット・トピック・下書きのあるメッセージ。写真・動画・ファイル・音声メッセージ。自分のグループとチャンネルの管理。
- **スクリーンショット機能。** ⌘⇧A で画面を止め、ウィンドウか範囲を選んで、書き込みをして送信。WeChat のように、どのチャットからでも。
- **ダウンロード。** 自分のサーバー用のダウンロードクライアント。WebDAV・FTP・FTPS・SFTP に対応（NAS、シードボックス、PikPak）。[詳しく](../docs/downloads.md)
- **音楽。** Subsonic API に対応した自分のサーバー（Navidrome など）のためのプレーヤー。[詳しく](../docs/music.md)
- **ライブ配信。** Owncast、PeerTube、Icecast と Telegram 自身の配信を、探して、フォローして、アプリの中で視聴。[詳しく](../docs/live-streams.md)
- **音声入力。** 話した言葉が入力欄に入ります。使う AI プロバイダーは自分で選び、キーも自分のもの。[詳しく](../docs/voice-input.md)
- **AI アシスタント。** Claude などの MCP 対応アプリが、あなたが許可すれば、FinchGram を通してチャットを読み、操作します。[詳しく](../docs/mcp.md)
- **3 つのテーマ。** Workbench、Broadsheet、Terminal。それぞれライトとダーク。Terminal には本当に使えるコマンドラインがあります。[使い方](../docs/terminal.md)
- **11 の言語**。アップデートは、検証してからアプリが自分でインストールします。

[できることのすべて（画像付き）](../docs/features.md)。まだないもの：秘密のチャットと通話、投票と予約送信、キーワードフィルター、Windows と Linux。 [次に来るもの](../docs/architecture.md#not-now)も参照してください。

## ダウンロード

[最新のリリース](https://github.com/FinchGram/FinchGram/releases/latest)から `FinchGram-<バージョン>-macos-arm64.zip` を
取得し、解凍して FinchGram を「アプリケーション」に移動します。Apple silicon の macOS 12 以降が必要です。アプリは North
Brookville LLC の Developer ID で署名され、Apple の公証を受けているので、ほかのアプリと同じようにそのまま開けます。
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

Terminal は画面いちばん下の行でコマンドを実行します。⌘K を押すか、`/` を入力してください。コマンドとキーは [Terminal テーマ](../docs/terminal.md)にあります。

<p align="center"><img src="../docs/screenshots/workbench-dark-chats.png" width="800" alt="Workbench、ダーク"></p>

## Telegram のルール

FinchGram は Telegram 公式アプリがすることだけをし、禁じられていることはしません。メッセージは見た時点で既読になり、
相手には他の Telegram アプリと同じように入力中やオンラインが見え、チャンネルのスポンサーメッセージは表示され、自動消滅の
メディアは一度だけ開け、Telegram から得たものは、ツールボックス → AI アクセスであなたがアシスタントを許可するか、ボイスメッセージを自分の AI プロバイダーで文字起こしする設定をオンにしない限り、どの AI にも渡しません。Telegram と、更新のための GitHub と、音声入力を使っているあいだあなた自身のキーで、選んだ AI プロバイダーに送る声のほかに、
あなたのマシンから出ていくものはありません。

## ドキュメント

ドキュメントは英語と中国語です。

- [FinchGram にできること](../docs/features.md)：すべての機能を画像付きで。
- [Terminal テーマ](../docs/terminal.md)：コマンドラインとコマンド一覧。
- [ソースからのビルド](../docs/development.md)、[リポジトリの構成](../docs/layout.md)、[翻訳](../docs/translations.md)、[リリース](../docs/releases.md)。
- [アーキテクチャ](../docs/architecture.md)と[規約](../docs/conventions.md)：どう作られているか、そしてなぜか。

## 開発

アプリは殻です。Telegram そのものは TDLib が担当し、アプリの隣で別プログラムとして動きます。TDLib はこのリポジトリが固定したソースからビルドしたもので、libmpv と Skia も同じです。ビルドには Apple silicon の macOS と自分の Telegram API ID が必要です。手順は [docs/development.md](../docs/development.md) にあります。

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
