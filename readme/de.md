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

- **Chats.** Anmelden mit Telefonnummer, Code und dem Passwort der zweistufigen Bestätigung oder durch
  Scannen eines QR-Codes; oder ein neues Konto anlegen. Die Chatliste mit Telegrams Ordnern,
  angehefteten Chats, Ungelesen-Zählern und Erwähnungen; eine Suche über alles (⌘K): deine Chats und
  Kontakte, öffentliche Benutzer, Gruppen und Kanäle per Benutzername, Personen per Telefonnummer und
  Nachrichten; die Suche im Chat (⌘F); eine Ansicht der Kanäle und eine der Bots. Der Stift legt eine
  neue Gruppe, einen neuen Kanal oder Chat an oder fügt einen Kontakt hinzu.
  Mehrere Konten, bis zu zehn, jedes mit eigenem TDLib: Wechsel über das Kontomenü oder mit ⌘1 … ⌘0.
  Gruppen und Kanäle verwalten: Foto, Name und Beschreibung, öffentlicher Benutzername oder Einladungslinks, Mitglieder und Administratoren mit ihren Rechten, Mitgliederrechte, langsamer Modus, Beitrittsfreigabe und Themen; Signaturen und Diskussionsgruppe eines Kanals.
- **Nachrichten.** Text mit Formatierung (fett, kursiv, Code, Links), Antworten und Weiterleitungen,
  Fotos, Videos, GIFs, Sticker, Dateien und eine Karte mit der Linkvorschau. Ein Rechtsklick auf eine
  Nachricht öffnet ihr Menü: antworten, bearbeiten, kopieren, Link kopieren, weiterleiten, melden,
  löschen oder mehrere auswählen. Fotos und Videos, die sich selbst zerstören, werden verschwommen
  gezeigt und einmal geöffnet, wie in Telegrams eigenen Apps. Die Schaltflächen der Bots unter ihren
  Nachrichten, ihre Tastaturen, Befehle und Menü-Schaltfläche; die angeheftete Nachricht des Chats
  unter der Kopfzeile. Ein Chat, der seinen Inhalt schützt, erscheint nicht in Bildschirmfotos und
  Bildschirmaufnahmen, solange er im Vordergrund steht. Reaktionen, mit einer Auswahl und einem
  Doppelklick für die Standardreaktion; runde Videonachrichten, an Ort und Stelle abgespielt;
  Kontaktkarten mit Nachricht, Kontakt hinzufügen und Teilen; das Teilen eines Beitrags oder einer
  Karte mit mehreren Chats auf einmal; Entwürfe, pro Chat und geräteübergreifend gespeichert;
  Gruppen mit Themen öffnen sich auf ihrer Themenliste; der Langsame Modus, im Eingabefeld
  heruntergezählt, und im Bereich der Chat-Einstellungen die Regeln einer Gruppe oder, für ihre
  Administratoren, ihre Berechtigungen und Optionen.
- **Medien.** Fotos und Videos öffnen sich in einem Betrachter über dem Fenster, mit dem Rest ihres
  Albums; Video läuft über mpv, aus den Quellen gebaut. Speichern in Downloads, wo der Chat es erlaubt.
- **Downloads.** Ein Download-Client im Mediencenter, über WebDAV, FTP, FTPS oder SFTP: dein NAS, eine
  Seedbox, PikPak, Alist/OpenList oder ein mit `vget serve` freigegebener Ordner, in der App durchstöbert. Dateien kommen über mehrere
  Verbindungen zugleich und laufen nach einer Unterbrechung weiter; Fotos und Videos öffnen sich im
  Betrachter, sobald sie da sind; Passwörter liegen versiegelt in der Datenbank von FinchGram. Siehe
  [docs/downloads.md](../docs/downloads.md).
- **Senden.** Fotos, Videos und Dateien über die Büroklammer, per Ziehen ins Fenster oder per
  Einfügen. Eine Karte zeigt sie vor dem Senden: mit Bildunterschrift, als Foto oder als Datei,
  zusammen als Album von bis zu zehn, mit Selbstzerstörungs-Timer in einem privaten Chat, ohne Ton.
