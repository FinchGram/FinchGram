# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · [Español](es.md) · [Português](pt.md) · [Deutsch](de.md) · **Français** · [Русский](ru.md) · [日本語](ja.md) · [한국어](ko.md) · [العربية](ar.md)

Un client de bureau Telegram open source, avec un centre multimédia, écrit en Rust +
[Slint](https://slint.dev). D'abord pour macOS (Apple silicon) ; Windows et Linux ensuite.

FinchGram utilise l'API Telegram et fait partie de l'écosystème Telegram. C'est un client non officiel,
qui n'est pas développé par Telegram.

État : débuts. L'enveloppe démarre TDLib et suit l'avancement de la connexion ; les pages viennent
ensuite, d'après la maquette.

L'application est une enveloppe (*shell*). Telegram lui-même est pris en charge par TDLib, la
bibliothèque officielle de Telegram, qui tourne comme un programme séparé à côté de l'exécutable :
`finchgram-tdlib`, compilé par ce dépôt à partir de sources aux versions figées (comme ffmpeg pour
Coova Studio). L'enveloppe ne lui parle qu'au travers de `src/telegram/`, dans le JSON propre à TDLib.
Voir [docs/architecture.md](../docs/architecture.md) et [docs/conventions.md](../docs/conventions.md)
(en anglais).

Tout est ici, en public : le code source, les compilations des dépendances (`vendor/`) et les versions
publiées.

## Développement

Pour l'instant, le développement nécessite macOS sur Apple silicon : finchgram-tdlib n'est compilé que
pour cette plateforme. Linux et Windows suivront.

finchgram-tdlib lui-même n'est pas dans git : `vendor/tdlib/` ne contient que le script qui le compile.
Après le clonage, récupérez le programme une fois :

```sh
scripts/fetch-tdlib.sh    # télécharge la release figée de finchgram-tdlib dans vendor/tdlib/bin/ et vérifie le SHA-256
FINCHGRAM_API_ID=… FINCHGRAM_API_HASH=… cargo run
```

- Tant que la première release `tdlib-*` n'est pas publiée, `scripts/fetch-tdlib.sh` le signale ;
  compilez alors finchgram-tdlib ici (quelques minutes ; il faut Xcode ou les Command Line Tools, et
  `brew install cmake ninja gperf`, uniquement des outils de compilation) :

  ```sh
  vendor/tdlib/build.sh && vendor/tdlib/build.sh install
  ```

- `FINCHGRAM_API_ID` et `FINCHGRAM_API_HASH` : les vôtres, obtenus sur
  [my.telegram.org](https://my.telegram.org) → API development tools. Ils sont lus à la compilation et
  n'ont jamais leur place dans le dépôt. Sans eux, l'application démarre et indique qu'elle n'a pas
  d'API ID.
- `FINCHGRAM_TEST_DC=1 cargo run` utilise les serveurs de test de Telegram, qui ont leurs propres
  comptes ; l'application garde pour eux une base de données distincte.

`build.rs` copie `vendor/tdlib/bin/finchgram-tdlib` à côté de l'exécutable compilé : `cargo run` utilise
ainsi exactement le même programme que l'application empaquetée ; sans lui, la compilation échoue avec
un message qui l'explique. Rust 1.92 ou plus récent.

La base de données de TDLib et les fichiers téléchargés se trouvent dans
`~/Library/Application Support/FinchGram/tdlib/`, les réglages dans
`~/Library/Application Support/FinchGram/settings.toml`.

## Organisation

```
.github/workflows/
  release.yml            # compile l'application à chaque push sur main ; publie les tags v* comme releases
  tdlib.yml              # compile finchgram-tdlib sur une machine propre ; publie les tags tdlib-* comme releases
Cargo.toml
build.rs                 # compile ui/app.slint, intègre lang/, copie vendor/tdlib/bin/ à côté de l'exécutable
docs/                    # architecture.md, conventions.md (+ zh-Hans)
lang/                    # traductions : lang/<code>/LC_MESSAGES/finchgram.po, compilées dans le binaire
readme/                  # ce README dans d'autres langues
release-signing.pub      # les clés publiques autorisées à signer les releases ; compilées dans l'application
scripts/
  bundle.sh              # construit dist/FinchGram.app (ce qu'exécute le workflow de release)
  fetch-tdlib.sh         # la release figée de finchgram-tdlib dans vendor/tdlib/bin/
  release.sh             # lance une release : version, tag, push ; GitHub Actions fait le reste
src/
  main.rs                # la fenêtre, les réglages, la langue, les mises à jour ; démarre Telegram
  telegram/              # le seul code qui parle à finchgram-tdlib
    process.rs           #   exécute le programme : le JSON de TDLib sur l'entrée et la sortie standard
    api.rs               #   les types TDLib qu'utilise FinchGram (td_api.tl de la version figée)
    mod.rs               #   requêtes et réponses, redémarrage, connexion
  platform/              # ce qui change d'un système d'exploitation à l'autre
  update.rs              # la mise à jour automatique : GitHub Releases, signature, remplacement, relance
  settings.rs            # les préférences de l'utilisateur (settings.toml)
  i18n.rs                # langue de l'interface : le choix enregistré, sinon celle du système, sinon l'anglais
  bin/                   # finchgram-release-sign.rs, l'outil de signature Ed25519 des releases
ui/
  app.slint              # la fenêtre principale
  state.slint            # les globals partagés par Rust et les pages
  theme.slint            # couleurs, clair et sombre
  logo/                  # le logo de FinchGram (svg, png) et ses règles
vendor/tdlib/            # finchgram-tdlib
  build.sh               #   le compile à partir de sources figées : commit de TDLib, version et SHA-256 d'OpenSSL en tête
  host/                  #   notre petit programme hôte (main.cpp) et son CMakeLists.txt
  bin/                   #   hors de git : le programme qu'utilise l'application (scripts/fetch-tdlib.sh ou build.sh install)
  work/, dist/           #   hors de git : les fichiers intermédiaires et le paquet d'une compilation locale
```

## Traductions

Chaque texte de l'interface s'écrit `@tr("English text")`. Une chaîne a une seule traduction partout où
elle apparaît (build.rs désactive le contexte par défaut de Slint) ; quand le même texte anglais demande
d'autres mots ailleurs, donnez-lui un contexte : `@tr("menu" => "Open")`. Extrayez les textes avec
l'outil officiel, puis fusionnez-les dans chaque langue :

```sh
cargo install slint-tr-extractor                                          # une fois
find ui -name '*.slint' | sort | xargs slint-tr-extractor --no-default-translation-context -o lang/finchgram.pot
for po in lang/*/LC_MESSAGES/finchgram.po; do msgmerge --update "$po" lang/finchgram.pot; done   # nécessite brew install gettext
```

Remplissez ensuite les entrées `msgstr` et relancez `cargo build`. L'anglais est la langue source ; le
chinois simplifié est fourni pour l'instant.

## Versions publiées

Les releases ne sont compilées que par GitHub Actions, à partir d'un commit tagué et sur une machine
propre (`.github/workflows/release.yml`), puis publiées dans ce dépôt : l'application zippée,
`SHA256SUMS` et sa signature Ed25519. Les copies installées se mettent à jour d'elles-mêmes depuis
là et n'installent rien qu'elles ne puissent vérifier. Un mainteneur lance une release en une
commande :

```sh
scripts/release.sh patch    # 0.1.0 -> 0.1.1 ; aussi minor, major ou une version précise
```

La signature est ad hoc pour l'instant : au premier lancement d'une copie téléchargée, macOS demande
une fois (Réglages Système → Confidentialité et sécurité). Les mises à jour installées par
l'application elle-même démarrent sans cette étape.

## Licence

GPL-3.0 ([LICENSE](../LICENSE)). finchgram-tdlib contient TDLib (Boost Software License 1.0) et
OpenSSL (Apache License 2.0) ; leurs textes de licence l'accompagnent.
