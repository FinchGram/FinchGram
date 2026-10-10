# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · **Español** · [Português](pt.md) · [Deutsch](de.md) · [Français](fr.md) · [Русский](ru.md) · [日本語](ja.md) · [한국어](ko.md) · [العربية](ar.md)

<p align="center"><img src="../ui/logo/png/FinchGram-icon-128.png" width="96" alt="FinchGram"></p>

**Un cliente de escritorio de Telegram con centro multimedia, en Rust y [Slint](https://slint.dev).**
Código abierto bajo la GPL-3.0. Hoy para macOS en Apple silicon; Windows y Linux, más adelante.

FinchGram habla con Telegram a través de [TDLib](https://core.telegram.org/tdlib), la biblioteca del
propio Telegram, y respeta los [términos de la API de Telegram](https://core.telegram.org/api/terms).
Es un cliente no oficial, no hecho por Telegram.

<p align="center"><img src="../docs/screenshots/workbench-light-chats.png" width="800" alt="La ventana de chat en el tema Workbench"></p>

## Qué hace

- **Chats.** Inicia sesión con tu número de teléfono, el código y tu contraseña de verificación en dos
  pasos, o escaneando un código QR; o crea una cuenta nueva. La lista de chats con las carpetas de
  Telegram, los chats fijados, los contadores de no leídos y las menciones; un buscador (⌘K) y una
  búsqueda dentro del chat (⌘F); una vista de tus canales y otra de tus bots.
- **Mensajes.** Texto con su formato (negrita, cursiva, código, enlaces), respuestas y reenvíos,
  fotos, vídeos, GIF, stickers, archivos y una tarjeta con la vista previa de un enlace. Con un clic
  derecho en un mensaje se abre su menú: responder, editar, copiar, copiar su enlace, reenviar,
  reportar, eliminar o seleccionar varios. Las fotos y los vídeos enviados para autodestruirse se
  muestran difuminados y se abren una sola vez, como en las apps de Telegram.
- **Multimedia.** Las fotos y los vídeos se abren en un visor sobre la ventana, con el resto de su
  álbum; el vídeo se reproduce con mpv, compilado desde el código fuente. Se guardan en Descargas
  donde el chat lo permite.
- **Envío.** Fotos, vídeos y archivos desde el clip, arrastrándolos a la ventana o pegándolos. Una
  tarjeta los muestra antes de enviarlos: con un pie, como foto o como archivo, juntos en un álbum de
  hasta diez, con temporizador de autodestrucción en un chat privado, sin sonido.
- **Capturas de pantalla.** Las tijeras del cuadro de escritura, o ⌘⇧A, congelan la pantalla: toma una
  ventana o arrastra una selección, dibuja rectángulos, elipses, flechas, trazos, texto y mosaico
  encima, y envíala, cópiala o guárdala. Como en WeChat, desde cualquier chat.
- **El menú de un chat.** Silenciar, fijar, marcar como leído, meterlo en una carpeta; bloquear o
  desbloquear a una persona, reportar, salir de un grupo o canal, eliminar un chat.
- **Notificaciones.** Los mensajes nuevos llegan como notificaciones de macOS y su número se ve en el
  icono del Dock; FinchGram sigue en el Dock al cerrar su ventana y puede vivir en la barra de menús.
- **Tres temas.** Workbench, Broadsheet y Terminal, cada uno claro y oscuro, cambiados sin reiniciar.
- **Ajustes.** Abrir al iniciar sesión, enviar con Intro, el atajo de captura, sonidos y vistas previas
  de las notificaciones, verificación en dos pasos, el idioma de la interfaz y las actualizaciones: la
  app se actualiza sola desde GitHub Releases y no instala nada que no pueda verificar.
- **Idiomas.** La interfaz y este README en once idiomas.

Todavía no: chats secretos, mensajes de voz y llamadas, varias cuentas, encuestas y mensajes
programados, filtros por palabras clave, Windows y Linux. Mira
[qué viene después](../docs/architecture.md#not-now).

## Descarga

Descarga `FinchGram-<versión>-macos-arm64.zip` de la [última versión](https://github.com/FinchGram/FinchGram/releases/latest),
descomprímelo y mueve FinchGram a Aplicaciones. Necesita macOS 12 o posterior en Apple silicon. La app
aún no está notarizada, así que macOS pregunta una vez al abrirla por primera vez: Ajustes del
Sistema → Privacidad y seguridad → «Abrir de todos modos». A partir de ahí se actualiza sola.

Cada versión viene con `SHA256SUMS` y su firma Ed25519. La app comprueba ambas antes de instalar una
actualización; tú también puedes, con la clave pública de [release-signing.pub](../release-signing.pub).

## Tres temas

| Workbench | Broadsheet | Terminal |
|---|---|---|
| ![Workbench](../docs/screenshots/workbench-light-chats.png) | ![Broadsheet](../docs/screenshots/broadsheet-light-chats.png) | ![Terminal](../docs/screenshots/terminal-light-chats.png) |

Workbench, el tema por defecto, es una ventana de trabajo con pestañas. Broadsheet se lee como un
periódico, con un color de acento por cuenta. Terminal es una pantalla monoespaciada con comandos.
Cada uno tiene un lado claro y otro oscuro, según el sistema o tu elección, en Ajustes → Apariencia.

<p align="center"><img src="../docs/screenshots/workbench-dark-chats.png" width="800" alt="Workbench, oscuro"></p>

## La herramienta de capturas

<p align="center"><img src="../docs/screenshots/workbench-light-shot-annotated.png" width="800" alt="La capa de captura, con anotaciones"></p>

Pulsa ⌘⇧A, o las tijeras junto al clip. La pantalla se congela, oscurecida, bajo una capa: arrastras
una selección, con una lupa y el tamaño en píxeles, o haces clic en una ventana para tomarla entera.
La barra de herramientas dibuja rectángulos, elipses, flechas, trazos, texto y mosaico, en tres
tamaños y seis colores, con deshacer. Done pone la imagen en la tarjeta de envío, donde puede llevar
un pie; ⌘C la copia; ⌘S la guarda en Descargas. En Ajustes → General → Capturas de pantalla están el
atajo y si la ventana de FinchGram se oculta mientras tanto.

<p align="center"><img src="../docs/screenshots/workbench-light-shot-send-card.png" width="800" alt="La tarjeta de envío con una captura"></p>

## Cómo está hecho

La app es una carcasa. De Telegram se ocupa TDLib, que corre como un programa aparte junto al
ejecutable, `finchgram-tdlib`, que este repositorio compila desde fuentes fijadas; la carcasa solo
habla con él a través de `src/telegram/`, en el JSON del propio TDLib. El vídeo se reproduce con
libmpv, compilado del mismo modo. No se toma nada de la máquina del usuario, y una compilación no
descarga nada salvo estas compilaciones fijadas y con suma de comprobación. Todo es público: el
código, las compilaciones de `vendor/` y las versiones. Mira [docs/architecture.md](../docs/architecture.md)
y [docs/conventions.md](../docs/conventions.md).

## Las reglas de Telegram

FinchGram hace lo que hacen las apps de Telegram y nada de lo que prohíben. Un mensaje se marca como
leído cuando lo ves, la otra parte te ve escribiendo y en línea como en cualquier app de Telegram,
los mensajes patrocinados de los canales se muestran, los archivos que se autodestruyen se abren una
vez y nada de Telegram va a ninguna IA. Nada sale de tu máquina salvo hacia Telegram, y hacia GitHub
para las actualizaciones.

## Desarrollo

Por ahora, el desarrollo requiere macOS en Apple silicon: finchgram-tdlib solo se compila para esa
plataforma. Linux y Windows llegarán más adelante.

Ni finchgram-tdlib, ni libmpv, ni las fuentes de la interfaz están en git: `vendor/tdlib/` y `vendor/mpv/`
solo contienen los scripts que los compilan, y las fuentes vienen de Google Fonts. Después de clonar,
obtenlos una vez:

```sh
scripts/fetch-tdlib.sh    # descarga la release fijada de finchgram-tdlib en vendor/tdlib/bin/ y verifica el SHA-256
scripts/fetch-mpv.sh      # descarga la release fijada de libmpv en vendor/mpv/bin/ y verifica el SHA-256
scripts/fetch-fonts.sh    # descarga las fuentes fijadas de la interfaz en vendor/fonts/ y verifica su SHA-256
FINCHGRAM_API_ID=… FINCHGRAM_API_HASH=… cargo run
```

- Para cambiar el propio finchgram-tdlib, compílalo aquí (unos minutos; hace falta Xcode o las
  Command Line Tools, y `brew install cmake ninja gperf`, solo herramientas de compilación):

  ```sh
  vendor/tdlib/build.sh && vendor/tdlib/build.sh install
  ```

- `FINCHGRAM_API_ID` y `FINCHGRAM_API_HASH`: los tuyos, de
  [my.telegram.org](https://my.telegram.org) → API development tools. Se leen al compilar y nunca deben
  estar en el repositorio. Sin ellos, la app arranca e indica que no tiene API ID.
- `FINCHGRAM_TEST_DC=1 cargo run` usa los servidores de prueba de Telegram, que tienen sus propias
  cuentas; la app guarda una base de datos aparte para ellos.
- `cargo test screenshots -- --ignored` dibuja cada página de cada tema, en claro y oscuro, con chats
  inventados, en `target/screenshots/`: una forma de ver la interfaz sin una cuenta.

`build.rs` copia `vendor/tdlib/bin/finchgram-tdlib` junto al ejecutable compilado, de modo que
`cargo run` usa exactamente el mismo programa que la app empaquetada; enlaza libmpv desde
`vendor/mpv/bin/` y también la copia allí, y las fuentes de `vendor/fonts/` se compilan dentro del
ejecutable. Si falta cualquiera de ellos, la compilación falla con un mensaje que lo explica. Rust 1.92 o
posterior.

La base de datos de TDLib y los archivos descargados están en
`~/Library/Application Support/FinchGram/tdlib/`; los ajustes, en
`~/Library/Application Support/FinchGram/settings.toml`.

## Estructura

```
.github/workflows/
  mpv.yml                # compila libmpv en una máquina limpia; publica las etiquetas mpv-* como releases
  release.yml            # compila la app en cada push a main; publica las etiquetas v* como releases
  tdlib.yml              # compila finchgram-tdlib en una máquina limpia; publica las etiquetas tdlib-* como releases
Cargo.toml
build.rs                 # compila ui/app.slint, empaqueta lang/, copia vendor/tdlib/bin/ junto al ejecutable
docs/                    # architecture.md, conventions.md, drag-and-drop.md, media-files.md (+ zh-Hans)
  screenshots/           #   las imágenes que muestran los README, de la prueba de capturas (scripts/readme-pictures.sh)
lang/                    # traducciones: lang/<código>/LC_MESSAGES/finchgram.po, compiladas en el binario
readme/                  # este README en otros idiomas
release-signing.pub      # las claves públicas que pueden firmar las releases; compiladas en la app
scripts/
  bundle.sh              # construye dist/FinchGram.app (lo que ejecuta el flujo de release)
  fetch-fonts.sh         # las fuentes fijadas de la interfaz en vendor/fonts/
  fetch-mpv.sh           # la release fijada de libmpv en vendor/mpv/bin/
  fetch-tdlib.sh         # la release fijada de finchgram-tdlib en vendor/tdlib/bin/
  readme-pictures.sh     # copia las imágenes de los README de target/screenshots/ a docs/screenshots/
  release.sh             # inicia una release: versión, etiqueta, push; GitHub Actions hace el resto
src/
  main.rs                # la ventana, los ajustes, el idioma y el tema, las actualizaciones; arranca Telegram
  fonts.rs               # las fuentes de la interfaz, compiladas dentro del ejecutable
  images.rs              # las imágenes de los mensajes, decodificadas fuera del hilo de la interfaz
  telegram/              # el único código que habla con finchgram-tdlib
    process.rs           #   ejecuta el programa: el JSON de TDLib por la entrada y la salida estándar
    api.rs               #   los tipos de TDLib que usa FinchGram (td_api.tl de la versión fijada)
    mod.rs               #   peticiones y respuestas, volver a arrancar; las actualizaciones van al store
    store.rs             #   lo que TDLib dijo de chats, usuarios y mensajes; los modelos de las páginas
    login.rs             #   el inicio de sesión, el registro
    chats.rs             #   la lista de chats
    conversation.rs      #   el chat abierto: mensajes, escribir
    actions.rs           #   lo que se puede hacer con un mensaje: su menú, responder, reenviar…
    account.rs           #   el perfil, cerrar sesión
    password.rs          #   la verificación en dos pasos en Ajustes
    files.rs             #   la descarga de archivos
    avatars.rs           #   las fotos de chats y personas, en lugar de sus iniciales
    viewer.rs            #   el visor: fotos, vídeos, guardar en Descargas
    rich_text.rs         #   texto con formato: negrita, cursiva, enlaces…
    notifications.rs     #   notificaciones de mensajes nuevos, el número de no leídos en el Dock
    online.rs            #   la cuenta está en línea mientras la ventana está delante y en uso
  platform/              # lo que cambia de un sistema operativo a otro
  player/                # el vídeo con libmpv, dibujado en la ventana con OpenGL
  screenshot/            # la herramienta de captura: la captura, la capa, las anotaciones, la imagen
  update.rs              # la autoactualización: GitHub Releases, firma, sustitución, reinicio
  settings.rs            # las preferencias del usuario (settings.toml)
  i18n.rs                # idioma de la interfaz: el elegido; si no, el del sistema; si no, inglés
  screenshots.rs         # cada página dibujada en un PNG (cargo test screenshots -- --ignored)
  bin/                   # finchgram-release-sign.rs, la herramienta de firma Ed25519 de las releases
ui/
  app.slint              # la ventana principal: los menús y qué página se muestra
  state.slint            # el estado de la app, compartido por Rust y las páginas
  telegram.slint         # lo que muestra Telegram: la cuenta, el inicio de sesión, chats, mensajes
  look.slint             # colores, tipografía y formas del tema, para las páginas compartidas
  format.slint           # fechas, cantidades y tipos de mensaje en el idioma de la interfaz
  widgets.slint          # piezas pequeñas compartidas; chat.slint: lo que comparten las ventanas de chat
  viewer.slint           # el visor sobre toda la ventana, al estilo de cada tema
  pages/                 # las páginas que comparten los tres temas: inicio de sesión, ajustes, perfil
  workbench/             # la ventana de chat del tema Workbench (el predeterminado)
  broadsheet/            # la ventana de chat del tema Broadsheet
  terminal/              # la ventana de chat del tema Terminal
  icons/                 # iconos Phosphor (MIT), normales y bicolores; icons.slint los enumera
  logo/                  # el logotipo de FinchGram (svg, png) y sus reglas
vendor/fonts/            # no está en git: las fuentes de la interfaz (scripts/fetch-fonts.sh)
vendor/mpv/              # libmpv: mpv y FFmpeg, que reproducen el vídeo
  build.sh               #   la compila a partir de fuentes fijadas: versiones y SHA-256 al principio
  bin/                   #   no está en git: la biblioteca (scripts/fetch-mpv.sh o build.sh install)
vendor/tdlib/            # finchgram-tdlib
  build.sh               #   lo compila a partir de fuentes fijadas: commit de TDLib, versión de OpenSSL y SHA-256 al principio
  host/                  #   nuestro pequeño programa anfitrión (main.cpp) y su CMakeLists.txt
  bin/                   #   no está en git: el programa que usa la app (scripts/fetch-tdlib.sh o build.sh install)
  work/, dist/           #   no están en git: los archivos intermedios y el paquete de una compilación local
```

## Traducciones

Todo texto de la interfaz se escribe como `@tr("English text")`. Una cadena tiene una sola traducción
dondequiera que aparezca (build.rs desactiva el contexto por defecto de Slint); cuando el mismo texto en
inglés necesita otras palabras en otro sitio, dale un contexto: `@tr("menu" => "Open")`. Extrae los
textos con la herramienta oficial y luego fusiónalos en cada idioma:

```sh
cargo install slint-tr-extractor                                          # una vez
find ui -name '*.slint' | sort | xargs slint-tr-extractor --no-default-translation-context -o lang/finchgram.pot
for po in lang/*/LC_MESSAGES/finchgram.po; do msgmerge --update "$po" lang/finchgram.pot; done   # requiere brew install gettext
```

Después rellena las entradas `msgstr` y vuelve a ejecutar `cargo build`. El inglés es el idioma de
origen; los otros diez se incluyen.

## Versiones

Las releases solo las compila GitHub Actions, a partir de un commit etiquetado y en una máquina limpia
(`.github/workflows/release.yml`), y se publican en este repositorio: la app comprimida,
`SHA256SUMS` y su firma Ed25519. Las copias instaladas se actualizan solas desde ahí y no instalan
nada que no puedan verificar. Un mantenedor inicia una release con un solo comando:

```sh
scripts/release.sh patch    # 0.1.0 -> 0.1.1; también minor, major o una versión exacta
```

La app está firmada con el certificado propio del proyecto, no con un Apple Developer ID, así que la
primera vez que se abre una copia descargada, macOS pide permiso una vez (Ajustes del Sistema →
Privacidad y seguridad). Las actualizaciones que instala la propia app arrancan sin ese paso.

## Contribuir

Los issues y los pull requests son bienvenidos. Antes de cambiar cómo encajan las piezas, lee
[docs/architecture.md](../docs/architecture.md) y [docs/conventions.md](../docs/conventions.md): cada
dependencia la compila este repositorio desde fuentes fijadas, la interfaz sigue el diseño en los tres
temas y nada en la app va contra los términos de la API de Telegram. Mantén limpios `cargo test`,
`cargo clippy --all-targets` y `cargo test screenshots -- --ignored`, y mira las imágenes. Las
imágenes de los README también salen de esa prueba: `scripts/readme-pictures.sh` actualiza
`docs/screenshots/`.

## Licencia

GPL-3.0 ([LICENSE](../LICENSE)). finchgram-tdlib contiene TDLib (Boost Software License 1.0) y
OpenSSL (Apache License 2.0); las fuentes de la interfaz están bajo la SIL Open Font License 1.1 y los
iconos bajo la licencia MIT. Sus textos de licencia se distribuyen con la app.