- **Bildschirmfotos.** Die Schere im Eingabefeld oder ⌘⇧A friert den Bildschirm ein: ein Fenster
  nehmen oder eine Auswahl ziehen, Rechtecke, Ellipsen, Pfeile, Stiftstriche, Text und Mosaik
  darauf zeichnen, dann senden, kopieren oder speichern. Wie in WeChat, aus jedem Chat heraus.
- **Das Menü eines Chats.** Stummschalten, anheften, die ID kopieren, in einen Ordner legen; eine
  Person blockieren oder entblocken, melden, eine Gruppe oder einen Kanal verlassen, einen Chat
  löschen. Ein Rechtsklick auf einen Ordner markiert alle seine Chats als gelesen oder sortiert
  sie: nach der neuesten Nachricht, nach Namen oder nach ungelesenen Nachrichten; angeheftete
  bleiben oben.
- **Mitteilungen.** Neue Nachrichten kommen als macOS-Mitteilungen, ihre Zahl steht auf dem
  Dock-Symbol; FinchGram bleibt im Dock, wenn sein Fenster schließt, und kann in der Menüleiste wohnen.
- **Drei Themen.** Workbench, Broadsheet und Terminal, jedes hell und dunkel, ohne Neustart gewechselt.
- **Einstellungen.** Beim Anmelden starten, Senden mit Enter, das Bildschirmfoto-Kürzel, Töne und
  Vorschauen der Mitteilungen, zweistufige Bestätigung, die Sprache der Oberfläche und Updates: die App
  aktualisiert sich selbst aus GitHub Releases und installiert nichts, was sie nicht prüfen kann.
- **Sprachnachrichten.** Abspielen im Chat, mit Wellenform, Zeit und 1×, 1,5× oder 2×; das
  Anhören sieht der Absender, wie in Telegrams eigenen Apps. Zum Aufnehmen das Mikrofon halten:
  loslassen sendet, nach links ziehen bricht ab, nach oben ziehen sperrt die Aufnahme, und
  gestoppt lässt sie sich vor dem Senden anhören.
- **Spracheingabe.** Drück ✦ im Eingabefeld und sprich: Der Text erscheint, während du
  sprichst, und landet ohne Äh und Ähm im Feld, damit du ihn abschickst. Sie läuft über deinen
  eigenen OpenAI-Schlüssel (etwa 1 $ pro Stunde Sprache): Werkzeuge → Spracheingabe, aus, bis du
  sie einschaltest, und dort trägst du den Schlüssel ein; in jeder Sprache der Oberfläche und in
  Sätzen, die sie mischen. Siehe
  [docs/voice-input.md](../docs/voice-input.md).
- **KI-Assistenten.** Claude Desktop, Claude Code und andere Apps, die das Model Context Protocol
  sprechen, können über FinchGram deine Chats lesen und darin handeln, sobald du es erlaubst:
  Werkzeuge → KI-Zugriff, aus, bis du es einschaltest, mit den Chats, die du wählst, und ob ein
  Assistent senden darf; beim ersten Verbinden fragt eine App nach. Siehe
  [docs/mcp.md](../docs/mcp.md).
- **Sprachen.** Die Oberfläche und dieses README in elf Sprachen.

