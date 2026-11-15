# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · [Español](es.md) · **Português** · [Deutsch](de.md) · [Français](fr.md) · [Русский](ru.md) · [日本語](ja.md) · [한국어](ko.md) · [العربية](ar.md)

<p align="center"><img src="../ui/logo/png/FinchGram-icon-128.png" width="96" alt="FinchGram"></p>

**Um cliente de desktop do Telegram com superpoderes, em Rust e [Slint](https://slint.dev).**
Código aberto sob a GPL-3.0. Hoje para macOS em Apple silicon; Windows e Linux depois.

O FinchGram fala com o Telegram pela [TDLib](https://core.telegram.org/tdlib), a biblioteca do
próprio Telegram, e respeita os [termos da API do Telegram](https://core.telegram.org/api/terms).
É um cliente não oficial, não feito pelo Telegram.

<p align="center"><img src="../docs/screenshots/workbench-light-chats.png" width="800" alt="A janela de chat no tema Workbench"></p>

## O que ele faz

- **Telegram, como nos apps dele.** Chats com pastas, busca e até dez contas; mensagens com respostas, reações, bots, tópicos e rascunhos; fotos, vídeos, arquivos e mensagens de voz; a gestão dos seus grupos e canais.
- **Uma ferramenta de captura.** ⌘⇧A congela a tela: pegue uma janela ou uma seleção, desenhe por cima e envie. De qualquer chat, como no WeChat.
- **Downloads.** Um cliente de download para os seus próprios servidores por WebDAV, FTP, FTPS ou SFTP: um NAS, uma seedbox, o PikPak. [Mais](../docs/downloads.md)
- **Música.** Um player para os seus próprios servidores que falam a API do Subsonic: Navidrome e afins. [Mais](../docs/music.md)
- **Transmissões ao vivo.** Owncast, PeerTube, Icecast e as do próprio Telegram: descubra, siga e assista no app. [Mais](../docs/live-streams.md)
- **Ditado por voz.** Fale e as palavras vão para a caixa de texto, pelo provedor de IA que você escolher e com a sua própria chave. [Mais](../docs/voice-input.md)
- **Assistentes de IA.** O Claude e outros apps com MCP leem os seus chats e agem neles pelo FinchGram, quando você permite. [Mais](../docs/mcp.md)
- **Três temas.** Workbench, Broadsheet e Terminal, cada um claro e escuro. O Terminal tem uma linha de comando de verdade. [Como usar](../docs/terminal.md)
- **Onze idiomas**, e atualizações que o app instala sozinho, depois de verificá-las.

[Tudo o que ele faz, com imagens](../docs/features.md). Ainda não: chats secretos e chamadas, enquetes e mensagens agendadas, filtros por palavra-chave, Windows e Linux. Veja [o que vem a seguir](../docs/architecture.md#not-now).

## Download

Baixe `FinchGram-<versão>-macos-arm64.zip` da [última versão](https://github.com/FinchGram/FinchGram/releases/latest),
descompacte e mova o FinchGram para Aplicativos. Precisa de macOS 12 ou mais novo em Apple silicon.
O app é assinado com o Developer ID da North Brookville LLC e notarizado pela Apple, então abre
como qualquer outro. Daí em diante ele se atualiza sozinho.

Cada versão vem com `SHA256SUMS` e sua assinatura Ed25519. O app confere as duas antes de instalar uma
atualização; você também pode, com a chave pública em [release-signing.pub](../release-signing.pub).

## Três temas

| Workbench | Broadsheet | Terminal |
|---|---|---|
| ![Workbench](../docs/screenshots/workbench-light-chats.png) | ![Broadsheet](../docs/screenshots/broadsheet-light-chats.png) | ![Terminal](../docs/screenshots/terminal-light-chats.png) |

O Workbench, o padrão, é uma janela de trabalho com abas. O Broadsheet se lê como um jornal, com uma
cor de destaque por conta. O Terminal é uma tela monoespaçada com comandos. Cada um tem um lado claro
e um escuro, seguindo o sistema ou a sua escolha, em Configurações → Aparência.

O Terminal executa comandos na linha do rodapé: pressione ⌘K ou digite `/`. [O tema Terminal](../docs/terminal.md) traz os comandos e as teclas.

<p align="center"><img src="../docs/screenshots/workbench-dark-chats.png" width="800" alt="Workbench, escuro"></p>

## As regras do Telegram

O FinchGram faz o que os apps do Telegram fazem e nada do que eles proíbem. Uma mensagem é marcada
como lida quando você a vê, o outro lado vê você digitando e online como em qualquer app do
Telegram, as mensagens patrocinadas dos canais aparecem, a mídia que se autodestrói abre uma vez e
nada do Telegram vai para nenhuma IA, a menos que você deixe um assistente entrar (Ferramentas →
Acesso de IA) ou ligue a transcrição de mensagens de voz com o seu próprio provedor de IA. Nada sai da sua
máquina a não ser para o Telegram, para o GitHub para as atualizações e, enquanto você usa o ditado
por voz, a sua voz para o provedor de IA que você escolheu, com a sua própria chave.

## Documentação

Os documentos estão em inglês e em chinês.

- [O que o FinchGram faz](../docs/features.md): cada recurso, com imagens.
- [O tema Terminal](../docs/terminal.md): a linha de comando e os comandos.
- [Compilar a partir do código-fonte](../docs/development.md), [a estrutura do repositório](../docs/layout.md), [as traduções](../docs/translations.md) e [as versões](../docs/releases.md).
- [Arquitetura](../docs/architecture.md) e [convenções](../docs/conventions.md): como é feito, e por quê.

## Desenvolvimento

O app é uma casca: do Telegram cuida a TDLib, que roda como um programa separado ao lado dele e que este repositório compila a partir de fontes fixadas, como a libmpv e o Skia. Para compilar é preciso macOS em Apple silicon e um API ID do Telegram próprio; os passos estão em [docs/development.md](../docs/development.md).

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
