# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · [Español](es.md) · [Português](pt.md) · [Deutsch](de.md) · [Français](fr.md) · **Русский** · [日本語](ja.md) · [한국어](ko.md) · [العربية](ar.md)

Десктопный клиент Telegram с открытым исходным кодом и медиацентром, написанный на Rust +
[Slint](https://slint.dev). Сначала для macOS (Apple silicon), позже для Windows и Linux.

FinchGram использует Telegram API и является частью экосистемы Telegram. Это неофициальный клиент,
его сделал не Telegram.

Состояние: ранняя стадия. Регистрация и вход в аккаунт, список чатов и чаты с текстовыми сообщениями
работают, в трёх темах макета: Workbench (по умолчанию), Broadsheet и Terminal, которые переключаются в
Настройках › Оформление. Двухэтапная проверка настраивается в Настройках › Конфиденциальность. Дальше —
фото и файлы, несколько аккаунтов, фильтры по словам и отложенные сообщения.

Приложение — это оболочка (shell). Сам Telegram берёт на себя TDLib, официальная библиотека Telegram:
она работает отдельной программой рядом с исполняемым файлом. Эта программа — `finchgram-tdlib`, её
собирает этот репозиторий из исходников с зафиксированными версиями (как ffmpeg у Coova Studio).
Оболочка общается с ней только через `src/telegram/`, на собственном JSON TDLib. См.
[docs/architecture.md](../docs/architecture.md) и [docs/conventions.md](../docs/conventions.md) (на английском).

Всё здесь и всё открыто: исходный код, сборки зависимостей (`vendor/`) и релизы.

## Разработка

Пока для разработки нужен macOS на Apple silicon: finchgram-tdlib собирается только для него. Linux и
Windows появятся позже.

Ни finchgram-tdlib, ни шрифты интерфейса в git не хранятся: в `vendor/tdlib/` лежит только скрипт,
который собирает программу, а шрифты берутся из Google Fonts. После клонирования один раз получите их:

```sh
scripts/fetch-tdlib.sh    # скачивает зафиксированный релиз finchgram-tdlib в vendor/tdlib/bin/ и проверяет SHA-256
scripts/fetch-fonts.sh    # скачивает зафиксированные шрифты интерфейса в vendor/fonts/ и проверяет их SHA-256
FINCHGRAM_API_ID=… FINCHGRAM_API_HASH=… cargo run
```

- Чтобы изменить сам finchgram-tdlib, соберите его здесь (несколько минут; нужны Xcode или Command
  Line Tools и `brew install cmake ninja gperf` — только инструменты сборки):

  ```sh
  vendor/tdlib/build.sh && vendor/tdlib/build.sh install
  ```

- `FINCHGRAM_API_ID` и `FINCHGRAM_API_HASH`: ваши собственные, с
  [my.telegram.org](https://my.telegram.org) → API development tools. Они читаются при компиляции и
  никогда не должны попадать в репозиторий. Без них приложение запускается и сообщает, что у него нет
  API ID.
- `FINCHGRAM_TEST_DC=1 cargo run` использует тестовые серверы Telegram, у которых свои аккаунты;
  приложение хранит для них отдельную базу данных.
- `cargo test screenshots -- --ignored` рисует каждую страницу каждой темы, светлую и тёмную, с
  выдуманными чатами, в `target/screenshots/`: так можно посмотреть интерфейс без аккаунта.

`build.rs` копирует `vendor/tdlib/bin/finchgram-tdlib` рядом со скомпилированным исполняемым файлом, так
что `cargo run` использует ровно ту же программу, что и упакованное приложение, а шрифты из
`vendor/fonts/` компилируются в исполняемый файл; без любого из них сборка завершается ошибкой с
объяснением. Rust 1.92 или новее.

База данных TDLib и скачанные файлы находятся в `~/Library/Application Support/FinchGram/tdlib/`,
настройки — в `~/Library/Application Support/FinchGram/settings.toml`.

## Структура

```
.github/workflows/
  release.yml            # собирает приложение при каждом push в main; публикует теги v* как релизы
  tdlib.yml              # собирает finchgram-tdlib на чистой машине; публикует теги tdlib-* как релизы
Cargo.toml
build.rs                 # компилирует ui/app.slint, встраивает lang/, копирует vendor/tdlib/bin/ рядом с исполняемым файлом
docs/                    # architecture.md, conventions.md (+ zh-Hans)
lang/                    # переводы: lang/<код>/LC_MESSAGES/finchgram.po, вкомпилированы в бинарный файл
readme/                  # этот README на других языках
release-signing.pub      # открытые ключи, которым разрешено подписывать релизы; вкомпилированы в приложение
scripts/
  bundle.sh              # собирает dist/FinchGram.app (его и запускает процесс релиза)
  fetch-fonts.sh         # зафиксированные шрифты интерфейса в vendor/fonts/
  fetch-tdlib.sh         # зафиксированный релиз finchgram-tdlib в vendor/tdlib/bin/
  release.sh             # запускает релиз: версия, тег, push; остальное делает GitHub Actions
src/
  main.rs                # окно, настройки, язык и тема, обновления; запускает Telegram
  fonts.rs               # шрифты интерфейса, скомпилированные в исполняемый файл
  telegram/              # единственный код, который общается с finchgram-tdlib
    process.rs           #   запускает программу: JSON TDLib через стандартный ввод и вывод
    api.rs               #   типы TDLib, которые использует FinchGram (td_api.tl зафиксированной версии)
    mod.rs               #   запросы и ответы, перезапуск; обновления идут в store
    store.rs             #   что TDLib сообщил о чатах, пользователях и сообщениях; модели страниц
    login.rs             #   вход в аккаунт, регистрация
    chats.rs             #   список чатов
    conversation.rs      #   открытый чат: сообщения, отправка
    account.rs           #   профиль, выход
    password.rs          #   двухэтапная проверка в настройках
  platform/              # то, что отличается от одной операционной системы к другой
  update.rs              # самообновление: GitHub Releases, подпись, замена, перезапуск
  settings.rs            # настройки пользователя (settings.toml)
  i18n.rs                # язык интерфейса: сохранённый выбор, иначе системный, иначе английский
  screenshots.rs         # каждая страница в PNG (cargo test screenshots -- --ignored)
  bin/                   # finchgram-release-sign.rs, инструмент подписи Ed25519 для релизов
ui/
  app.slint              # главное окно: меню и то, какая страница показана
  state.slint            # состояние приложения, общее для Rust и страниц
  telegram.slint         # что показывает Telegram: аккаунт, вход, чаты, сообщения
  look.slint             # цвета, шрифты и формы темы для общих страниц
  format.slint           # даты, числа и виды сообщений на языке интерфейса
  widgets.slint          # мелкие общие детали; chat.slint — общее для окон чатов
  pages/                 # страницы, общие для трёх тем: вход, настройки, профиль
  workbench/             # окно чатов темы Workbench (по умолчанию)
  broadsheet/            # окно чатов темы Broadsheet
  terminal/              # окно чатов темы Terminal
  icons/                 # значки Phosphor (MIT), обычные и двухцветные; их список в icons.slint
  logo/                  # логотип FinchGram (svg, png) и правила его использования
vendor/fonts/            # не в git: шрифты интерфейса (scripts/fetch-fonts.sh)
vendor/tdlib/            # finchgram-tdlib
  build.sh               #   собирает его из зафиксированных исходников: коммит TDLib, версия и SHA-256 OpenSSL в начале
  host/                  #   наша небольшая программа-хост (main.cpp) и её CMakeLists.txt
  bin/                   #   не в git: программа, которую использует приложение (scripts/fetch-tdlib.sh или build.sh install)
  work/, dist/           #   не в git: промежуточные файлы и пакет локальной сборки
```

## Переводы

Каждый текст интерфейса пишется как `@tr("English text")`. У строки один перевод, где бы она ни
встречалась (build.rs отключает контекст Slint по умолчанию); если тот же английский текст в другом
месте требует других слов, дайте ему контекст: `@tr("menu" => "Open")`. Извлеките строки официальным
инструментом, затем объедините их с каждым языком:

```sh
cargo install slint-tr-extractor                                          # один раз
find ui -name '*.slint' | sort | xargs slint-tr-extractor --no-default-translation-context -o lang/finchgram.pot
for po in lang/*/LC_MESSAGES/finchgram.po; do msgmerge --update "$po" lang/finchgram.pot; done   # нужен brew install gettext
```

Затем заполните записи `msgstr` и снова запустите `cargo build`. Исходный язык — английский; пока в
комплекте упрощённый китайский.

## Релизы

Релизы собирает только GitHub Actions — из коммита с тегом на чистой машине
(`.github/workflows/release.yml`) — и публикует их в этом репозитории: упакованное приложение,
`SHA256SUMS` и его подпись Ed25519. Установленные копии обновляются отсюда сами и не устанавливают
ничего, что не могут проверить. Мейнтейнер запускает релиз одной командой:

```sh
scripts/release.sh patch    # 0.1.0 -> 0.1.1; также minor, major или точная версия
```

Пока подпись ad hoc: при первом запуске скачанной копии macOS один раз спросит разрешения
(Системные настройки → Конфиденциальность и безопасность). Обновления, которые устанавливает само
приложение, запускаются без этого шага.

## Лицензия

GPL-3.0 ([LICENSE](../LICENSE)). finchgram-tdlib содержит TDLib (Boost Software License 1.0) и
OpenSSL (Apache License 2.0); шрифты интерфейса распространяются по SIL Open Font License 1.1, значки —
по лицензии MIT. Тексты их лицензий поставляются вместе с приложением.