Noch nicht da: geheime Chats und Anrufe, Umfragen und geplante Nachrichten,
Schlüsselwortfilter, Windows und Linux. Siehe
[was als Nächstes kommt](../docs/architecture.md#not-now).

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

<p align="center"><img src="../docs/screenshots/workbench-dark-chats.png" width="800" alt="Workbench, dunkel"></p>

## Das Bildschirmfoto-Werkzeug

<p align="center"><img src="../docs/screenshots/workbench-light-shot-annotated.png" width="800" alt="Das Overlay für Bildschirmfotos, mit Anmerkungen"></p>

Drücke ⌘⇧A oder die Schere neben der Büroklammer. Der Bildschirm friert abgedunkelt unter einem
Overlay ein: du ziehst eine Auswahl, mit Lupe und der Größe in Pixeln, oder klickst ein Fenster an,
um es ganz zu nehmen. Die Werkzeugleiste zeichnet Rechtecke, Ellipsen, Pfeile, Stiftstriche, Text
und Mosaik, in drei Größen und sechs Farben, mit Rückgängig. Done legt das Bild in die Sendekarte,
wo eine Bildunterschrift dazukommen kann; ⌘C kopiert es; ⌘S speichert es in Downloads. Einstellungen
→ Allgemein → Bildschirmfotos hält das Kürzel und ob das FinchGram-Fenster sich derweil versteckt.

<p align="center"><img src="../docs/screenshots/workbench-light-shot-send-card.png" width="800" alt="Die Sendekarte mit einem Bildschirmfoto"></p>

## KI-Assistenten

<p align="center"><img src="../docs/screenshots/workbench-light-ai-access.png" width="800" alt="Werkzeuge → KI-Zugriff"></p>

FinchGram ist ein MCP-Server: Die App eines KI-Assistenten auf deinem Mac (Claude Desktop, Claude
Code, Cursor …) kann über die laufende App mit deinem eigenen Konto dein Telegram lesen und darin
handeln. Werkzeuge → KI-Zugriff ist der Schalter, standardmäßig aus. Dort wählst du, welche
Chats ein Assistent sehen darf (alle außer geheimen, oder nur gewählte) und was er schreiben darf
(Nachrichten in deinem Namen, nur Entwürfe, die du sendest, oder nichts), und du siehst, welche Apps
sich verbunden haben, mit dem Namen, den macOS für sie nennt, und was sie angefragt haben. Beim
ersten Verbinden fragt FinchGram, ob eine App erlaubt wird. Lesen markiert nichts als gelesen,
geheime Chats werden nie gezeigt, und nichts wird für eine KI aufbewahrt. Die Seite liefert die
Zeile zum Einfügen in Claude Code oder Claude Desktop; die Einzelheiten stehen in
[docs/mcp.md](../docs/mcp.md).

## Die Downloads-Seite

<p align="center"><img src="../docs/screenshots/workbench-light-downloads.png" width="800" alt="Die Downloads-Seite"></p>

Darstellung → Downloads, oder das Download-Symbol über dem Zahnrad. Füge einen Server über WebDAV, FTP, FTPS oder
SFTP hinzu (ein NAS, eine Seedbox, PikPak, Alist/OpenList, Nextcloud, `rclone serve webdav` oder einen mit
`vget serve` freigegebenen Ordner), durchstöbere seine Ordner, kreuze Dateien an oder nimm einen
ganzen Ordner, und sie landen in ~/Documents/FinchGram/webdav: jede Datei in Stücken von 8 MB über bis zu
zwölf Verbindungen, und nach „Erneut versuchen“ geht es dort weiter, wo es stehen geblieben ist,
wie bei vget, dem Kommandozeilenwerkzeug des Maintainers. Die Warteschlange rechts zeigt die
Geschwindigkeit und was noch aussteht; „Öffnen“ holt ein Foto oder Video und zeigt es im Betrachter.
FinchGram ändert nichts auf einem Server, solange du es für diesen Server nicht erlaubst (dann kann es dort löschen, verschieben, umbenennen und hochladen), und das Passwort kommt versiegelt in die Datenbank von FinchGram, nie in eine Klartextdatei.
[docs/downloads.md](../docs/downloads.md) hat die Einzelheiten und was noch fehlt (ein
selbstsigniertes Zertifikat wird abgelehnt: im LAN http nehmen, oder ein echtes Zertifikat).

## Wie es gebaut ist

Die App ist eine Hülle. Telegram selbst erledigt TDLib, das als eigenes Programm neben der
ausführbaren Datei läuft, `finchgram-tdlib`, das dieses Repository aus festgelegten Quellen baut; die
Hülle spricht mit ihm nur über `src/telegram/`, in TDLibs eigenem JSON. Video läuft über libmpv, auf
dieselbe Art gebaut. Nichts wird vom Rechner des Nutzers genommen, und ein Build lädt nichts herunter
außer diesen festgelegten, mit Prüfsummen versehenen Builds. Alles ist öffentlich: der Quellcode, die
Builds in `vendor/` und die Releases. Siehe [docs/architecture.md](../docs/architecture.md) und
[docs/conventions.md](../docs/conventions.md).

## Telegrams Regeln

FinchGram tut, was Telegrams eigene Apps tun, und nichts, was sie verbieten. Eine Nachricht gilt als
gelesen, wenn du sie siehst, die Gegenseite sieht dich tippen und online wie in jeder Telegram-App,
gesponserte Nachrichten in Kanälen werden gezeigt, selbstzerstörende Medien öffnen sich einmal, und
nichts von Telegram geht an irgendeine KI, außer du lässt einen Assistenten herein (Werkzeuge →
KI-Zugriff). Nichts verlässt deinen Rechner außer zu Telegram, zu GitHub für Updates und, während du
die Spracheingabe benutzt, deine Stimme mit deinem eigenen Schlüssel zu OpenAI.

## Entwicklung

Entwickelt wird vorerst nur auf macOS mit Apple Silicon: finchgram-tdlib wird bisher nur dafür gebaut.
Linux und Windows folgen später.

finchgram-tdlib, libmpv und die Schriften der Oberfläche liegen nicht in Git: `vendor/tdlib/` und
`vendor/mpv/` enthalten nur die Skripte, die sie bauen, und die Schriften kommen von Google Fonts. Nach
dem Klonen einmal holen:

```sh
scripts/fetch-tdlib.sh    # lädt das festgelegte finchgram-tdlib-Release nach vendor/tdlib/bin/ und prüft die SHA-256
scripts/fetch-mpv.sh      # lädt das festgelegte libmpv-Release nach vendor/mpv/bin/ und prüft die SHA-256
scripts/fetch-fonts.sh    # lädt die festgelegten Schriften der Oberfläche nach vendor/fonts/ und prüft ihre SHA-256
FINCHGRAM_API_ID=… FINCHGRAM_API_HASH=… cargo run
```

- Um finchgram-tdlib selbst zu ändern, es hier bauen (ein paar Minuten; braucht Xcode oder die
  Command Line Tools sowie `brew install cmake ninja gperf`, nur Build-Werkzeuge):

  ```sh
  vendor/tdlib/build.sh && vendor/tdlib/build.sh install
  ```

- `FINCHGRAM_API_ID` und `FINCHGRAM_API_HASH`: deine eigenen, von
  [my.telegram.org](https://my.telegram.org) → API development tools. Sie werden beim Kompilieren
  gelesen und gehören nie ins Repository. Ohne sie startet die App und meldet, dass sie keine API-ID hat.
- `FINCHGRAM_TEST_DC=1 cargo run` nutzt die Testserver von Telegram, die eigene Konten haben; die App
  führt für sie eine eigene Datenbank.
- `cargo test screenshots -- --ignored` zeichnet jede Seite jedes Designs, hell und dunkel, mit
  erfundenen Chats nach `target/screenshots/`: so lässt sich die Oberfläche ohne Konto ansehen.

`build.rs` kopiert `vendor/tdlib/bin/finchgram-tdlib` neben die kompilierte ausführbare Datei, sodass
`cargo run` genau dasselbe Programm verwendet wie die verpackte App; es bindet libmpv aus
`vendor/mpv/bin/` ein und kopiert sie ebenfalls dorthin, und die Schriften aus `vendor/fonts/` werden in
die ausführbare Datei kompiliert. Fehlt eines davon, bricht der Build mit einer entsprechenden Meldung ab.
Rust 1.92 oder neuer.

Die Datenbank von TDLib und die heruntergeladenen Dateien liegen in
`~/Library/Application Support/FinchGram/tdlib/`, die Einstellungen in
`~/Library/Application Support/FinchGram/settings.toml`.

## Aufbau

```
.github/workflows/
  mpv.yml                # baut libmpv auf einer sauberen Maschine; veröffentlicht mpv-*-Tags als Releases
  check.yml              # Clippy und die Tests bei jedem Push auf main
  release.yml            # baut und veröffentlicht die App für die Release-v*-Commits
  tdlib.yml              # baut finchgram-tdlib auf einer sauberen Maschine; veröffentlicht tdlib-*-Tags als Releases
Cargo.toml
build.rs                 # kompiliert ui/app.slint, bündelt lang/, kopiert vendor/tdlib/bin/ neben die ausführbare Datei
docs/                    # architecture.md, conventions.md, downloads.md, drag-and-drop.md, media-files.md, skia.md (+ zh-Hans)
  screenshots/           #   die Bilder, die die READMEs zeigen, aus dem Bildschirmfoto-Test (scripts/readme-pictures.sh)
lang/                    # Übersetzungen: lang/<Code>/LC_MESSAGES/finchgram.po, in die Binärdatei kompiliert
readme/                  # diese README in anderen Sprachen
release-signing.pub      # die öffentlichen Schlüssel, die Releases signieren dürfen; in die App kompiliert
scripts/
  bundle.sh              # baut dist/FinchGram.app (das, was der Release-Workflow ausführt)
  fetch-fonts.sh         # die festgelegten Schriften der Oberfläche nach vendor/fonts/
  fetch-mpv.sh           # das festgelegte libmpv-Release nach vendor/mpv/bin/
  fetch-tdlib.sh         # das festgelegte finchgram-tdlib-Release nach vendor/tdlib/bin/
  readme-pictures.sh     # kopiert die Bilder der READMEs von target/screenshots/ nach docs/screenshots/
  release.sh             # startet ein Release: Version, Tag, Push; den Rest erledigt GitHub Actions
src/
  main.rs                # das Fenster, die Einstellungen, Sprache und Design, Updates; startet Telegram
  fonts.rs               # die Schriften der Oberfläche, in die ausführbare Datei kompiliert
  images.rs              # Bilder in Nachrichten, außerhalb des UI-Threads dekodiert
  telegram/              # der einzige Code, der mit finchgram-tdlib spricht
    process.rs           #   führt das Programm aus: TDLib-JSON über Standardein- und -ausgabe
    api.rs               #   die TDLib-Typen, die FinchGram nutzt (td_api.tl der festgelegten Version)
    mod.rs               #   Anfragen und Antworten, Neustart; Updates gehen an den Store
    store.rs             #   was TDLib über Chats, Nutzer und Nachrichten gesagt hat; die Modelle der Seiten
    login.rs             #   die Anmeldung, die Registrierung
    chats.rs             #   die Chatliste
    conversation.rs      #   der offene Chat: Nachrichten, Schreiben
    actions.rs           #   was man mit einer Nachricht tun kann: Menü, Antworten, Weiterleiten, …
    account.rs           #   das Profil, Abmelden
    password.rs          #   die zweistufige Bestätigung in den Einstellungen
    files.rs             #   das Herunterladen von Dateien
    avatars.rs           #   die Fotos von Chats und Personen, statt ihrer Initialen
    viewer.rs            #   der Viewer: Fotos, Videos, in „Downloads“ sichern
    rich_text.rs         #   formatierter Text: fett, kursiv, Links …
    notifications.rs     #   Mitteilungen über neue Nachrichten, die Zahl der ungelesenen im Dock
    online.rs            #   das Konto ist online, solange das Fenster vorne ist und benutzt wird
  platform/              # was sich von Betriebssystem zu Betriebssystem unterscheidet
  player/                # Video mit libmpv, per OpenGL ins Fenster gezeichnet
  screenshot/            # das Bildschirmfoto-Werkzeug: Aufnahme, Overlay, Anmerkungen, das Bild
  downloads/             # die Downloads-Seite: WebDAV, FTP, FTPS und SFTP, mit einer Engine für mehrere Verbindungen
    remote.rs            #   ein Gesicht für die drei Clients: auflisten, prüfen, holen
    webdav.rs            #   PROPFIND-Listen, Adressen, Basic-Authentifizierung
    ftp.rs               #   FTP und FTPS über suppaftp: ein Strom, REST zum Weitermachen
    sftp.rs              #   SFTP über russh: Stücke über mehrere Sitzungen, Serverschlüssel gemerkt
    transfer.rs          #   eine Datei in Stücken über mehrere Verbindungen; nach einer Unterbrechung geht es weiter
    mod.rs               #   der Zustand der Seite, ihre Threads, die Warteschlange, das Formular für die Server
  update.rs              # die Selbstaktualisierung: GitHub Releases, Signatur, Austausch, Neustart
  settings.rs            # die Einstellungen des Nutzers (settings.toml)
  i18n.rs                # Sprache der Oberfläche: gespeicherte Wahl, sonst die des Systems, sonst Englisch
  screenshots.rs         # jede Seite als PNG gezeichnet (cargo test screenshots -- --ignored)
  bin/                   # finchgram-release-sign.rs, das Ed25519-Signierwerkzeug hinter den Releases
ui/
  app.slint              # das Hauptfenster: die Menüs, und welche Seite zu sehen ist
  state.slint            # der Zustand der App, geteilt von Rust und den Seiten
  telegram.slint         # was Telegram zeigt: das Konto, die Anmeldung, Chats, Nachrichten
  downloads.slint        # der Zustand der Downloads-Seite: Server, der Ordner, die Warteschlange
  look.slint             # Farben, Schrift und Formen des Designs, für die gemeinsamen Seiten
  format.slint           # Datum, Anzahlen und Arten von Nachrichten in der Sprache der Oberfläche
  widgets.slint          # kleine gemeinsame Teile; chat.slint: was die Chatfenster gemeinsam haben
  viewer.slint           # der Viewer über dem ganzen Fenster, im Stil jedes Designs
  pages/                 # die Seiten, die alle drei Designs teilen: Anmeldung, Einstellungen, Profil, Downloads
  workbench/             # das Chatfenster des Designs Workbench (Standard)
  broadsheet/            # das Chatfenster des Designs Broadsheet
  terminal/              # das Chatfenster des Designs Terminal
  icons/                 # Phosphor-Symbole (MIT), normal und zweifarbig; icons.slint listet sie auf
  logo/                  # das FinchGram-Logo (svg, png) und seine Regeln
vendor/fonts/            # nicht in Git: die Schriften der Oberfläche (scripts/fetch-fonts.sh)
vendor/mpv/              # libmpv: mpv und FFmpeg, die Videos abspielen
  build.sh               #   baut sie aus festgelegten Quellen: Versionen und SHA-256 ganz oben
  bin/                   #   nicht in Git: die Bibliothek (scripts/fetch-mpv.sh oder build.sh install)
vendor/tdlib/            # finchgram-tdlib
  build.sh               #   baut es aus festgelegten Quellen: TDLib-Commit, OpenSSL-Version und SHA-256 ganz oben
  host/                  #   unser kleines Host-Programm (main.cpp) und seine CMakeLists.txt
  bin/                   #   nicht in Git: das Programm, das die App nutzt (scripts/fetch-tdlib.sh oder build.sh install)
  work/, dist/           #   nicht in Git: Zwischendateien und Paket eines lokalen Builds
```

## Übersetzungen

Jeder Text der Oberfläche wird als `@tr("English text")` geschrieben. Ein String hat überall, wo er
vorkommt, dieselbe Übersetzung (build.rs schaltet Slints Standardkontext ab); braucht derselbe englische
Text an anderer Stelle andere Worte, bekommt er einen Kontext: `@tr("menu" => "Open")`. Mit dem
offiziellen Werkzeug extrahieren, dann in jede Sprache einarbeiten:

```sh
cargo install slint-tr-extractor                                          # einmalig
find ui -name '*.slint' | sort | xargs slint-tr-extractor --no-default-translation-context -o lang/finchgram.pot
for po in lang/*/LC_MESSAGES/finchgram.po; do msgmerge --update "$po" lang/finchgram.pot; done   # braucht brew install gettext
```

Danach die `msgstr`-Einträge ausfüllen und erneut `cargo build` ausführen. Englisch ist die
Ausgangssprache; die anderen zehn sind dabei.

## Releases

Releases baut ausschließlich GitHub Actions, aus einem getaggten Commit auf einer sauberen Maschine
(`.github/workflows/release.yml`), und veröffentlicht sie in diesem Repository: die gezippte App,
`SHA256SUMS` und deren Ed25519-Signatur. Installierte Kopien aktualisieren sich von dort selbst und
installieren nichts, was sie nicht prüfen können. Ein Maintainer startet ein Release mit einem Befehl:

```sh
scripts/release.sh patch    # 0.1.0 -> 0.1.1; auch minor, major oder eine genaue Version
```

Signiert wird mit der Apple Developer ID von North Brookville LLC, mit Hardened Runtime, und von
Apple notarisiert: Eine heruntergeladene Kopie öffnet sich ohne Nachfrage von Gatekeeper, und
Updates, die die App selbst installiert, ebenso.

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
