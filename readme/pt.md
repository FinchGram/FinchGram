# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · [Español](es.md) · **Português** · [Deutsch](de.md) · [Français](fr.md) · [Русский](ru.md) · [日本語](ja.md) · [한국어](ko.md) · [العربية](ar.md)

Um cliente de desktop do Telegram de código aberto, com uma central de mídia, escrito em Rust +
[Slint](https://slint.dev). Primeiro para macOS (Apple silicon); Windows e Linux, depois.

O FinchGram usa a API do Telegram e faz parte do ecossistema do Telegram. É um cliente não oficial,
não feito pelo Telegram.

Status: fase inicial. O cadastro e o login, a lista de conversas e as conversas com mensagens de texto
funcionam, nos três temas do design: Workbench (o padrão), Broadsheet e Terminal, trocados em
Configurações › Aparência. Fotos e vídeos aparecem nas conversas e abrem num visualizador que
reproduz o vídeo com o mpv. A verificação em duas etapas é gerenciada em Configurações › Privacidade e
segurança. Em seguida vêm arquivos, várias contas, filtros por palavras-chave e mensagens agendadas.

O app é uma casca (*shell*). O Telegram em si fica a cargo do TDLib, a biblioteca oficial do Telegram,
que roda como um programa separado ao lado do executável: `finchgram-tdlib`, compilado por este
repositório a partir de fontes com versões fixadas (como o ffmpeg no Coova Studio). A casca só fala com
ele por meio de `src/telegram/`, no próprio JSON do TDLib. Veja
[docs/architecture.md](../docs/architecture.md) e [docs/conventions.md](../docs/conventions.md) (em inglês).

Está tudo aqui, em público: o código-fonte, as compilações das dependências (`vendor/`) e as versões
publicadas.

## Desenvolvimento

Por enquanto, o desenvolvimento requer macOS com Apple silicon: o finchgram-tdlib só é compilado para
essa plataforma. Linux e Windows virão depois.

Nem o finchgram-tdlib, nem a libmpv, nem as fontes da interface ficam no git: `vendor/tdlib/` e
`vendor/mpv/` guardam só os scripts que os compilam, e as fontes vêm do Google Fonts. Depois de clonar,
baixe-os uma vez:

```sh
scripts/fetch-tdlib.sh    # baixa a release fixada do finchgram-tdlib para vendor/tdlib/bin/ e verifica o SHA-256
scripts/fetch-mpv.sh      # baixa a release fixada da libmpv para vendor/mpv/bin/ e verifica o SHA-256
scripts/fetch-fonts.sh    # baixa as fontes fixadas da interface para vendor/fonts/ e verifica o SHA-256
FINCHGRAM_API_ID=… FINCHGRAM_API_HASH=… cargo run
```

- Para mudar o próprio finchgram-tdlib, compile-o aqui (alguns minutos; precisa do Xcode ou das
  Command Line Tools, e de `brew install cmake ninja gperf`, só ferramentas de compilação):

  ```sh
  vendor/tdlib/build.sh && vendor/tdlib/build.sh install
  ```

- `FINCHGRAM_API_ID` e `FINCHGRAM_API_HASH`: os seus, de
  [my.telegram.org](https://my.telegram.org) → API development tools. Eles são lidos na compilação e
  nunca devem ir para o repositório. Sem eles, o app abre e avisa que não tem API ID.
- `FINCHGRAM_TEST_DC=1 cargo run` usa os servidores de teste do Telegram, que têm contas próprias; o app
  mantém um banco de dados separado para eles.
- `cargo test screenshots -- --ignored` desenha cada página de cada tema, claro e escuro, com conversas
  inventadas, em `target/screenshots/`: um jeito de ver a interface sem uma conta.

`build.rs` copia `vendor/tdlib/bin/finchgram-tdlib` para junto do executável compilado, então
`cargo run` usa exatamente o mesmo programa que o app empacotado; ele liga a libmpv de
`vendor/mpv/bin/` e também a copia para lá, e as fontes de `vendor/fonts/` são compiladas dentro do
executável. Sem qualquer um deles, a compilação falha com uma mensagem explicando isso. Rust 1.92 ou mais
recente.

O banco de dados do TDLib e os arquivos baixados ficam em
`~/Library/Application Support/FinchGram/tdlib/`; as configurações, em
`~/Library/Application Support/FinchGram/settings.toml`.

## Estrutura

```
.github/workflows/
  mpv.yml                # compila a libmpv numa máquina limpa; publica as tags mpv-* como releases
  release.yml            # compila o app a cada push na main; publica as tags v* como releases
  tdlib.yml              # compila o finchgram-tdlib numa máquina limpa; publica as tags tdlib-* como releases
Cargo.toml
build.rs                 # compila ui/app.slint, empacota lang/, copia vendor/tdlib/bin/ para junto do executável
docs/                    # architecture.md, conventions.md (+ zh-Hans)
lang/                    # traduções: lang/<código>/LC_MESSAGES/finchgram.po, compiladas no binário
readme/                  # este README em outros idiomas
release-signing.pub      # as chaves públicas que podem assinar as releases; compiladas no app
scripts/
  bundle.sh              # monta dist/FinchGram.app (o que o fluxo de release executa)
  fetch-fonts.sh         # as fontes fixadas da interface para vendor/fonts/
  fetch-mpv.sh           # a release fixada da libmpv em vendor/mpv/bin/
  fetch-tdlib.sh         # a release fixada do finchgram-tdlib em vendor/tdlib/bin/
  release.sh             # inicia uma release: versão, tag, push; o GitHub Actions faz o resto
src/
  main.rs                # a janela, as configurações, o idioma e o tema, as atualizações; inicia o Telegram
  fonts.rs               # as fontes da interface, compiladas dentro do executável
  images.rs              # as imagens das mensagens, decodificadas fora da thread da interface
  telegram/              # o único código que fala com o finchgram-tdlib
    process.rs           #   executa o programa: o JSON do TDLib pela entrada e saída padrão
    api.rs               #   os tipos do TDLib que o FinchGram usa (td_api.tl da versão fixada)
    mod.rs               #   pedidos e respostas, reiniciar; as atualizações vão para o store
    store.rs             #   o que o TDLib disse sobre conversas, usuários e mensagens; os modelos das páginas
    login.rs             #   o login, o cadastro
    chats.rs             #   a lista de conversas
    conversation.rs      #   a conversa aberta: mensagens, escrever
    account.rs           #   o perfil, sair
    password.rs          #   a verificação em duas etapas nas Configurações
    files.rs             #   o download de arquivos
    viewer.rs            #   o visualizador: fotos, vídeos, salvar em Downloads
    rich_text.rs         #   texto formatado: negrito, itálico, links…
  platform/              # o que muda de um sistema operacional para outro
  player/                # o vídeo com a libmpv, desenhado na janela com OpenGL
  update.rs              # a atualização automática: GitHub Releases, assinatura, troca, reinício
  settings.rs            # as preferências do usuário (settings.toml)
  i18n.rs                # idioma da interface: o escolhido; senão, o do sistema; senão, inglês
  screenshots.rs         # cada página desenhada num PNG (cargo test screenshots -- --ignored)
  bin/                   # finchgram-release-sign.rs, a ferramenta de assinatura Ed25519 das releases
ui/
  app.slint              # a janela principal: os menus e qual página aparece
  state.slint            # o estado do app, compartilhado pelo Rust e pelas páginas
  telegram.slint         # o que o Telegram mostra: a conta, o login, conversas, mensagens
  look.slint             # cores, tipografia e formas do tema, para as páginas compartilhadas
  format.slint           # datas, quantidades e tipos de mensagem no idioma da interface
  widgets.slint          # pequenas peças compartilhadas; chat.slint: o que as janelas de conversa compartilham
  viewer.slint           # o visualizador sobre a janela inteira, no estilo de cada tema
  pages/                 # as páginas que os três temas compartilham: login, configurações, perfil
  workbench/             # a janela de conversa do tema Workbench (o padrão)
  broadsheet/            # a janela de conversa do tema Broadsheet
  terminal/              # a janela de conversa do tema Terminal
  icons/                 # ícones Phosphor (MIT), normais e bicolores; icons.slint os lista
  logo/                  # o logo do FinchGram (svg, png) e suas regras
vendor/fonts/            # fora do git: as fontes da interface (scripts/fetch-fonts.sh)
vendor/mpv/              # libmpv: mpv e FFmpeg, que reproduzem o vídeo
  build.sh               #   compila a partir de fontes fixadas: versões e SHA-256 no topo
  bin/                   #   fora do git: a biblioteca (scripts/fetch-mpv.sh ou build.sh install)
vendor/tdlib/            # finchgram-tdlib
  build.sh               #   compila a partir de fontes fixadas: commit do TDLib, versão do OpenSSL e SHA-256 no topo
  host/                  #   nosso pequeno programa hospedeiro (main.cpp) e seu CMakeLists.txt
  bin/                   #   fora do git: o programa que o app usa (scripts/fetch-tdlib.sh ou build.sh install)
  work/, dist/           #   fora do git: os arquivos intermediários e o pacote de uma compilação local
```

## Traduções

Todo texto da interface é escrito como `@tr("English text")`. Uma string tem uma única tradução onde quer
que apareça (build.rs desliga o contexto padrão do Slint); quando o mesmo texto em inglês precisa de
outras palavras em outro lugar, dê a ele um contexto: `@tr("menu" => "Open")`. Extraia os textos com a
ferramenta oficial e depois mescle em cada idioma:

```sh
cargo install slint-tr-extractor                                          # uma vez
find ui -name '*.slint' | sort | xargs slint-tr-extractor --no-default-translation-context -o lang/finchgram.pot
for po in lang/*/LC_MESSAGES/finchgram.po; do msgmerge --update "$po" lang/finchgram.pot; done   # requer brew install gettext
```

Depois preencha as entradas `msgstr` e rode `cargo build` de novo. O inglês é o idioma de origem; por
enquanto vem incluído o chinês simplificado.

## Versões

As releases são compiladas só pelo GitHub Actions, a partir de um commit com tag e numa máquina limpa
(`.github/workflows/release.yml`), e publicadas neste repositório: o app compactado, `SHA256SUMS` e
sua assinatura Ed25519. As cópias instaladas se atualizam sozinhas a partir daqui e não instalam
nada que não consigam verificar. Um mantenedor inicia uma release com um único comando:

```sh
scripts/release.sh patch    # 0.1.0 -> 0.1.1; também minor, major ou uma versão exata
```

Por enquanto a assinatura é ad hoc: na primeira vez que uma cópia baixada é aberta, o macOS pede
permissão uma vez (Ajustes do Sistema → Privacidade e Segurança). As atualizações instaladas pelo
próprio app abrem sem esse passo.

## Licença

GPL-3.0 ([LICENSE](../LICENSE)). O finchgram-tdlib contém o TDLib (Boost Software License 1.0) e o
OpenSSL (Apache License 2.0); as fontes da interface estão sob a SIL Open Font License 1.1 e os ícones
sob a licença MIT. Os textos das licenças acompanham o app.
