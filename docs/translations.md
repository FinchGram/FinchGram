# Translating the interface

[中文](translations.zh-Hans.md)

Every piece of UI text is written as `@tr("English text")`. A string has one translation wherever it
appears (crates/finchgram-ui/build.rs turns off Slint's default context); when the same English needs different words
somewhere else, give it a context: `@tr("menu" => "Open")`. Extract with the official tool, then merge
into each language:

```sh
cargo install slint-tr-extractor                                          # once
find ui -name '*.slint' | sort | xargs slint-tr-extractor --no-default-translation-context -o lang/finchgram.pot
for po in lang/*/LC_MESSAGES/finchgram.po; do msgmerge --update "$po" lang/finchgram.pot; done   # needs brew install gettext
```

Then fill in the `msgstr` entries and `cargo build` again. English is the source language; the
other ten ship with it.
