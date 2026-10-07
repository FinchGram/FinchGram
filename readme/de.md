# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · [Español](es.md) · [Português](pt.md) · **Deutsch** · [Français](fr.md) · [Русский](ru.md) · [日本語](ja.md) · [한국어](ko.md) · [العربية](ar.md)

Ein quelloffener Telegram-Desktop-Client mit Medienzentrale, geschrieben in Rust +
[Slint](https://slint.dev). Zuerst für macOS (Apple Silicon); Windows und Linux folgen später.

FinchGram nutzt die Telegram-API und ist Teil des Telegram-Ökosystems. Es ist ein inoffizieller Client
und stammt nicht von Telegram.

Stand: früh. Registrieren und Anmelden, die Chatliste und Chats mit Textnachrichten funktionieren, in
den drei Designs des Entwurfs: Workbench (Standard), Broadsheet und Terminal, umschaltbar unter
Einstellungen › Darstellung. Fotos und Videos erscheinen in den Chats und öffnen sich in einem Viewer,
der Videos mit mpv abspielt. Fotos, Videos und Dateien lassen sich über die Büroklammer senden, in das Fenster ziehen oder einfügen. Die Schere im Eingabefeld oder ⌘⇧A nimmt einen Ausschnitt des Bildschirms auf, mit Rechtecken, Pfeilen, Text und Mosaik darauf, und sendet ihn. Ein Rechtsklick auf eine Nachricht öffnet ihr Menü: antworten,
bearbeiten, kopieren, Link kopieren, weiterleiten, melden, löschen oder mehrere Nachrichten auswählen.
Die zweistufige Bestätigung wird unter Einstellungen › Datenschutz & Sicherheit verwaltet. Neue
Nachrichten kommen als macOS-Mitteilungen, ihre Zahl steht am Dock-Symbol (Einstellungen ›
Mitteilungen & Töne). Als Nächstes kommen Dateien, mehrere Konten, Stichwortfilter und geplante
Nachrichten.

Die App ist eine Hülle (Shell). Telegram selbst übernimmt TDLib, die offizielle Bibliothek von Telegram,
die als eigenes Programm neben der ausführbaren Datei läuft: `finchgram-tdlib`, von diesem Repository
aus Quellen mit festgelegten Versionen gebaut (so wie ffmpeg bei Coova Studio). Die Hülle spricht mit ihm
nur über `src/telegram/`, im TDLib-eigenen JSON. Siehe [docs/architecture.md](../docs/architecture.md) und
[docs/conventions.md](../docs/conventions.md) (auf Englisch).

Alles ist hier, öffentlich: der Quellcode, die Builds der Abhängigkeiten (`vendor/`) und die Releases.

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
  release.yml            # baut die App bei jedem Push auf main; veröffentlicht v*-Tags als Releases
  tdlib.yml              # baut finchgram-tdlib auf einer sauberen Maschine; veröffentlicht tdlib-*-Tags als Releases
Cargo.toml
build.rs                 # kompiliert ui/app.slint, bündelt lang/, kopiert vendor/tdlib/bin/ neben die ausführbare Datei
docs/                    # architecture.md, conventions.md, drag-and-drop.md (+ zh-Hans)
lang/                    # Übersetzungen: lang/<Code>/LC_MESSAGES/finchgram.po, in die Binärdatei kompiliert
readme/                  # diese README in anderen Sprachen
release-signing.pub      # die öffentlichen Schlüssel, die Releases signieren dürfen; in die App kompiliert
scripts/
  bundle.sh              # baut dist/FinchGram.app (das, was der Release-Workflow ausführt)
  fetch-fonts.sh         # die festgelegten Schriften der Oberfläche nach vendor/fonts/
  fetch-mpv.sh           # das festgelegte libmpv-Release nach vendor/mpv/bin/
  fetch-tdlib.sh         # das festgelegte finchgram-tdlib-Release nach vendor/tdlib/bin/
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
  update.rs              # die Selbstaktualisierung: GitHub Releases, Signatur, Austausch, Neustart
  settings.rs            # die Einstellungen des Nutzers (settings.toml)
  i18n.rs                # Sprache der Oberfläche: gespeicherte Wahl, sonst die des Systems, sonst Englisch
  screenshots.rs         # jede Seite als PNG gezeichnet (cargo test screenshots -- --ignored)
  bin/                   # finchgram-release-sign.rs, das Ed25519-Signierwerkzeug hinter den Releases
ui/
  app.slint              # das Hauptfenster: die Menüs, und welche Seite zu sehen ist
  state.slint            # der Zustand der App, geteilt von Rust und den Seiten
  telegram.slint         # was Telegram zeigt: das Konto, die Anmeldung, Chats, Nachrichten
  look.slint             # Farben, Schrift und Formen des Designs, für die gemeinsamen Seiten
  format.slint           # Datum, Anzahlen und Arten von Nachrichten in der Sprache der Oberfläche
  widgets.slint          # kleine gemeinsame Teile; chat.slint: was die Chatfenster gemeinsam haben
  viewer.slint           # der Viewer über dem ganzen Fenster, im Stil jedes Designs
  pages/                 # die Seiten, die alle drei Designs teilen: Anmeldung, Einstellungen, Profil
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
Ausgangssprache; derzeit ist vereinfachtes Chinesisch dabei.

## Releases

Releases baut ausschließlich GitHub Actions, aus einem getaggten Commit auf einer sauberen Maschine
(`.github/workflows/release.yml`), und veröffentlicht sie in diesem Repository: die gezippte App,
`SHA256SUMS` und deren Ed25519-Signatur. Installierte Kopien aktualisieren sich von dort selbst und
installieren nichts, was sie nicht prüfen können. Ein Maintainer startet ein Release mit einem Befehl:

```sh
scripts/release.sh patch    # 0.1.0 -> 0.1.1; auch minor, major oder eine genaue Version
```

Signiert wird vorerst ad hoc: Beim ersten Start einer heruntergeladenen Kopie fragt macOS einmal nach
(Systemeinstellungen → Datenschutz & Sicherheit). Updates, die die App selbst installiert, starten
ohne diesen Schritt.

## Lizenz

GPL-3.0 ([LICENSE](../LICENSE)). finchgram-tdlib enthält TDLib (Boost Software License 1.0) und
OpenSSL (Apache License 2.0); die Schriften der Oberfläche stehen unter der SIL Open Font License 1.1,
die Symbole unter der MIT-Lizenz. Deren Lizenztexte werden mit der App ausgeliefert.
