# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · **Español** · [Português](pt.md) · [Deutsch](de.md) · [Français](fr.md) · [Русский](ru.md) · [日本語](ja.md) · [한국어](ko.md) · [العربية](ar.md)

Un cliente de escritorio de Telegram de código abierto, con un centro multimedia, escrito en Rust +
[Slint](https://slint.dev). Primero para macOS (Apple silicon); Windows y Linux, más adelante.

FinchGram usa la API de Telegram y forma parte del ecosistema de Telegram. Es un cliente no oficial,
no desarrollado por Telegram.

Estado: temprano. La carcasa arranca TDLib y sigue el avance del inicio de sesión; las páginas vienen
después, a partir del diseño.

La app es una carcasa (*shell*). De Telegram en sí se encarga TDLib, la biblioteca oficial de Telegram,
que se ejecuta como un programa aparte junto al ejecutable: `finchgram-tdlib`, compilado por este
repositorio a partir de fuentes con versiones fijadas (como ffmpeg en Coova Studio). La carcasa solo
habla con él a través de `src/telegram/`, en el propio JSON de TDLib. Consulta
[docs/architecture.md](../docs/architecture.md) y [docs/conventions.md](../docs/conventions.md) (en inglés).

Todo está aquí, en público: el código fuente, las compilaciones de las dependencias (`vendor/`) y las
versiones publicadas.

## Desarrollo

Por ahora, el desarrollo requiere macOS en Apple silicon: finchgram-tdlib solo se compila para esa
plataforma. Linux y Windows llegarán más adelante.

finchgram-tdlib en sí no está en git: `vendor/tdlib/` solo contiene el script que lo compila. Después de
clonar, obtén el programa una vez:

```sh
scripts/fetch-tdlib.sh    # descarga la release fijada de finchgram-tdlib en vendor/tdlib/bin/ y verifica el SHA-256
FINCHGRAM_API_ID=… FINCHGRAM_API_HASH=… cargo run
```

- Hasta que se publique la primera release `tdlib-*`, `scripts/fetch-tdlib.sh` lo avisa; en ese caso,
  compila finchgram-tdlib aquí (unos minutos; hace falta Xcode o las Command Line Tools, y
  `brew install cmake ninja gperf`, solo herramientas de compilación):

  ```sh
  vendor/tdlib/build.sh && vendor/tdlib/build.sh install
  ```

- `FINCHGRAM_API_ID` y `FINCHGRAM_API_HASH`: los tuyos, de
  [my.telegram.org](https://my.telegram.org) → API development tools. Se leen al compilar y nunca deben
  estar en el repositorio. Sin ellos, la app arranca e indica que no tiene API ID.
- `FINCHGRAM_TEST_DC=1 cargo run` usa los servidores de prueba de Telegram, que tienen sus propias
  cuentas; la app guarda una base de datos aparte para ellos.

`build.rs` copia `vendor/tdlib/bin/finchgram-tdlib` junto al ejecutable compilado, de modo que
`cargo run` usa exactamente el mismo programa que la app empaquetada; sin él, la compilación falla con
un mensaje que lo explica. Rust 1.92 o posterior.

La base de datos de TDLib y los archivos descargados están en
`~/Library/Application Support/FinchGram/tdlib/`; los ajustes, en
`~/Library/Application Support/FinchGram/settings.toml`.

## Estructura

```
.github/workflows/
  release.yml            # compila la app en cada push a main; publica las etiquetas v* como releases
  tdlib.yml              # compila finchgram-tdlib en una máquina limpia; publica las etiquetas tdlib-* como releases
Cargo.toml
build.rs                 # compila ui/app.slint, empaqueta lang/, copia vendor/tdlib/bin/ junto al ejecutable
docs/                    # architecture.md, conventions.md (+ zh-Hans)
lang/                    # traducciones: lang/<código>/LC_MESSAGES/finchgram.po, compiladas en el binario
readme/                  # este README en otros idiomas
release-signing.pub      # las claves públicas que pueden firmar las releases; compiladas en la app
scripts/
  bundle.sh              # construye dist/FinchGram.app (lo que ejecuta el flujo de release)
  fetch-tdlib.sh         # la release fijada de finchgram-tdlib en vendor/tdlib/bin/
  release.sh             # inicia una release: versión, etiqueta, push; GitHub Actions hace el resto
src/
  main.rs                # la ventana, los ajustes, el idioma, las actualizaciones; arranca Telegram
  telegram/              # el único código que habla con finchgram-tdlib
    process.rs           #   ejecuta el programa: el JSON de TDLib por la entrada y la salida estándar
    api.rs               #   los tipos de TDLib que usa FinchGram (td_api.tl de la versión fijada)
    mod.rs               #   peticiones y respuestas, reinicio, inicio de sesión
  platform/              # lo que cambia de un sistema operativo a otro
  update.rs              # la autoactualización: GitHub Releases, firma, sustitución, reinicio
  settings.rs            # las preferencias del usuario (settings.toml)
  i18n.rs                # idioma de la interfaz: el elegido; si no, el del sistema; si no, inglés
  bin/                   # finchgram-release-sign.rs, la herramienta de firma Ed25519 de las releases
ui/
  app.slint              # la ventana principal
  state.slint            # los globals que comparten Rust y las páginas
  theme.slint            # colores, claro y oscuro
  logo/                  # el logotipo de FinchGram (svg, png) y sus reglas
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
origen; por ahora se incluye el chino simplificado.

## Versiones

Las releases solo las compila GitHub Actions, a partir de un commit etiquetado y en una máquina limpia
(`.github/workflows/release.yml`), y se publican en este repositorio: la app comprimida,
`SHA256SUMS` y su firma Ed25519. Las copias instaladas se actualizan solas desde ahí y no instalan
nada que no puedan verificar. Un mantenedor inicia una release con un solo comando:

```sh
scripts/release.sh patch    # 0.1.0 -> 0.1.1; también minor, major o una versión exacta
```

Por ahora la firma es ad hoc: la primera vez que se abre una copia descargada, macOS pide permiso una
vez (Ajustes del Sistema → Privacidad y seguridad). Las actualizaciones que instala la propia app
arrancan sin ese paso.

## Licencia

GPL-3.0 ([LICENSE](../LICENSE)). finchgram-tdlib contiene TDLib (Boost Software License 1.0) y
OpenSSL (Apache License 2.0); sus textos de licencia se distribuyen con él.
