# FinchGram

[English](../README.md) · [简体中文](zh-Hans.md) · [繁體中文](zh-Hant.md) · [Español](es.md) · [Português](pt.md) · [Deutsch](de.md) · [Français](fr.md) · [Русский](ru.md) · [日本語](ja.md) · **한국어** · [العربية](ar.md)

<p align="center"><img src="../ui/logo/png/FinchGram-icon-128.png" width="96" alt="FinchGram"></p>

**미디어 센터를 갖춘 Telegram 데스크톱 클라이언트. Rust와 [Slint](https://slint.dev)로 만들었습니다.** GPL-3.0
오픈 소스입니다. 지금은 Apple silicon macOS용이고, Windows와 Linux는 나중에 지원합니다.

FinchGram은 Telegram 공식 라이브러리 [TDLib](https://core.telegram.org/tdlib)을 통해 Telegram과 통신하며
[Telegram API 약관](https://core.telegram.org/api/terms)을 지킵니다. 비공식 클라이언트이며 Telegram이 만든 것이 아닙니다.

<p align="center"><img src="../docs/screenshots/workbench-light-chats.png" width="800" alt="Workbench 테마의 채팅 창"></p>

## 할 수 있는 일

- **채팅.** 전화번호, 인증 코드, 2단계 인증 비밀번호로 로그인하거나 QR 코드를 스캔해 로그인하고, 새 계정을 만들 수도
  있습니다. 채팅 목록에는 Telegram의 폴더, 고정, 읽지 않은 수, 멘션이 표시되고, 검색창(⌘K), 채널 보기, 봇 보기가 있습니다.
- **메시지.** 서식 있는 텍스트(굵게, 기울임, 코드, 링크), 답장과 전달, 사진, 동영상, GIF, 스티커, 파일, 링크 미리보기
  카드. 메시지를 마우스 오른쪽 버튼으로 클릭하면 메뉴가 열립니다: 답장, 수정, 복사, 링크 복사, 전달, 신고, 삭제, 여러 개
  선택. 자동 삭제로 보낸 사진과 동영상은 흐리게 표시되고 한 번만 열 수 있습니다. Telegram 공식 앱과 같습니다.
- **미디어.** 사진과 동영상은 창 전체를 덮는 뷰어에서 열리고, 같은 앨범의 다른 사진도 볼 수 있습니다. 동영상은 소스에서
  빌드한 mpv로 재생합니다. 채팅이 허용하면 '다운로드'에 저장할 수 있습니다.
- **보내기.** 사진, 동영상, 파일은 클립 메뉴에서 보내거나 창에 끌어다 놓거나 붙여 넣어 보낼 수 있습니다. 보내기 전에
  카드로 확인합니다: 설명 추가, 사진으로 또는 파일로, 최대 10개를 앨범으로 묶기, 개인 채팅에서는 자동 삭제 타이머,
  소리 없이 보내기.
- **스크린샷.** 입력란의 가위 또는 ⌘⇧A로 화면을 고정합니다: 창을 선택하거나 영역을 끌어 지정하고, 사각형, 타원, 화살표,
  펜, 글자, 모자이크를 그려 넣은 뒤 바로 보내거나 복사하거나 저장합니다. WeChat처럼, 모든 채팅에서.
- **채팅 메뉴.** 음소거, 고정, 읽음으로 표시, 폴더에 넣기. 상대 차단과 해제, 신고, 그룹이나 채널 나가기, 채팅 삭제.
- **알림.** 새 메시지는 macOS 알림으로 오고 그 수가 Dock 아이콘에 표시됩니다. 창을 닫아도 FinchGram은 Dock에 남고,
  메뉴 막대에 둘 수도 있습니다.
- **세 가지 테마.** Workbench, Broadsheet, Terminal. 각각 라이트와 다크가 있고, 다시 시작하지 않고 바꿀 수 있습니다.
- **설정.** 로그인 시 실행, Enter로 보내기, 스크린샷 단축키, 알림 소리와 미리보기, 2단계 인증, 인터페이스 언어, 그리고
  업데이트: 앱은 GitHub Releases에서 스스로 업데이트하며 검증할 수 없는 것은 설치하지 않습니다.
- **언어.** 인터페이스와 이 README는 11개 언어.

아직 없는 것: 비밀 채팅, 음성 메시지와 통화, 여러 계정, 투표와 예약 메시지, 키워드 필터, Windows와 Linux.
[다음에 올 것](../docs/architecture.md#not-now)도 보세요.

## 다운로드

[최신 릴리스](https://github.com/FinchGram/FinchGram/releases/latest)에서 `FinchGram-<버전>-macos-arm64.zip`을 받아
압축을 풀고 FinchGram을 '응용 프로그램'으로 옮깁니다. Apple silicon의 macOS 12 이상이 필요합니다. 앱은 아직 공증되지
않아서 처음 열 때 macOS가 한 번 묻습니다: 시스템 설정 → 개인정보 보호 및 보안 → '그래도 열기'. 그 뒤로는 스스로
업데이트합니다.

모든 릴리스에는 `SHA256SUMS`와 그 Ed25519 서명이 함께 옵니다. 앱은 업데이트를 설치하기 전에 둘 다 확인합니다.
[release-signing.pub](../release-signing.pub)의 공개 키로 직접 확인할 수도 있습니다.

## 세 가지 테마

| Workbench | Broadsheet | Terminal |
|---|---|---|
| ![Workbench](../docs/screenshots/workbench-light-chats.png) | ![Broadsheet](../docs/screenshots/broadsheet-light-chats.png) | ![Terminal](../docs/screenshots/terminal-light-chats.png) |

기본 테마 Workbench는 탭이 있는 작업 창입니다. Broadsheet는 신문처럼 읽히고 계정마다 강조색이 있습니다. Terminal은
고정폭 글꼴 화면에 명령을 씁니다. 모두 라이트와 다크가 있고, 시스템을 따르거나 직접 고를 수 있습니다(설정 › 외관).

<p align="center"><img src="../docs/screenshots/workbench-dark-chats.png" width="800" alt="Workbench, 다크"></p>

## 스크린샷 도구

<p align="center"><img src="../docs/screenshots/workbench-light-shot-annotated.png" width="800" alt="주석이 있는 스크린샷 오버레이"></p>

⌘⇧A를 누르거나 클립 옆의 가위를 클릭합니다. 화면이 어두워진 채 오버레이 아래에 고정됩니다. 영역을 끌어 지정하거나
(돋보기와 픽셀 단위 크기가 함께 표시됩니다), 창을 클릭해 통째로 고를 수 있습니다. 도구 모음으로 사각형, 타원, 화살표, 펜,
글자, 모자이크를 세 가지 굵기와 여섯 가지 색으로 그릴 수 있고, 실행 취소도 됩니다. Done을 누르면 그림이 보내기 카드에
들어가 설명을 붙일 수 있습니다. ⌘C는 복사, ⌘S는 '다운로드'에 저장. 설정 › 일반 › 스크린샷에서 단축키와, 찍는 동안
FinchGram 창을 숨길지 정할 수 있습니다.

<p align="center"><img src="../docs/screenshots/workbench-light-shot-send-card.png" width="800" alt="스크린샷이 담긴 보내기 카드"></p>

## 어떻게 만들었나

앱은 껍데기입니다. Telegram 자체는 TDLib이 맡고, 실행 파일 옆에서 별도 프로그램 `finchgram-tdlib`으로 실행됩니다. 이것은
이 저장소가 고정된 소스에서 빌드합니다. 껍데기는 `src/telegram/`을 통해서만, TDLib 자체의 JSON으로 통신합니다. 동영상은
같은 방식으로 빌드한 libmpv로 재생합니다. 사용자의 컴퓨터에서 아무것도 가져오지 않고, 빌드할 때 이 고정되고 체크섬이
있는 빌드 외에는 아무것도 내려받지 않습니다. 소스, vendor 빌드(`vendor/`), 릴리스까지 모두 공개입니다.
[docs/architecture.md](../docs/architecture.md)와 [docs/conventions.md](../docs/conventions.md)를 보세요.

## Telegram의 규칙

FinchGram은 Telegram 공식 앱이 하는 일만 하고, 금지된 일은 하지 않습니다. 메시지는 보는 순간 읽음으로 표시되고, 상대는
다른 Telegram 앱에서처럼 입력 중과 온라인을 볼 수 있으며, 채널의 스폰서 메시지는 표시되고, 자동 삭제 미디어는 한 번만
열리며, Telegram에서 온 것은 어떤 AI에도 넘기지 않습니다. Telegram과 업데이트용 GitHub 외에는 아무것도 컴퓨터 밖으로
나가지 않습니다.

## 개발

지금은 개발에 Apple silicon macOS가 필요합니다. finchgram-tdlib을 아직 그 플랫폼용으로만 빌드하기 때문입니다.
Linux와 Windows는 나중에 지원합니다.

finchgram-tdlib, libmpv, UI 글꼴은 git에 들어 있지 않습니다. `vendor/tdlib/`와 `vendor/mpv/`에는 그것들을
빌드하는 스크립트만 있고, 글꼴은 Google Fonts에서 받습니다. clone한 뒤 한 번 받아 오세요:

```sh
scripts/fetch-tdlib.sh    # 고정된 finchgram-tdlib 릴리스를 vendor/tdlib/bin/에 내려받고 SHA-256을 검증
scripts/fetch-mpv.sh      # 고정된 libmpv 릴리스를 vendor/mpv/bin/에 내려받고 SHA-256을 검증
scripts/fetch-fonts.sh    # 고정된 UI 글꼴을 vendor/fonts/에 내려받고 SHA-256을 검증
FINCHGRAM_API_ID=… FINCHGRAM_API_HASH=… cargo run
```

- finchgram-tdlib 자체를 바꾸려면 여기서 빌드하세요(몇 분 걸립니다. Xcode 또는 Command Line Tools와
  `brew install cmake ninja gperf`가 필요하며, 모두 빌드 도구일 뿐입니다):

  ```sh
  vendor/tdlib/build.sh && vendor/tdlib/build.sh install
  ```

- `FINCHGRAM_API_ID`와 `FINCHGRAM_API_HASH`: 자신의 것을
  [my.telegram.org](https://my.telegram.org) → API development tools에서 발급받으세요. 컴파일할 때 읽히며
  절대 저장소에 넣지 않습니다. 이것이 없어도 앱은 실행되며, API ID가 없다고 알려 줍니다.
- `FINCHGRAM_TEST_DC=1 cargo run`은 Telegram의 테스트 서버를 사용합니다. 테스트 서버에는 별도의 계정이
  있으며, 앱은 이를 위해 별도의 데이터베이스를 둡니다.
- `cargo test screenshots -- --ignored`는 모든 테마의 모든 페이지를 라이트와 다크로, 가상의 채팅과 함께
  `target/screenshots/`에 그립니다. 계정 없이 UI를 볼 수 있는 방법입니다.

`build.rs`는 `vendor/tdlib/bin/finchgram-tdlib`을 컴파일된 실행 파일 옆에 복사합니다. 그래서 `cargo run`은
패키징된 앱과 정확히 같은 프로그램을 사용합니다. libmpv는 `vendor/mpv/bin/`에서 링크하고 마찬가지로 옆에
복사합니다. `vendor/fonts/`의 글꼴은 실행 파일에 포함됩니다. 하나라도 없으면 빌드가 그 이유를 알리는 메시지와
함께 실패합니다. Rust 1.92 이상이 필요합니다.

TDLib의 데이터베이스와 내려받은 파일은 `~/Library/Application Support/FinchGram/tdlib/`에,
설정은 `~/Library/Application Support/FinchGram/settings.toml`에 있습니다.

## 구조

```
.github/workflows/
  mpv.yml                # 깨끗한 머신에서 libmpv를 빌드하고, mpv-* 태그를 릴리스로 공개
  release.yml            # main에 push할 때마다 앱을 빌드하고, v* 태그를 릴리스로 공개
  tdlib.yml              # 깨끗한 머신에서 finchgram-tdlib을 빌드하고, tdlib-* 태그를 릴리스로 공개
Cargo.toml
build.rs                 # ui/app.slint를 컴파일하고, lang/을 포함하며, vendor/tdlib/bin/을 실행 파일 옆에 복사
docs/                    # architecture.md, conventions.md, drag-and-drop.md, media-files.md (+ zh-Hans)
  screenshots/           #   README에 싣는 그림, 스크린샷 테스트에서 (scripts/readme-pictures.sh)
lang/                    # 번역: lang/<코드>/LC_MESSAGES/finchgram.po, 바이너리에 포함됨
readme/                  # 이 README의 다른 언어판
release-signing.pub      # 릴리스에 서명할 수 있는 공개 키; 앱에 포함됨
scripts/
  bundle.sh              # dist/FinchGram.app을 빌드(릴리스 워크플로가 실행하는 것)
  fetch-fonts.sh         # 고정된 UI 글꼴을 vendor/fonts/로
  fetch-mpv.sh           # 고정된 libmpv 릴리스를 vendor/mpv/bin/으로
  fetch-tdlib.sh         # 고정된 finchgram-tdlib 릴리스를 vendor/tdlib/bin/으로
  readme-pictures.sh     # README의 그림을 target/screenshots/에서 docs/screenshots/로 복사
  release.sh             # 릴리스 시작: 버전, 태그, push; 나머지는 GitHub Actions가 처리
src/
  main.rs                # 창, 설정, 언어와 테마, 업데이트. Telegram을 시작
  fonts.rs               # UI 글꼴. 실행 파일에 포함됨
  images.rs              # 메시지의 이미지. UI 스레드 밖에서 디코딩
  telegram/              # finchgram-tdlib과 통신하는 유일한 코드
    process.rs           #   프로그램 실행: 표준 입출력으로 TDLib의 JSON을 주고받음
    api.rs               #   FinchGram이 쓰는 TDLib 타입(고정 버전의 td_api.tl 기준)
    mod.rs               #   요청과 응답, 다시 시작. 업데이트는 store로
    store.rs             #   TDLib이 알려 준 채팅·사용자·메시지. 페이지의 모델
    login.rs             #   로그인, 가입
    chats.rs             #   채팅 목록
    conversation.rs      #   열린 채팅: 메시지, 보내기
    actions.rs           #   메시지 작업: 오른쪽 클릭 메뉴, 답장, 전달 등
    account.rs           #   프로필, 로그아웃
    password.rs          #   설정의 2단계 인증
    files.rs             #   파일 다운로드
    avatars.rs           #   채팅과 사람의 사진. 이니셜 대신 표시
    viewer.rs            #   미디어 뷰어: 사진, 동영상, ‘다운로드’에 저장
    rich_text.rs         #   서식 있는 텍스트: 굵게, 기울임, 링크 등
    notifications.rs     #   새 메시지 알림, Dock 아이콘의 읽지 않은 수
    online.rs            #   창이 앞에 있고 쓰이는 동안 계정은 온라인
  platform/              # 운영 체제마다 달라지는 부분
  player/                # libmpv로 동영상 재생. OpenGL로 창에 그림
  screenshot/            # 스크린샷 도구: 캡처, 오버레이, 주석, 그림
  update.rs              # 자동 업데이트: GitHub Releases, 서명 확인, 교체, 재시작
  settings.rs            # 사용자 설정(settings.toml)
  i18n.rs                # UI 언어: 저장된 선택, 없으면 시스템 언어, 그것도 없으면 영어
  screenshots.rs         # 모든 페이지를 PNG로 그림(cargo test screenshots -- --ignored)
  bin/                   # finchgram-release-sign.rs, 릴리스에 쓰는 Ed25519 서명 도구
ui/
  app.slint              # 메인 창: 메뉴와, 어떤 페이지를 보여 줄지
  state.slint            # 앱의 상태. Rust와 페이지가 함께 씀
  telegram.slint         # Telegram이 보여 주는 것: 계정, 로그인, 채팅, 메시지
  look.slint             # 테마의 색·글꼴·모양. 공유 페이지용
  format.slint           # UI 언어로 쓴 날짜·수·메시지 종류
  widgets.slint          # 작은 공유 부품. chat.slint는 채팅 창의 공유 부분
  viewer.slint           # 창 전체를 덮는 미디어 뷰어. 테마마다 다른 모양
  pages/                 # 세 테마가 함께 쓰는 페이지: 로그인, 설정, 프로필
  workbench/             # 워크벤치 테마의 채팅 창(기본)
  broadsheet/            # 브로드시트 테마의 채팅 창
  terminal/              # 터미널 테마의 채팅 창
  icons/                 # Phosphor 아이콘(MIT), 일반과 듀오톤. icons.slint가 목록
  logo/                  # FinchGram 로고(svg, png)와 사용 규칙
vendor/fonts/            # git에 없음: UI 글꼴(scripts/fetch-fonts.sh)
vendor/mpv/              # libmpv: 동영상 재생용 mpv와 FFmpeg
  build.sh               #   고정된 소스로 빌드: 버전과 SHA-256은 맨 위에 고정
  bin/                   #   git에 없음: 라이브러리 자체(scripts/fetch-mpv.sh 또는 build.sh install)
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

그런 다음 `msgstr` 항목을 채우고 다시 `cargo build`를 실행하세요. 원본 언어는 영어이며, 나머지 10개 언어가
포함되어 있습니다.

## 릴리스

릴리스는 오직 GitHub Actions가 태그가 붙은 커밋으로 깨끗한 머신에서 빌드하며
(`.github/workflows/release.yml`), 이 저장소에 공개합니다: 압축한 앱, `SHA256SUMS`, 그리고 그 Ed25519
서명. 설치된 앱은 여기서 스스로 업데이트하며, 검증할 수 없는 것은 아무것도 설치하지 않습니다.
메인테이너는 명령 하나로 릴리스를 시작합니다:

```sh
scripts/release.sh patch    # 0.1.0 -> 0.1.1; minor, major 또는 정확한 버전도 가능
```

서명은 Apple Developer ID가 아니라 프로젝트 자체 인증서로 하므로, 내려받은 앱을 처음 열 때 macOS가 한 번
확인을 요청합니다 (시스템 설정 → 개인정보 보호 및 보안). 앱이 직접 설치한 업데이트는 이 단계 없이 실행됩니다.

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
