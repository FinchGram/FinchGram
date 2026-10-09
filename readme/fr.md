# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · [Español](es.md) · [Português](pt.md) · [Deutsch](de.md) · **Français** · [Русский](ru.md) · [日本語](ja.md) · [한국어](ko.md) · [العربية](ar.md)

<p align="center"><img src="../ui/logo/png/FinchGram-icon-128.png" width="96" alt="FinchGram"></p>

**Un client Telegram de bureau avec un centre multimédia, en Rust et [Slint](https://slint.dev).**
Libre, sous GPL-3.0. Aujourd'hui pour macOS sur Apple silicon ; Windows et Linux plus tard.

FinchGram parle à Telegram par [TDLib](https://core.telegram.org/tdlib), la bibliothèque de Telegram
lui-même, et respecte les [conditions de l'API Telegram](https://core.telegram.org/api/terms). C'est
un client non officiel, qui n'est pas fait par Telegram.

<p align="center"><img src="../docs/screenshots/workbench-light-chats.png" width="800" alt="La fenêtre de discussion dans le thème Workbench"></p>

## Ce qu'il fait

- **Discussions.** Connexion avec le numéro de téléphone, le code et le mot de passe de la validation
  en deux étapes, ou en scannant un code QR ; ou création d'un nouveau compte. La liste des
  discussions avec les dossiers de Telegram, les discussions épinglées, les compteurs de non-lus et
  les mentions ; une recherche (⌘K) ; une vue de vos chaînes et une de vos bots.
- **Messages.** Du texte avec sa mise en forme (gras, italique, code, liens), réponses et transferts,
  photos, vidéos, GIF, stickers, fichiers et une carte d'aperçu pour un lien. Un clic droit sur un
  message ouvre son menu : répondre, modifier, copier, copier le lien, transférer, signaler,
  supprimer ou en sélectionner plusieurs. Les photos et vidéos envoyées pour s'autodétruire
  s'affichent floutées et s'ouvrent une seule fois, comme dans les applications de Telegram.
- **Médias.** Les photos et vidéos s'ouvrent dans une visionneuse par-dessus la fenêtre, avec le reste
  de leur album ; la vidéo passe par mpv, compilé depuis les sources. Enregistrement dans
  Téléchargements là où la discussion le permet.
- **Envoi.** Photos, vidéos et fichiers depuis le trombone, en les glissant dans la fenêtre ou en les
  collant. Une carte les montre avant l'envoi : avec une légende, en photo ou en fichier, ensemble
  dans un album de dix au plus, avec un minuteur d'autodestruction dans une discussion privée, sans
  son.
- **Captures d'écran.** Les ciseaux du champ de saisie, ou ⌘⇧A, figent l'écran : prenez une fenêtre
  ou tracez une sélection, dessinez dessus rectangles, ellipses, flèches, traits, texte et mosaïque,
  puis envoyez, copiez ou enregistrez. Comme dans WeChat, depuis chaque discussion.
- **Le menu d'une discussion.** Mettre en sourdine, épingler, marquer comme lu, ranger dans un
  dossier ; bloquer ou débloquer une personne, signaler, quitter un groupe ou une chaîne, supprimer
  une discussion.
- **Notifications.** Les nouveaux messages arrivent en notifications macOS et leur nombre s'affiche
  sur l'icône du Dock ; FinchGram reste dans le Dock quand sa fenêtre se ferme et peut vivre dans la
  barre des menus.
- **Trois thèmes.** Workbench, Broadsheet et Terminal, chacun en clair et en sombre, changés sans
  redémarrer.
- **Réglages.** Ouvrir à la connexion, envoyer avec Entrée, le raccourci de capture, sons et aperçus
  des notifications, validation en deux étapes, langue de l'interface, et mises à jour : l'application
  se met à jour elle-même depuis GitHub Releases et n'installe rien qu'elle ne puisse vérifier.
- **Langues.** L'interface et ce README en onze langues.

Pas encore : discussions secrètes, messages vocaux et appels, plusieurs comptes, sondages et messages
programmés, filtres par mots-clés, Windows et Linux. Voir
[ce qui vient ensuite](../docs/architecture.md#not-now).

## Téléchargement

Prenez `FinchGram-<version>-macos-arm64.zip` dans la [dernière version](https://github.com/FinchGram/FinchGram/releases/latest),
décompressez-le et déplacez FinchGram dans Applications. Il faut macOS 12 ou plus récent sur Apple
silicon. L'application n'est pas encore notariée, alors macOS demande une fois au premier lancement :
Réglages Système → Confidentialité et sécurité → « Ouvrir quand même ». Ensuite elle se met à jour
elle-même.

Chaque version vient avec `SHA256SUMS` et sa signature Ed25519. L'application vérifie les deux avant
d'installer une mise à jour ; vous le pouvez aussi, avec la clé publique de
[release-signing.pub](../release-signing.pub).

## Trois thèmes

| Workbench | Broadsheet | Terminal |
|---|---|---|
| ![Workbench](../docs/screenshots/workbench-light-chats.png) | ![Broadsheet](../docs/screenshots/broadsheet-light-chats.png) | ![Terminal](../docs/screenshots/terminal-light-chats.png) |

Workbench, le thème par défaut, est une fenêtre de travail à onglets. Broadsheet se lit comme un
journal, avec une couleur d'accent par compte. Terminal est un écran en chasse fixe avec des
commandes. Chacun a un côté clair et un côté sombre, selon le système ou votre choix, dans Réglages →
Apparence.

<p align="center"><img src="../docs/screenshots/workbench-dark-chats.png" width="800" alt="Workbench, sombre"></p>

## L'outil de capture

<p align="center"><img src="../docs/screenshots/workbench-light-shot-annotated.png" width="800" alt="Le calque de capture, avec des annotations"></p>

Appuyez sur ⌘⇧A, ou sur les ciseaux à côté du trombone. L'écran se fige, assombri, sous un calque :
vous tracez une sélection, avec une loupe et la taille en pixels, ou cliquez une fenêtre pour la
prendre entière. La barre d'outils dessine rectangles, ellipses, flèches, traits, texte et mosaïque,
en trois tailles et six couleurs, avec annulation. Done met l'image dans la carte d'envoi, où une
légende peut l'accompagner ; ⌘C la copie ; ⌘S l'enregistre dans Téléchargements. Réglages → Général
→ Captures d'écran tient le raccourci et dit si la fenêtre de FinchGram se cache pendant ce temps.

<p align="center"><img src="../docs/screenshots/workbench-light-shot-send-card.png" width="800" alt="La carte d'envoi avec une capture"></p>

## Comment c'est fait

L'application est une coquille. Telegram lui-même est l'affaire de TDLib, qui tourne comme un
programme à part à côté de l'exécutable, `finchgram-tdlib`, que ce dépôt compile depuis des sources
figées ; la coquille ne lui parle que par `src/telegram/`, dans le JSON de TDLib. La vidéo passe par
libmpv, compilé de la même manière. Rien n'est pris sur la machine de l'utilisateur, et une
compilation ne télécharge rien d'autre que ces builds figés et vérifiés par somme de contrôle. Tout
est public : les sources, les builds de `vendor/` et les versions. Voir
[docs/architecture.md](../docs/architecture.md) et [docs/conventions.md](../docs/conventions.md).

## Les règles de Telegram

FinchGram fait ce que font les applications de Telegram, et rien de ce qu'elles interdisent. Un
message est marqué lu quand vous le voyez, l'autre côté vous voit écrire et en ligne comme dans toute
application Telegram, les messages sponsorisés des chaînes sont affichés, les médias qui
s'autodétruisent s'ouvrent une fois, et rien de Telegram ne va à une IA. Rien ne quitte votre machine
sauf vers Telegram, et vers GitHub pour les mises à jour.

## Développement

Pour l'instant, le développement nécessite macOS sur Apple silicon : finchgram-tdlib n'est compilé que
pour cette plateforme. Linux et Windows suivront.

Ni finchgram-tdlib, ni libmpv, ni les polices de l'interface ne sont dans git : `vendor/tdlib/` et
`vendor/mpv/` ne contiennent que les scripts qui les compilent, et les polices viennent de Google Fonts.
Après le clonage, récupérez-les une fois :

```sh
scripts/fetch-tdlib.sh    # télécharge la release figée de finchgram-tdlib dans vendor/tdlib/bin/ et vérifie le SHA-256
scripts/fetch-mpv.sh      # télécharge la release figée de libmpv dans vendor/mpv/bin/ et vérifie le SHA-256
scripts/fetch-fonts.sh    # télécharge les polices figées de l'interface dans vendor/fonts/ et vérifie leur SHA-256
FINCHGRAM_API_ID=… FINCHGRAM_API_HASH=… cargo run
```

- Pour modifier finchgram-tdlib lui-même, compilez-le ici (quelques minutes ; il faut Xcode ou les
  Command Line Tools, et `brew install cmake ninja gperf`, uniquement des outils de compilation) :

  ```sh
  vendor/tdlib/build.sh && vendor/tdlib/build.sh install
  ```

- `FINCHGRAM_API_ID` et `FINCHGRAM_API_HASH` : les vôtres, obtenus sur
  [my.telegram.org](https://my.telegram.org) → API development tools. Ils sont lus à la compilation et
  n'ont jamais leur place dans le dépôt. Sans eux, l'application démarre et indique qu'elle n'a pas
  d'API ID.
- `FINCHGRAM_TEST_DC=1 cargo run` utilise les serveurs de test de Telegram, qui ont leurs propres
  comptes ; l'application garde pour eux une base de données distincte.
- `cargo test screenshots -- --ignored` dessine chaque page de chaque thème, en clair et en sombre, avec
  des discussions inventées, dans `target/screenshots/` : de quoi voir l'interface sans compte.

`build.rs` copie `vendor/tdlib/bin/finchgram-tdlib` à côté de l'exécutable compilé : `cargo run` utilise
ainsi exactement le même programme que l'application empaquetée ; il lie libmpv depuis `vendor/mpv/bin/`
et la copie au même endroit, et les polices de `vendor/fonts/` sont compilées dans l'exécutable. S'il en
manque un, la compilation échoue avec un message qui l'explique. Rust 1.92 ou plus récent.

La base de données de TDLib et les fichiers téléchargés se trouvent dans
`~/Library/Application Support/FinchGram/tdlib/`, les réglages dans
`~/Library/Application Support/FinchGram/settings.toml`.

## Organisation

```
.github/workflows/
  mpv.yml                # compile libmpv sur une machine propre ; publie les tags mpv-* comme releases
  release.yml            # compile l'application à chaque push sur main ; publie les tags v* comme releases
  tdlib.yml              # compile finchgram-tdlib sur une machine propre ; publie les tags tdlib-* comme releases
Cargo.toml
build.rs                 # compile ui/app.slint, intègre lang/, copie vendor/tdlib/bin/ à côté de l'exécutable
docs/                    # architecture.md, conventions.md, drag-and-drop.md (+ zh-Hans)
  screenshots/           #   les images que montrent les README, issues du test de captures (scripts/readme-pictures.sh)
lang/                    # traductions : lang/<code>/LC_MESSAGES/finchgram.po, compilées dans le binaire
readme/                  # ce README dans d'autres langues
release-signing.pub      # les clés publiques autorisées à signer les releases ; compilées dans l'application
scripts/
  bundle.sh              # construit dist/FinchGram.app (ce qu'exécute le workflow de release)
  fetch-fonts.sh         # les polices figées de l'interface dans vendor/fonts/
  fetch-mpv.sh           # la release figée de libmpv dans vendor/mpv/bin/
  fetch-tdlib.sh         # la release figée de finchgram-tdlib dans vendor/tdlib/bin/
  readme-pictures.sh     # copie les images des README de target/screenshots/ vers docs/screenshots/
  release.sh             # lance une release : version, tag, push ; GitHub Actions fait le reste
src/
  main.rs                # la fenêtre, les réglages, la langue et le thème, les mises à jour ; démarre Telegram
  fonts.rs               # les polices de l'interface, compilées dans l'exécutable
  images.rs              # les images des messages, décodées hors du fil de l'interface
  telegram/              # le seul code qui parle à finchgram-tdlib
    process.rs           #   exécute le programme : le JSON de TDLib sur l'entrée et la sortie standard
    api.rs               #   les types TDLib qu'utilise FinchGram (td_api.tl de la version figée)
    mod.rs               #   requêtes et réponses, redémarrage ; les mises à jour vont au store
    store.rs             #   ce que TDLib a dit des discussions, utilisateurs et messages ; les modèles des pages
    login.rs             #   la connexion, l'inscription
    chats.rs             #   la liste des discussions
    conversation.rs      #   la discussion ouverte : messages, écriture
    actions.rs           #   ce qu'on peut faire d'un message : son menu, répondre, transférer…
    account.rs           #   le profil, la déconnexion
    password.rs          #   la validation en deux étapes dans les réglages
    files.rs             #   le téléchargement des fichiers
    avatars.rs           #   les photos des discussions et des personnes, à la place de leurs initiales
    viewer.rs            #   la visionneuse : photos, vidéos, enregistrer dans Téléchargements
    rich_text.rs         #   texte mis en forme : gras, italique, liens…
    notifications.rs     #   notifications des nouveaux messages, le nombre de non-lus dans le Dock
    online.rs            #   le compte est en ligne tant que la fenêtre est devant et utilisée
  platform/              # ce qui change d'un système d'exploitation à l'autre
  player/                # la vidéo avec libmpv, dessinée dans la fenêtre par OpenGL
  screenshot/            # l'outil de capture : la capture, le calque, les annotations, l'image
  update.rs              # la mise à jour automatique : GitHub Releases, signature, remplacement, relance
  settings.rs            # les préférences de l'utilisateur (settings.toml)
  i18n.rs                # langue de l'interface : le choix enregistré, sinon celle du système, sinon l'anglais
  screenshots.rs         # chaque page dessinée en PNG (cargo test screenshots -- --ignored)
  bin/                   # finchgram-release-sign.rs, l'outil de signature Ed25519 des releases
ui/
  app.slint              # la fenêtre principale : les menus, et quelle page s'affiche
  state.slint            # l'état de l'application, partagé par Rust et les pages
  telegram.slint         # ce que montre Telegram : le compte, la connexion, les discussions, les messages
  look.slint             # couleurs, typographie et formes du thème, pour les pages partagées
  format.slint           # dates, nombres et types de message dans la langue de l'interface
  widgets.slint          # petites pièces partagées ; chat.slint : ce que partagent les fenêtres de discussion
  viewer.slint           # la visionneuse sur toute la fenêtre, à la manière de chaque thème
  pages/                 # les pages que partagent les trois thèmes : connexion, réglages, profil
  workbench/             # la fenêtre de discussion du thème Workbench (par défaut)
  broadsheet/            # la fenêtre de discussion du thème Broadsheet
  terminal/              # la fenêtre de discussion du thème Terminal
  icons/                 # icônes Phosphor (MIT), normales et bicolores ; icons.slint les énumère
  logo/                  # le logo de FinchGram (svg, png) et ses règles
vendor/fonts/            # hors de git : les polices de l'interface (scripts/fetch-fonts.sh)
vendor/mpv/              # libmpv : mpv et FFmpeg, qui lisent les vidéos
  build.sh               #   la compile à partir de sources figées : versions et SHA-256 en tête
  bin/                   #   hors de git : la bibliothèque (scripts/fetch-mpv.sh ou build.sh install)
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

Remplissez ensuite les entrées `msgstr` et relancez `cargo build`. L'anglais est la langue source ; les
dix autres sont fournies.

## Versions publiées

Les releases ne sont compilées que par GitHub Actions, à partir d'un commit tagué et sur une machine
propre (`.github/workflows/release.yml`), puis publiées dans ce dépôt : l'application zippée,
`SHA256SUMS` et sa signature Ed25519. Les copies installées se mettent à jour d'elles-mêmes depuis
là et n'installent rien qu'elles ne puissent vérifier. Un mainteneur lance une release en une
commande :

```sh
scripts/release.sh patch    # 0.1.0 -> 0.1.1 ; aussi minor, major ou une version précise
```

L'application est signée avec le certificat du projet, pas avec un Apple Developer ID : au premier
lancement d'une copie téléchargée, macOS demande donc une fois (Réglages Système → Confidentialité et
sécurité). Les mises à jour installées par l'application elle-même démarrent sans cette étape.

## Contribuer

Les issues et les pull requests sont les bienvenus. Avant de changer la façon dont les pièces
s'assemblent, lisez [docs/architecture.md](../docs/architecture.md) et
[docs/conventions.md](../docs/conventions.md) : chaque dépendance est compilée par ce dépôt depuis des
sources figées, l'interface suit le design dans les trois thèmes, et rien dans l'application ne va
contre les conditions de l'API Telegram. Gardez `cargo test`, `cargo clippy --all-targets` et
`cargo test screenshots -- --ignored` propres, et regardez les images. Les images des README viennent
aussi de ce test : `scripts/readme-pictures.sh` rafraîchit `docs/screenshots/`.

## Licence

GPL-3.0 ([LICENSE](../LICENSE)). finchgram-tdlib contient TDLib (Boost Software License 1.0) et
OpenSSL (Apache License 2.0) ; les polices de l'interface sont sous SIL Open Font License 1.1, les icônes
sous licence MIT. Leurs textes de licence accompagnent l'application.
