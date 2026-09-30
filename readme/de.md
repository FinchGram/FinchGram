# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · [Español](es.md) · [Português](pt.md) · **Deutsch** · [Français](fr.md) · [Русский](ru.md) · [日本語](ja.md) · [한국어](ko.md) · [العربية](ar.md)

Ein quelloffener Telegram-Desktop-Client mit Medienzentrale, geschrieben in Rust +
[Slint](https://slint.dev). Zuerst für macOS (Apple Silicon); Windows und Linux folgen später.

FinchGram nutzt die Telegram-API und ist Teil des Telegram-Ökosystems. Es ist ein inoffizieller Client
und stammt nicht von Telegram.

Stand: früh. Die Hülle startet TDLib und verfolgt, wie weit die Anmeldung ist; als Nächstes kommen die
Seiten, nach dem Design.

Die App ist eine Hülle (Shell). Telegram selbst übernimmt TDLib, die offizielle Bibliothek von Telegram,
die als eigenes Programm neben der ausführbaren Datei läuft: `finchgram-tdlib`, von diesem Repository
aus Quellen mit festgelegten Versionen gebaut (so wie ffmpeg bei Coova Studio). Die Hülle spricht mit ihm
nur über `src/telegram/`, im TDLib-eigenen JSON. Siehe [docs/architecture.md](../docs/architecture.md) und
[docs/conventions.md](../docs/conventions.md) (auf Englisch).

Alles ist hier, öffentlich: der Quellcode, die Builds der Abhängigkeiten (`vendor/`) und die Releases.

## Entwicklung

Entwickelt wird vorerst nur auf macOS mit Apple Silicon: finchgram-tdlib wird bisher nur dafür gebaut.
Linux und Windows folgen später.

finchgram-tdlib selbst liegt nicht in Git: `vendor/tdlib/` enthält nur das Skript, das es baut. Nach dem
Klonen das Programm einmal holen:

```sh
scripts/fetch-tdlib.sh    # lädt das festgelegte finchgram-tdlib-Release nach vendor/tdlib/bin/ und prüft die SHA-256
FINCHGRAM_API_ID=… FINCHGRAM_API_HASH=… cargo run
```

- Solange das erste `tdlib-*`-Release nicht veröffentlicht ist, meldet `scripts/fetch-tdlib.sh` das;
  dann finchgram-tdlib hier bauen (ein paar Minuten; braucht Xcode oder die Command Line Tools sowie
  `brew install cmake ninja gperf`, nur Build-Werkzeuge):

  ```sh
  vendor/tdlib/build.sh && vendor/tdlib/build.sh install
  ```

- `FINCHGRAM_API_ID` und `FINCHGRAM_API_HASH`: deine eigenen, von
  [my.telegram.org](https://my.telegram.org) → API development tools. Sie werden beim Kompilieren
  gelesen und gehören nie ins Repository. Ohne sie startet die App und meldet, dass sie keine API-ID hat.
- `FINCHGRAM_TEST_DC=1 cargo run` nutzt die Testserver von Telegram, die eigene Konten haben; die App
  führt für sie eine eigene Datenbank.

`build.rs` kopiert `vendor/tdlib/bin/finchgram-tdlib` neben die kompilierte ausführbare Datei, sodass
`cargo run` genau dasselbe Programm verwendet wie die verpackte App; fehlt es, bricht der Build mit einer
entsprechenden Meldung ab. Rust 1.92 oder neuer.

Die Datenbank von TDLib und die heruntergeladenen Dateien liegen in
`~/Library/Application Support/FinchGram/tdlib/`, die Einstellungen in
`~/Library/Application Support/FinchGram/settings.toml`.

## Aufbau

```
.github/workflows/
  release.yml            # baut die App bei jedem Push auf main; veröffentlicht v*-Tags als Releases
  tdlib.yml              # baut finchgram-tdlib auf einer sauberen Maschine; veröffentlicht tdlib-*-Tags als Releases
Cargo.toml
build.rs                 # kompiliert ui/app.slint, bündelt lang/, kopiert vendor/tdlib/bin/ neben die ausführbare Datei
docs/                    # architecture.md, conventions.md (+ zh-Hans)
lang/                    # Übersetzungen: lang/<Code>/LC_MESSAGES/finchgram.po, in die Binärdatei kompiliert
readme/                  # diese README in anderen Sprachen
release-signing.pub      # die öffentlichen Schlüssel, die Releases signieren dürfen; in die App kompiliert
scripts/
  bundle.sh              # baut dist/FinchGram.app (das, was der Release-Workflow ausführt)
  fetch-tdlib.sh         # das festgelegte finchgram-tdlib-Release nach vendor/tdlib/bin/
  release.sh             # startet ein Release: Version, Tag, Push; den Rest erledigt GitHub Actions
src/
  main.rs                # das Fenster, die Einstellungen, die Sprache, Updates; startet Telegram
  telegram/              # der einzige Code, der mit finchgram-tdlib spricht
    process.rs           #   führt das Programm aus: TDLib-JSON über Standardein- und -ausgabe
    api.rs               #   die TDLib-Typen, die FinchGram nutzt (td_api.tl der festgelegten Version)
    mod.rs               #   Anfragen und Antworten, Neustart, Anmeldung
  platform/              # was sich von Betriebssystem zu Betriebssystem unterscheidet
  update.rs              # die Selbstaktualisierung: GitHub Releases, Signatur, Austausch, Neustart
  settings.rs            # die Einstellungen des Nutzers (settings.toml)
  i18n.rs                # Sprache der Oberfläche: gespeicherte Wahl, sonst die des Systems, sonst Englisch
  bin/                   # finchgram-release-sign.rs, das Ed25519-Signierwerkzeug hinter den Releases
ui/
  app.slint              # das Hauptfenster
  state.slint            # die Globals, die Rust und die Seiten teilen
  theme.slint            # Farben, hell und dunkel
  logo/                  # das FinchGram-Logo (svg, png) und seine Regeln
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
OpenSSL (Apache License 2.0); deren Lizenztexte werden mit ihm ausgeliefert.
