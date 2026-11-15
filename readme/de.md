# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · [Español](es.md) · [Português](pt.md) · **Deutsch** · [Français](fr.md) · [Русский](ru.md) · [日本語](ja.md) · [한국어](ko.md) · [العربية](ar.md)

<p align="center"><img src="../ui/logo/png/FinchGram-icon-128.png" width="96" alt="FinchGram"></p>

**Ein Telegram-Desktop-Client mit Superkräften, in Rust und [Slint](https://slint.dev).** Open Source
unter der GPL-3.0. Heute für macOS auf Apple Silicon; Windows und Linux später.

FinchGram spricht mit Telegram über [TDLib](https://core.telegram.org/tdlib), Telegrams eigene
Bibliothek, und hält sich an die [Telegram-API-Bedingungen](https://core.telegram.org/api/terms). Es ist
ein inoffizieller Client, nicht von Telegram gemacht.

<p align="center"><img src="../docs/screenshots/workbench-light-chats.png" width="800" alt="Das Chatfenster im Thema Workbench"></p>

## Was es kann

- **Telegram, wie in seinen eigenen Apps.** Chats mit Ordnern, Suche und bis zu zehn Konten; Nachrichten mit Antworten, Reaktionen, Bots, Themen und Entwürfen; Fotos, Videos, Dateien und Sprachnachrichten; die Verwaltung deiner Gruppen und Kanäle.
- **Ein Bildschirmfoto-Werkzeug.** ⌘⇧A friert den Bildschirm ein: ein Fenster oder eine Auswahl nehmen, darauf zeichnen, senden. Aus jedem Chat, wie in WeChat.
- **Downloads.** Ein Download-Client für deine eigenen Server über WebDAV, FTP, FTPS oder SFTP: ein NAS, eine Seedbox, PikPak. [Mehr](../docs/downloads.md)
- **Musik.** Ein Player für deine eigenen Server, die die Subsonic-API sprechen: Navidrome und Ähnliche. [Mehr](../docs/music.md)
- **Livestreams.** Owncast, PeerTube, Icecast und Telegrams eigene: entdecken, folgen und in der App ansehen. [Mehr](../docs/live-streams.md)
- **Spracheingabe.** Sprich, und die Worte landen im Eingabefeld, über den KI-Anbieter deiner Wahl und mit deinem eigenen Schlüssel. [Mehr](../docs/voice-input.md)
- **KI-Assistenten.** Claude und andere MCP-Apps lesen deine Chats und handeln darin über FinchGram, sobald du es erlaubst. [Mehr](../docs/mcp.md)
- **Drei Themen.** Workbench, Broadsheet und Terminal, jedes hell und dunkel. Terminal hat eine echte Befehlszeile. [So bedienst du sie](../docs/terminal.md)
- **Elf Sprachen**, und Updates, die die App selbst installiert, nachdem sie sie geprüft hat.

[Alles, was es kann, mit Bildern](../docs/features.md). Noch nicht da: geheime Chats und Anrufe, Umfragen und geplante Nachrichten, Schlüsselwortfilter, Windows und Linux. Siehe [was als Nächstes kommt](../docs/architecture.md#not-now).

## Download

Hole `FinchGram-<Version>-macos-arm64.zip` aus dem [neuesten Release](https://github.com/FinchGram/FinchGram/releases/latest),
entpacke es und lege FinchGram in Programme. Es braucht macOS 12 oder neuer auf Apple Silicon. Die App
ist mit der Developer ID von North Brookville LLC signiert und von Apple notarisiert, sie öffnet
sich also wie jede andere. Danach aktualisiert sie sich selbst.

Jedes Release kommt mit `SHA256SUMS` und dessen Ed25519-Signatur. Die App prüft beides, bevor sie ein
Update installiert; das kannst du auch, mit dem öffentlichen Schlüssel in
[release-signing.pub](../release-signing.pub).

## Drei Themen

| Workbench | Broadsheet | Terminal |
|---|---|---|
| ![Workbench](../docs/screenshots/workbench-light-chats.png) | ![Broadsheet](../docs/screenshots/broadsheet-light-chats.png) | ![Terminal](../docs/screenshots/terminal-light-chats.png) |

Workbench, die Voreinstellung, ist ein Arbeitsfenster mit Tabs. Broadsheet liest sich wie eine
Zeitung, mit einer Akzentfarbe je Konto. Terminal ist ein Bildschirm in Festbreitenschrift mit
Befehlen. Jedes hat eine helle und eine dunkle Seite, dem System folgend oder nach Wahl, in
Einstellungen → Erscheinungsbild.

Terminal führt Befehle in der Zeile am unteren Rand aus: ⌘K drücken oder `/` tippen. [Das Terminal-Thema](../docs/terminal.md) nennt die Befehle und die Tasten.

<p align="center"><img src="../docs/screenshots/workbench-dark-chats.png" width="800" alt="Workbench, dunkel"></p>

## Telegrams Regeln

FinchGram tut, was Telegrams eigene Apps tun, und nichts, was sie verbieten. Eine Nachricht gilt als
gelesen, wenn du sie siehst, die Gegenseite sieht dich tippen und online wie in jeder Telegram-App,
gesponserte Nachrichten in Kanälen werden gezeigt, selbstzerstörende Medien öffnen sich einmal, und
nichts von Telegram geht an irgendeine KI, außer du lässt einen Assistenten herein (Werkzeuge →
KI-Zugriff) oder schaltest das Transkribieren von Sprachnachrichten mit deinem eigenen KI-Anbieter ein.
Nichts verlässt deinen Rechner außer zu Telegram, zu GitHub für Updates und, während du die
Spracheingabe benutzt, deine Stimme mit deinem eigenen Schlüssel zu dem KI-Anbieter, den
du gewählt hast.

## Dokumentation

Die Dokumente gibt es auf Englisch und Chinesisch.

- [Was FinchGram kann](../docs/features.md): jede Funktion, mit Bildern.
- [Das Terminal-Thema](../docs/terminal.md): seine Befehlszeile und seine Befehle.
- [Aus dem Quellcode bauen](../docs/development.md), [der Aufbau des Repositorys](../docs/layout.md), [Übersetzungen](../docs/translations.md) und [Releases](../docs/releases.md).
- [Architektur](../docs/architecture.md) und [Konventionen](../docs/conventions.md): wie es gebaut ist, und warum.

## Entwicklung

Die App ist eine Hülle: Telegram selbst erledigt TDLib, das als eigenes Programm neben ihr läuft und von diesem Repository aus festgelegten Quellen gebaut wird, ebenso wie libmpv und Skia. Zum Bauen braucht es macOS auf Apple Silicon und eine eigene Telegram-API-ID; die Schritte stehen in [docs/development.md](../docs/development.md).

## Mitmachen

Issues und Pull Requests sind willkommen. Bevor du änderst, wie die Teile zusammenpassen, lies
[docs/architecture.md](../docs/architecture.md) und [docs/conventions.md](../docs/conventions.md): jede
Abhängigkeit baut dieses Repository aus festgelegten Quellen, die Oberfläche folgt dem Design in allen
drei Themen, und nichts in der App verstößt gegen die Telegram-API-Bedingungen. Halte `cargo test`,
`cargo clippy --all-targets` und `cargo test screenshots -- --ignored` sauber, und sieh dir die Bilder
an. Die Bilder der READMEs stammen ebenfalls aus diesem Test: `scripts/readme-pictures.sh` erneuert
`docs/screenshots/`.

## Lizenz

GPL-3.0 ([LICENSE](../LICENSE)). finchgram-tdlib enthält TDLib (Boost Software License 1.0) und
OpenSSL (Apache License 2.0); die Schriften der Oberfläche stehen unter der SIL Open Font License 1.1,
die Symbole unter der MIT-Lizenz. Deren Lizenztexte werden mit der App ausgeliefert.
