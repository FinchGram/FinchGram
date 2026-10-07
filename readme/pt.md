# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · [Español](es.md) · **Português** · [Deutsch](de.md) · [Français](fr.md) · [Русский](ru.md) · [日本語](ja.md) · [한국어](ko.md) · [العربية](ar.md)

<p align="center"><img src="../ui/logo/png/FinchGram-icon-128.png" width="96" alt="FinchGram"></p>

**Um cliente de desktop do Telegram com central de mídia, em Rust e [Slint](https://slint.dev).**
Código aberto sob a GPL-3.0. Hoje para macOS em Apple silicon; Windows e Linux depois.

O FinchGram fala com o Telegram pela [TDLib](https://core.telegram.org/tdlib), a biblioteca do
próprio Telegram, e respeita os [termos da API do Telegram](https://core.telegram.org/api/terms).
É um cliente não oficial, não feito pelo Telegram.

<p align="center"><img src="../docs/screenshots/workbench-light-chats.png" width="800" alt="A janela de chat no tema Workbench"></p>

## O que ele faz

- **Chats.** Entre com o número de telefone, o código e a senha da verificação em duas etapas, ou
  lendo um código QR; ou crie uma conta nova. A lista de chats com as pastas do Telegram, os chats
  fixados, as contagens de não lidos e as menções; uma busca (⌘K); uma visão dos seus canais e outra
  dos seus bots.
- **Mensagens.** Texto com formatação (negrito, itálico, código, links), respostas e encaminhamentos,
  fotos, vídeos, GIFs, figurinhas, arquivos e um cartão com a prévia de um link. Um clique direito
  numa mensagem abre o menu dela: responder, editar, copiar, copiar o link, encaminhar, denunciar,
  apagar ou selecionar várias. Fotos e vídeos enviados para se autodestruir aparecem desfocados e
  abrem uma única vez, como nos apps do Telegram.
- **Mídia.** Fotos e vídeos abrem num visualizador sobre a janela, com o resto do álbum; o vídeo toca
  com o mpv, compilado do código-fonte. Salvar em Downloads onde o chat permite.
- **Envio.** Fotos, vídeos e arquivos pelo clipe, arrastando-os para a janela ou colando-os. Um cartão
  os mostra antes de ir: com legenda, como foto ou como arquivo, juntos num álbum de até dez, com
  timer de autodestruição num chat privado, sem som.
- **Capturas de tela.** A tesoura na caixa de escrita, ou ⌘⇧A, congela a tela: pegue uma janela ou
  arraste uma seleção, desenhe retângulos, elipses, setas, traços, texto e mosaico por cima, e
  envie, copie ou salve. Como no WeChat, a partir de qualquer chat.
- **O menu de um chat.** Silenciar, fixar, marcar como lido, colocar numa pasta; bloquear ou
  desbloquear uma pessoa, denunciar, sair de um grupo ou canal, apagar um chat.
- **Notificações.** Mensagens novas chegam como notificações do macOS e o número delas fica no ícone
  do Dock; o FinchGram continua no Dock quando a janela fecha e pode morar na barra de menus.
- **Três temas.** Workbench, Broadsheet e Terminal, cada um claro e escuro, trocados sem reiniciar.
- **Configurações.** Abrir ao iniciar a sessão, enviar com Enter, o atalho da captura, sons e prévias
  das notificações, verificação em duas etapas, o idioma da interface e as atualizações: o app se
  atualiza sozinho a partir do GitHub Releases e não instala nada que não consiga verificar.
- **Idiomas.** A interface em inglês e chinês simplificado; este README em onze idiomas.

Ainda não: chats secretos, mensagens de voz e chamadas, várias contas, enquetes e mensagens
agendadas, filtros por palavra-chave, Windows e Linux. Veja
[o que vem a seguir](../docs/architecture.md#not-now).

## Download

Baixe `FinchGram-<versão>-macos-arm64.zip` da [última versão](https://github.com/FinchGram/FinchGram/releases/latest),
descompacte e mova o FinchGram para Aplicativos. Precisa de macOS 12 ou mais novo em Apple silicon.
O app ainda não é notarizado, então o macOS pergunta uma vez na primeira abertura: Ajustes do
Sistema → Privacidade e Segurança → "Abrir Mesmo Assim". Daí em diante ele se atualiza sozinho.

Cada versão vem com `SHA256SUMS` e sua assinatura Ed25519. O app confere as duas antes de instalar uma
atualização; você também pode, com a chave pública em [release-signing.pub](../release-signing.pub).

## Três temas

| Workbench | Broadsheet | Terminal |
|---|---|---|
| ![Workbench](../docs/screenshots/workbench-light-chats.png) | ![Broadsheet](../docs/screenshots/broadsheet-light-chats.png) | ![Terminal](../docs/screenshots/terminal-light-chats.png) |

O Workbench, o padrão, é uma janela de trabalho com abas. O Broadsheet se lê como um jornal, com uma
cor de destaque por conta. O Terminal é uma tela monoespaçada com comandos. Cada um tem um lado claro
e um escuro, seguindo o sistema ou a sua escolha, em Configurações → Aparência.

<p align="center"><img src="../docs/screenshots/workbench-dark-chats.png" width="800" alt="Workbench, escuro"></p>

## A ferramenta de captura

<p align="center"><img src="../docs/screenshots/workbench-light-shot-annotated.png" width="800" alt="A camada de captura, com anotações"></p>

Pressione ⌘⇧A, ou a tesoura ao lado do clipe. A tela congela sob uma camada: a janela sob o ponteiro é
oferecida, ou você arrasta uma seleção, com uma lupa e o tamanho em pixels. A barra de ferramentas
desenha retângulos, elipses, setas, traços, texto e mosaico, em três tamanhos e seis cores, com
desfazer. Done põe a imagem no cartão de envio, onde pode levar uma legenda; ⌘C a copia; ⌘S a salva em
Downloads. Configurações → Geral → Capturas de tela guarda o atalho e se a janela do FinchGram se
esconde enquanto isso.

<p align="center"><img src="../docs/screenshots/workbench-light-shot-send-card.png" width="800" alt="O cartão de envio com uma captura"></p>

## Como é feito

O app é uma casca. O Telegram em si fica com a TDLib, que roda como um programa separado ao lado do
executável, o `finchgram-tdlib`, que este repositório compila de fontes fixadas; a casca só fala com
ele por `src/telegram/`, no JSON da própria TDLib. O vídeo toca com a libmpv, compilada do mesmo jeito.
Nada é tirado da máquina do usuário, e uma compilação não baixa nada além dessas compilações fixadas e
com soma de verificação. Tudo é público: o código, as compilações em `vendor/` e as versões. Veja
[docs/architecture.md](../docs/architecture.md) e [docs/conventions.md](../docs/conventions.md).

## As regras do Telegram

O FinchGram faz o que os apps do Telegram fazem e nada do que eles proíbem. Uma mensagem é marcada
como lida quando você a vê, o outro lado vê você digitando e online como em qualquer app do Telegram,
as mensagens patrocinadas dos canais aparecem, a mídia que se autodestrói abre uma vez e nada do
Telegram vai para nenhuma IA. Nada sai da sua máquina a não ser para o Telegram, e para o GitHub para
as atualizações.

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
docs/                    # architecture.md, conventions.md, drag-and-drop.md (+ zh-Hans)
  screenshots/           #   as imagens que os READMEs mostram, do teste de capturas (scripts/readme-pictures.sh)
lang/                    # traduções: lang/<código>/LC_MESSAGES/finchgram.po, compiladas no binário
readme/                  # este README em outros idiomas
release-signing.pub      # as chaves públicas que podem assinar as releases; compiladas no app
scripts/
  bundle.sh              # monta dist/FinchGram.app (o que o fluxo de release executa)
  fetch-fonts.sh         # as fontes fixadas da interface para vendor/fonts/
  fetch-mpv.sh           # a release fixada da libmpv em vendor/mpv/bin/
  fetch-tdlib.sh         # a release fixada do finchgram-tdlib em vendor/tdlib/bin/
  readme-pictures.sh     # copia as imagens dos READMEs de target/screenshots/ para docs/screenshots/
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
    actions.rs           #   o que se pode fazer com uma mensagem: o menu, responder, encaminhar…
    account.rs           #   o perfil, sair
    password.rs          #   a verificação em duas etapas nas Configurações
    files.rs             #   o download de arquivos
    avatars.rs           #   as fotos de chats e pessoas, no lugar das iniciais
    viewer.rs            #   o visualizador: fotos, vídeos, salvar em Downloads
    rich_text.rs         #   texto formatado: negrito, itálico, links…
    notifications.rs     #   notificações de mensagens novas, o número de não lidas no Dock
    online.rs            #   a conta fica online enquanto a janela está na frente e em uso
  platform/              # o que muda de um sistema operacional para outro
  player/                # o vídeo com a libmpv, desenhado na janela com OpenGL
  screenshot/            # a ferramenta de captura: a captura, a camada, as anotações, a imagem
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

## Contribuir

Issues e pull requests são bem-vindos. Antes de mudar como as peças se encaixam, leia
[docs/architecture.md](../docs/architecture.md) e [docs/conventions.md](../docs/conventions.md): cada
dependência é compilada por este repositório de fontes fixadas, a interface segue o design nos três
temas e nada no app vai contra os termos da API do Telegram. Mantenha `cargo test`,
`cargo clippy --all-targets` e `cargo test screenshots -- --ignored` limpos, e olhe as imagens. As
imagens dos READMEs também vêm desse teste: `scripts/readme-pictures.sh` atualiza `docs/screenshots/`.

## Licença

GPL-3.0 ([LICENSE](../LICENSE)). O finchgram-tdlib contém o TDLib (Boost Software License 1.0) e o
OpenSSL (Apache License 2.0); as fontes da interface estão sob a SIL Open Font License 1.1 e os ícones
sob a licença MIT. Os textos das licenças acompanham o app.
