# Codex Discord Remote — Rust

Discord에서 Codex 작업을 요청하고, 진행 상황과 최종 답변을 받아 보는
**Rust 기본 실행 버전**입니다.

기존 [codex-discord-remote](https://github.com/simdorei/codex-discord-remote)에서
현재 Rust 구현을 분리했습니다. 과거 Git 변경 이력, 실제 대화 기록,
사용자 설정, 데이터베이스 및 실행 파일은 포함하지 않습니다.

## 포함된 기능

- Codex app-server를 통한 요청 처리, 대기열 및 진행 중 요청 수정(steering)
- Codex 스레드와 Discord 스레드 연결, 메시지 미러링, 최종 답변 전달
- 새 스레드, 보관, 모델 설정, 사용량, 진단 및 도움말 명령
- Windows 트레이, 상태 확인, 재시작 및 자동 실행 보조 도구
- 선택 기능인 ChatGPT Pro 연결, 원격 MCP 서버와 PC 에이전트
- 재시작·메시지 전달·저장 상태를 검증하는 자동 테스트

실제 명령 목록은 Discord의 `!help` 또는 `/help`를 확인하세요.
`!new` 다음에 보내는 메시지는 새 Codex 스레드의 첫 요청입니다.

## 실행 구조

`crates/`에 Rust 작업 공간이 있으며, `cdr-runtime`이 Discord 봇 실행 파일입니다.
저장 상태는 SQLite, Codex 연결은 app-server가 담당합니다.

Python 파일도 의도적으로 남아 있습니다. 설치·설정·플러그인 검사와
기존 동작 비교 테스트, 명시적으로 선택하는 수동 Python 복구 경로에 필요합니다.
**봇의 기본 실행은 Rust이며, 모든 보조 도구까지 Rust로 바꾼 저장소는 아닙니다.**
Rust 실패 시 Python으로 자동 전환하지 않습니다.

## Windows 설치

필요한 항목:

- Git, 로그인된 Codex 및 app-server를 제공하는 Codex 실행 파일
- `rust-toolchain.toml`에 고정된 Rust 1.97.1
- Rust Windows 빌드에 필요한 Visual Studio C++ Build Tools와 Windows SDK
- 직접 만든 Discord 봇과 본인 서버의 관리 권한

```powershell
git clone https://github.com/simdorei/codex-discord-remote-rust.git
cd codex-discord-remote-rust
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\setup-discord-bot.ps1
```

설치 도구는 Python 보조 실행 환경과 Rust 봇을 준비합니다.
설정 도구는 로컬 `.env`를 만들고 Windows 자동 실행 작업을 등록합니다.
기존 봇이 있는 PC에서는 설정 도구를 중복 실행하지 마세요.
같은 Discord 봇 토큰으로 두 실행본을 동시에 켜지 마세요.

설치 전에 변경 없이 확인하려면:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1 -DryRun -SkipDependencies -SkipEnvFile
```

`.env.example`을 참고해 허용할 Discord 서버·사용자·채널을 좁게 설정하세요.
토큰은 `.env`에만 보관하며 Git에 올리지 않습니다.
MCP 예제의 `mcp.example.com`은 실제 주소가 아닙니다. 선택 기능을 사용할 때만
본인이 관리하는 서버 주소와 기기별 인증 값을 설정하세요.

## 직접 빌드 및 실행

```powershell
cargo build --release --locked -p cdr-runtime
.\target\release\cdr-runtime.exe --help
.\target\release\cdr-runtime.exe --env .\.env --check-config
.\target\release\cdr-runtime.exe --env .\.env
```

설치 후에는 `codex-discord-bot.cmd`로 트레이 실행을 사용할 수 있습니다.
상태 확인은 다음 명령으로 실행합니다. 이 명령은 봇을 시작하거나 멈추지 않습니다.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\codex-discord-rust-status.ps1
```

이 저장소 공개 작업은 기존 설치의 업데이트나 재시작을 수행하지 않습니다.
배포·복구 스크립트는 실제 실행 중인 봇에 영향을 줄 수 있으므로
내용과 대상 폴더를 확인한 뒤 사용하세요.

## 테스트

일반 자동 테스트는 가짜 서버와 임시 데이터로 실행합니다.
실제 계정·기기를 사용하는 `ignored` 테스트는 자동으로 실행하지 않습니다.
일부 테스트에는 Python 3.12와 PowerShell, Node.js가 필요합니다.
Python 비교 테스트에는 `requirements.txt`의 봇 보조 환경과
`remote_mcp_server/uv.lock`의 별도 MCP 환경이 필요합니다.
두 고정 버전 묶음을 한 환경에 섞지 마세요. 설치 순서와 `PYTHON_EXE` 지정은
[Windows 자동 검사 설정](.github/workflows/windows-contract.yml)을 참고하세요.
백업 패키징 시험 중 일부는 PC 전체에 실행 중인 봇이 없어야 합니다.
운용 중인 봇이 있다면 해당 시험은 GitHub Actions의 새 검사 환경에서 실행하세요.
이 저장소의 테스트를 위해 기존 봇을 자동으로 정지하지 않습니다.

```powershell
cargo test --workspace --locked
powershell -NoProfile -ExecutionPolicy Bypass -File .\plugins\codex-discord-remote\scripts\qa-smoke.ps1
```

`Cargo.lock`과 `rust-toolchain.toml`을 함께 유지해 빌드 버전을 고정합니다.
실제 Discord 송수신 확인은 자동 테스트와 별개이며, 본인 테스트 채널에서 진행하세요.

## 선택 기능: Rust MCP 서버

`cdr-mcp-server`와 `cdr-remote-agent`가 서버 및 PC 측 코드를 제공합니다.
서버 배포 예시는 `remote_mcp_server/Dockerfile.rust`와 `compose.rust.yaml`입니다.
이 공개 작업에서는 VPS나 기존 MCP 서버를 변경하지 않습니다.
인증 설정과 HTTPS 프록시는 본인 환경에 맞게 별도로 구성해야 합니다.

## 범위와 제한

- Windows 트레이와 운영 스크립트가 주 설치 경로입니다.
- macOS/Linux에 대한 동일 기능 보장을 의미하지 않습니다.
- Computer Use의 화면 캡처 가능 여부는 Codex 도구와 운영체제 지원에 달려 있습니다.
- 개인 작업 기록, 배포 영수증, 로그, DB, 첨부파일 및 비밀 값은 공개하지 않습니다.
- Rust의 정비 예제와 `scripts/Register-CdrMaintenance.ps1`은 가짜 경로·식별자를 사용합니다.
  실행 가능한 개인 배포 지시서가 아니며, 별도 검토 없이 실행하지 마세요.
- 개인 배포 인증 기록은 가져오지 않아 `drain-certified-artifacts.json`은 빈 목록입니다.
  검증된 종료 절차를 요구하는 관리형 배포는 새 환경의 검증 기록이 없으면 거부됩니다.
- Pro 연결에는 기존 `Simdorei Local Project Oauth` 이름·주소 계약이 남아 있습니다.
  임의의 MCP 주소만 바꾼다고 Pro 커넥터까지 자동 변경되지는 않습니다.
- `NOTICE.md`와 각 플러그인의 고지문에 포함된 출처 표시를 유지합니다.
