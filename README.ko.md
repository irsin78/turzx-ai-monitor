# turzx-ai-monitor

Windows용 **TURZX 8.8" USB 바 LCD**(1920×480) 대시보드로, 제조사 프로그램을 대신합니다.
**Claude, Codex, Antigravity** 요금제 사용량이 얼마나 남았는지와 함께 CPU / GPU / 램 / VRAM
사용률·온도·소비전력, 팬, 네트워크, 시계, 공휴일이 표시되는 달력, 날씨와 미세먼지를 보여줍니다.

[English README](README.md)

![대시보드](docs/images/dashboard-ko.png)

Windows가 종료되면 패널에 대기 화면을 띄우고, 다음에 켤 때까지 유지합니다.

![대기 화면](docs/images/standby.png)

## 기능

- **AI 사용량**: 5시간 한도와 주간 한도의 남은 양과 초기화까지 남은 시간. 초기화되면 바로 다시 조회합니다.
- **하드웨어**: CPU(코어별 그래프 포함), 메모리, GPU, VRAM 사용률과 기록 그래프, 온도, CPU 패키지·GPU
  보드 전력, 메인보드 팬 2개와 GPU 팬, 네트워크 업·다운로드 속도
- **시계와 달력**(12/24시간) — 사는 나라의 공휴일 표시
- **날씨**: 지금, 내일, 일출·일몰, 초미세·미세먼지(한국 환경부 등급 또는 미국 AQI)
- **16개 언어**: English, 한국어, 日本語, 简体中文, 繁體中文, Español, Français, Deutsch, Italiano,
  Português, Русский, Polski, Türkçe, Nederlands, Tiếng Việt, Bahasa Indonesia
- **가벼움**: 메모리 약 65MB, Windows 효율 모드에서 코어 1개 기준 약 5%
- **튼튼함**: USB 선을 건드려 끊겨도 1초 안팎으로 다시 연결, 드라이버 문제로 죽어도 스스로 재시작,
  PC를 잠그면 화면 끄기(검은 화면)

## 필요 사항

- Windows 10 / 11 (x64)
- TURZX 8.8" USB LCD (USB `1CBE:0088`, 480×1920). 패널은 한 프로그램만 쓸 수 있으니 제조사 프로그램은 먼저 종료하세요.
- 선택
  - NVIDIA GPU: GPU·VRAM 정보
  - [PawnIO](https://pawnio.eu) (FanControl을 설치하면 함께 설치됨): CPU·램 온도, CPU 전력, 메인보드 팬.
    메인보드 팬 채널은 현재 Nuvoton Super I/O 칩 기준입니다.
  - 이 PC에 로그인된 Claude Code, Codex, Antigravity: AI 사용량

## 설치

1. [Releases](../../releases)에서 `turzx-dashboard.exe`를 받습니다(또는 아래처럼 빌드).
2. 더블클릭하고 **예**를 누르면 설치됩니다. `%LOCALAPPDATA%\Programs\TurzxDashboard`에 복사되고, 시작 메뉴
   바로가기와 *설정 → 앱* 항목이 생기며, Windows와 함께 시작되고 바로 실행됩니다.
   - PawnIO가 있으면 관리자 권한 예약 작업으로 자동 실행합니다(설치할 때 UAC 한 번, 로그인 때는 없음).
   - PawnIO가 없으면 일반 시작 프로그램으로 등록합니다(UAC 없음).

터미널에서:

```
turzx-dashboard install [--no-autostart]
turzx-dashboard uninstall [--purge]     # --purge: 설정·로그·캐시까지 삭제
turzx-dashboard autostart on|off
turzx-dashboard settings                # 설정 파일 열기
turzx-dashboard run                     # 설치 없이 실행
```

제거는 *설정 → 앱 → TURZX AI Monitor* 또는 `turzx-dashboard uninstall`로 합니다.

## 설정

`%APPDATA%\TurzxDashboard\config.toml` (시작 메뉴: *TURZX AI Monitor settings*). 저장하면 대시보드가
새 설정으로 다시 시작합니다. 항목은 [영어 README](README.md#settings)와 같습니다: 언어, 12/24시간,
섭씨/화씨, 도시(또는 위도·경도), 공휴일 나라, 미세먼지 기준, 켤 AI 서비스, 팬 이름.

## AI 사용량을 읽는 방식

각 도구가 이 PC에 저장해 둔 로그인 정보를 읽어, 그 도구들과 [CodexBar](https://github.com/steipete/CodexBar)가
쓰는 사용량 엔드포인트에 묻습니다. 다른 곳으로 보내는 정보는 없고, 로그인 정보는 읽기만 합니다.

- **Claude**: Claude Code의 사용량 API를 15분마다(요청 제한이 엄격함). 실패하면 API가 다시 될 때까지 Claude
  Code의 `/usage` 화면을 5분마다 읽습니다.
- **Codex**: Codex CLI 로그인으로 ChatGPT 사용량 API를 5분마다
- **Antigravity**: Antigravity 로그인으로 Google Cloud Code 할당량 API를 5분마다

공개 API가 아니라서 바뀌거나 막힐 수 있습니다. 원하지 않는 서비스는 `[ai]`에서 끄세요.

## 데이터 출처

- 날씨·미세먼지: [Open-Meteo](https://open-meteo.com) (키 불필요). 미세먼지는 측정소 값이 아니라 CAMS 모델 값입니다.
- 공휴일: 한국은 내장 표, 다른 나라는 [Nager.Date](https://date.nager.at)
- 로그: `%LOCALAPPDATA%\TurzxDashboard\dashboard.log`

## 빌드

Windows에서 Rust(stable)가 필요합니다.

```
cd rust
cargo build --release
```

`turzx-dashboard diag <명령>`으로 패널 없이 미리보기와 점검을 할 수 있습니다:
`layout`, `standby`, `sensors`, `weather`, `profile`, `jpeg-check`, `fps-test`, `test-pattern`.
패널 구동 방식(USB 프로토콜, JPEG 프레임, 부분 재압축)은 [docs/protocol.md](docs/protocol.md)에 있습니다.

## 라이선스

MIT ([LICENSE](LICENSE)). 함께 들어 있는 폰트와 PawnIO 모듈은 각자의 라이선스를 따릅니다:
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). TURZX, Anthropic, OpenAI, Google과 관계없는 개인 프로젝트입니다.
