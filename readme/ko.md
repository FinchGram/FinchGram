# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · [Español](es.md) · [Português](pt.md) · [Deutsch](de.md) · [Français](fr.md) · [Русский](ru.md) · [日本語](ja.md) · **한국어** · [العربية](ar.md)

미디어 센터를 갖춘 오픈 소스 Telegram 데스크톱 클라이언트로, Rust + [Slint](https://slint.dev)로
작성되었습니다. macOS(Apple silicon)를 먼저 지원하고, Windows와 Linux는 나중에 지원합니다.

FinchGram은 Telegram API를 사용하며 Telegram 생태계의 일부입니다. 비공식 클라이언트이며,
Telegram이 만든 것이 아닙니다.

상태: 초기 단계. 셸이 TDLib을 시작하고 로그인 진행 상황을 따라갑니다. 다음은 디자인에 따라 페이지를
만드는 것입니다.

앱은 셸(껍데기)입니다. Telegram 자체는 Telegram 공식 라이브러리인 TDLib이 맡으며, 실행 파일 옆에서
별도의 프로그램으로 실행됩니다. 그 프로그램이 `finchgram-tdlib`이고, 이 저장소가 버전을 고정한 소스로
빌드합니다(Coova Studio의 ffmpeg처럼). 셸은 오직 `src/telegram/`을 통해서만, TDLib 고유의 JSON으로 이
프로그램과 통신합니다. [docs/architecture.md](../docs/architecture.md)와
[docs/conventions.md](../docs/conventions.md)(영어)를 참고하세요.

모든 것이 여기에 공개되어 있습니다: 소스 코드, 의존성 빌드(`vendor/`), 릴리스.

## 개발

지금은 개발에 Apple silicon macOS가 필요합니다. finchgram-tdlib을 아직 그 플랫폼용으로만 빌드하기 때문입니다.
Linux와 Windows는 나중에 지원합니다.

finchgram-tdlib 자체는 git에 들어 있지 않습니다. `vendor/tdlib/`에는 그것을 빌드하는 스크립트만 있습니다.
clone한 뒤 프로그램을 한 번 받아 오세요:

```sh
scripts/fetch-tdlib.sh    # 고정된 finchgram-tdlib 릴리스를 vendor/tdlib/bin/에 내려받고 SHA-256을 검증
FINCHGRAM_API_ID=… FINCHGRAM_API_HASH=… cargo run
```

- 첫 `tdlib-*` 릴리스가 공개되기 전에는 `scripts/fetch-tdlib.sh`가 그렇다고 알려 줍니다. 그럴 때는 여기서
  finchgram-tdlib을 빌드하세요(몇 분 걸립니다. Xcode 또는 Command Line Tools와
  `brew install cmake ninja gperf`가 필요하며, 모두 빌드 도구일 뿐입니다):

  ```sh
  vendor/tdlib/build.sh && vendor/tdlib/build.sh install
  ```

- `FINCHGRAM_API_ID`와 `FINCHGRAM_API_HASH`: 자신의 것을
  [my.telegram.org](https://my.telegram.org) → API development tools에서 발급받으세요. 컴파일할 때 읽히며
  절대 저장소에 넣지 않습니다. 이것이 없어도 앱은 실행되며, API ID가 없다고 알려 줍니다.
- `FINCHGRAM_TEST_DC=1 cargo run`은 Telegram의 테스트 서버를 사용합니다. 테스트 서버에는 별도의 계정이
  있으며, 앱은 이를 위해 별도의 데이터베이스를 둡니다.

`build.rs`는 `vendor/tdlib/bin/finchgram-tdlib`을 컴파일된 실행 파일 옆에 복사합니다. 그래서 `cargo run`은
패키징된 앱과 정확히 같은 프로그램을 사용합니다. 이것이 없으면 빌드가 그 이유를 알리는 메시지와 함께
실패합니다. Rust 1.92 이상이 필요합니다.

TDLib의 데이터베이스와 내려받은 파일은 `~/Library/Application Support/FinchGram/tdlib/`에,
설정은 `~/Library/Application Support/FinchGram/settings.toml`에 있습니다.

## 구조

```
.github/workflows/
  release.yml            # main에 push할 때마다 앱을 빌드하고, v* 태그를 릴리스로 공개
  tdlib.yml              # 깨끗한 머신에서 finchgram-tdlib을 빌드하고, tdlib-* 태그를 릴리스로 공개
Cargo.toml
build.rs                 # ui/app.slint를 컴파일하고, lang/을 포함하며, vendor/tdlib/bin/을 실행 파일 옆에 복사
docs/                    # architecture.md, conventions.md (+ zh-Hans)
lang/                    # 번역: lang/<코드>/LC_MESSAGES/finchgram.po, 바이너리에 포함됨
readme/                  # 이 README의 다른 언어판
release-signing.pub      # 릴리스에 서명할 수 있는 공개 키; 앱에 포함됨
scripts/
  bundle.sh              # dist/FinchGram.app을 빌드(릴리스 워크플로가 실행하는 것)
  fetch-tdlib.sh         # 고정된 finchgram-tdlib 릴리스를 vendor/tdlib/bin/으로
  release.sh             # 릴리스 시작: 버전, 태그, push; 나머지는 GitHub Actions가 처리
src/
  main.rs                # 창, 설정, 언어, 업데이트; Telegram을 시작
  telegram/              # finchgram-tdlib과 통신하는 유일한 코드
    process.rs           #   프로그램 실행: 표준 입출력으로 TDLib의 JSON을 주고받음
    api.rs               #   FinchGram이 쓰는 TDLib 타입(고정 버전의 td_api.tl 기준)
    mod.rs               #   요청과 응답, 재시작, 로그인
  platform/              # 운영 체제마다 달라지는 부분
  update.rs              # 자동 업데이트: GitHub Releases, 서명 확인, 교체, 재시작
  settings.rs            # 사용자 설정(settings.toml)
  i18n.rs                # UI 언어: 저장된 선택, 없으면 시스템 언어, 그것도 없으면 영어
  bin/                   # finchgram-release-sign.rs, 릴리스에 쓰는 Ed25519 서명 도구
ui/
  app.slint              # 메인 창
  state.slint            # Rust와 페이지가 공유하는 global
  theme.slint            # 색상, 라이트와 다크
  logo/                  # FinchGram 로고(svg, png)와 사용 규칙
vendor/tdlib/            # finchgram-tdlib
  build.sh               #   고정된 소스로 빌드: TDLib 커밋, OpenSSL 버전과 SHA-256은 맨 위에 고정
  host/                  #   작은 호스트 프로그램(main.cpp)과 그 CMakeLists.txt
  bin/                   #   git에 없음: 앱이 쓰는 프로그램(scripts/fetch-tdlib.sh 또는 build.sh install)
  work/, dist/           #   git에 없음: 로컬 빌드의 중간 파일과 패키지
```

## 번역

UI의 모든 텍스트는 `@tr("English text")`로 씁니다. 같은 문자열은 어디에 나오든 번역이 하나입니다
(build.rs가 Slint의 기본 컨텍스트를 끕니다). 같은 영어 문장이 다른 곳에서 다른 말로 옮겨져야 하면
컨텍스트를 붙이세요: `@tr("menu" => "Open")`. 공식 도구로 추출한 뒤 각 언어에 병합합니다:

```sh
cargo install slint-tr-extractor                                          # 한 번만
find ui -name '*.slint' | sort | xargs slint-tr-extractor --no-default-translation-context -o lang/finchgram.pot
for po in lang/*/LC_MESSAGES/finchgram.po; do msgmerge --update "$po" lang/finchgram.pot; done   # brew install gettext 필요
```

그런 다음 `msgstr` 항목을 채우고 다시 `cargo build`를 실행하세요. 원본 언어는 영어이며, 현재 중국어 간체가
포함되어 있습니다.

## 릴리스

릴리스는 오직 GitHub Actions가 태그가 붙은 커밋으로 깨끗한 머신에서 빌드하며
(`.github/workflows/release.yml`), 이 저장소에 공개합니다: 압축한 앱, `SHA256SUMS`, 그리고 그 Ed25519
서명. 설치된 앱은 여기서 스스로 업데이트하며, 검증할 수 없는 것은 아무것도 설치하지 않습니다.
메인테이너는 명령 하나로 릴리스를 시작합니다:

```sh
scripts/release.sh patch    # 0.1.0 -> 0.1.1; minor, major 또는 정확한 버전도 가능
```

지금은 ad hoc 서명이라, 내려받은 앱을 처음 열 때 macOS가 한 번 확인을 요청합니다
(시스템 설정 → 개인정보 보호 및 보안). 앱이 직접 설치한 업데이트는 이 단계 없이 실행됩니다.

## 라이선스

GPL-3.0([LICENSE](../LICENSE)). finchgram-tdlib에는 TDLib(Boost Software License 1.0)과
OpenSSL(Apache License 2.0)이 포함되어 있으며, 각 라이선스 전문이 함께 배포됩니다.
