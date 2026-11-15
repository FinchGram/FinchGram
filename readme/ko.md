# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · [Español](es.md) · [Português](pt.md) · [Deutsch](de.md) · [Français](fr.md) · [Русский](ru.md) · [日本語](ja.md) · **한국어** · [العربية](ar.md)

<p align="center"><img src="../ui/logo/png/FinchGram-icon-128.png" width="96" alt="FinchGram"></p>

**초능력을 갖춘 Telegram 데스크톱 클라이언트. Rust와 [Slint](https://slint.dev)로 만들었습니다.** GPL-3.0
오픈 소스입니다. 지금은 Apple silicon macOS용이고, Windows와 Linux는 나중에 지원합니다.

FinchGram은 Telegram 공식 라이브러리 [TDLib](https://core.telegram.org/tdlib)을 통해 Telegram과 통신하며
[Telegram API 약관](https://core.telegram.org/api/terms)을 지킵니다. 비공식 클라이언트이며 Telegram이 만든 것이 아닙니다.

<p align="center"><img src="../docs/screenshots/workbench-light-chats.png" width="800" alt="Workbench 테마의 채팅 창"></p>

## 할 수 있는 일

- **Telegram 공식 앱이 하는 일을 그대로.** 폴더, 검색, 최대 열 개 계정의 채팅; 답장, 반응, 봇, 주제, 임시 저장이 있는 메시지; 사진, 동영상, 파일, 음성 메시지; 내 그룹과 채널 관리.
- **스크린샷 도구.** ⌘⇧A로 화면을 멈추고, 창이나 영역을 골라 그 위에 그린 뒤 보냅니다. WeChat처럼 어느 채팅에서나.
- **다운로드.** 내 서버를 위한 다운로드 클라이언트. WebDAV, FTP, FTPS, SFTP 지원: NAS, 시드박스, PikPak. [자세히](../docs/downloads.md)
- **음악.** Subsonic API를 쓰는 내 서버(Navidrome 등)를 위한 플레이어. [자세히](../docs/music.md)
- **라이브 방송.** Owncast, PeerTube, Icecast와 Telegram 자체 방송을 찾고, 팔로우하고, 앱 안에서 봅니다. [자세히](../docs/live-streams.md)
- **음성 입력.** 말하면 입력창에 글이 들어갑니다. AI 업체는 직접 고르고, 키도 내 것을 씁니다. [자세히](../docs/voice-input.md)
- **AI 어시스턴트.** Claude 같은 MCP 앱이, 내가 허락하면 FinchGram을 통해 채팅을 읽고 그 안에서 동작합니다. [자세히](../docs/mcp.md)
- **세 가지 테마.** Workbench, Broadsheet, Terminal, 각각 라이트와 다크. Terminal에는 실제로 쓸 수 있는 명령줄이 있습니다. [사용법](../docs/terminal.md)
- **열한 개 언어**, 그리고 앱이 검증한 뒤 스스로 설치하는 업데이트.

[할 수 있는 모든 일, 그림과 함께](../docs/features.md). 아직 없는 것: 비밀 채팅과 통화, 투표와 예약 메시지, 키워드 필터, Windows와 Linux. [다음에 올 것](../docs/architecture.md#not-now)도 보세요.

## 다운로드

[최신 릴리스](https://github.com/FinchGram/FinchGram/releases/latest)에서 `FinchGram-<버전>-macos-arm64.zip`을 받아
압축을 풀고 FinchGram을 '응용 프로그램'으로 옮깁니다. Apple silicon의 macOS 12 이상이 필요합니다. 앱은 North Brookville
LLC의 Developer ID로 서명되고 Apple의 공증을 받았으므로 다른 앱처럼 그냥 열립니다. 그 뒤로는 스스로
업데이트합니다.

모든 릴리스에는 `SHA256SUMS`와 그 Ed25519 서명이 함께 옵니다. 앱은 업데이트를 설치하기 전에 둘 다 확인합니다.
[release-signing.pub](../release-signing.pub)의 공개 키로 직접 확인할 수도 있습니다.

## 세 가지 테마

| Workbench | Broadsheet | Terminal |
|---|---|---|
| ![Workbench](../docs/screenshots/workbench-light-chats.png) | ![Broadsheet](../docs/screenshots/broadsheet-light-chats.png) | ![Terminal](../docs/screenshots/terminal-light-chats.png) |

기본 테마 Workbench는 탭이 있는 작업 창입니다. Broadsheet는 신문처럼 읽히고 계정마다 강조색이 있습니다. Terminal은
고정폭 글꼴 화면에 명령을 씁니다. 모두 라이트와 다크가 있고, 시스템을 따르거나 직접 고를 수 있습니다(설정 › 외관).

Terminal은 창 맨 아래 줄에서 명령을 실행합니다. ⌘K를 누르거나 `/`를 입력하세요. 명령과 키는 [Terminal 테마](../docs/terminal.md)에 있습니다.

<p align="center"><img src="../docs/screenshots/workbench-dark-chats.png" width="800" alt="Workbench, 다크"></p>

## Telegram의 규칙

FinchGram은 Telegram 공식 앱이 하는 일만 하고, 금지된 일은 하지 않습니다. 메시지는 보는 순간 읽음으로 표시되고, 상대는
다른 Telegram 앱에서처럼 입력 중과 온라인을 볼 수 있으며, 채널의 스폰서 메시지는 표시되고, 자동 삭제 미디어는 한 번만
열리며, Telegram에서 온 것은 도구 상자 → AI 접근에서 어시스턴트를 허용하거나 음성 메시지를 본인의 AI 제공업체로 받아쓰는 설정을 켜지 않는 한 어떤 AI에도 넘기지 않습니다. Telegram과 업데이트용 GitHub, 그리고 음성 입력을 쓰는 동안 본인의 키로 직접 고른 AI 제공업체에 보내는 목소리 외에는 아무것도 컴퓨터 밖으로
나가지 않습니다.

## 문서

문서는 영어와 중국어로 되어 있습니다.

- [FinchGram이 하는 일](../docs/features.md): 모든 기능을 그림과 함께.
- [Terminal 테마](../docs/terminal.md): 명령줄과 명령 목록.
- [소스에서 빌드하기](../docs/development.md), [저장소 구조](../docs/layout.md), [번역](../docs/translations.md), [릴리스](../docs/releases.md).
- [아키텍처](../docs/architecture.md)와 [규칙](../docs/conventions.md): 어떻게 만들었고, 왜 그렇게 했는지.

## 개발

앱은 껍데기입니다. Telegram 자체는 TDLib가 맡고, 앱 옆에서 별도 프로그램으로 실행됩니다. TDLib는 이 저장소가 고정된 소스에서 빌드하며, libmpv와 Skia도 마찬가지입니다. 빌드하려면 Apple silicon의 macOS와 본인의 Telegram API ID가 필요합니다. 단계는 [docs/development.md](../docs/development.md)에 있습니다.

## 기여

이슈와 풀 리퀘스트를 환영합니다. 구조를 바꾸기 전에 [docs/architecture.md](../docs/architecture.md)와
[docs/conventions.md](../docs/conventions.md)를 읽어 주세요: 모든 의존성은 이 저장소가 고정된 소스에서 빌드하고, UI는
세 테마 모두 디자인을 따르며, 앱에 Telegram API 약관에 어긋나는 것은 넣지 않습니다. `cargo test`,
`cargo clippy --all-targets`, `cargo test screenshots -- --ignored`를 깨끗하게 유지하고, 그려진 그림을 확인하세요.
README의 그림도 이 테스트에서 나옵니다: `scripts/readme-pictures.sh`가 `docs/screenshots/`를 갱신합니다.

## 라이선스

GPL-3.0([LICENSE](../LICENSE)). finchgram-tdlib에는 TDLib(Boost Software License 1.0)과
OpenSSL(Apache License 2.0)이 포함되어 있습니다. UI 글꼴은 SIL Open Font License 1.1, 아이콘은 MIT
라이선스를 따릅니다. 각 라이선스 전문이 앱과 함께 배포됩니다.
