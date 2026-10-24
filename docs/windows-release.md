# Windows 다운로드판 설치

일반적인 Intel·AMD Windows 10/11 64비트 PC용입니다. Git, Rust, Visual Studio,
Python을 설치하거나 소스를 빌드할 필요가 없습니다. Codex는 별도로 설치하고
로그인해 주세요. Discord 봇은 본인 계정으로 만들고 본인 서버에 초대해야 합니다.

## 1. 다운로드와 설치

1. [GitHub Releases](https://github.com/simdorei/codex-discord-remote-rust/releases)에서
   `codex-discord-remote-버전-windows-x64.zip`과 같은 이름의 `.sha256` 파일을 받습니다.
   GitHub가 자동 제공하는 **Source code (zip)**에는 실행 파일이 없습니다.
2. ZIP을 계속 사용할 폴더에 **모두 압축 해제**합니다. ZIP 안에서 직접 실행하지 마세요.
3. 압축 해제한 폴더에서 `install.cmd`를 실행합니다. Windows에 로그인한 본인 계정으로
   실행하세요. 설치 결과와 오류는 창에 표시되고, 키를 누를 때까지 창이 유지됩니다.
4. 설치 완료 메시지를 확인합니다. 설치 도구는 파일을 검사하고 봇·보조 도구와
   Codex 플러그인을 준비합니다. 봇을 시작하거나 자동 실행을 등록하지 않습니다.

다운로드 파일이 손상됐는지 확인하려면 PowerShell에서 다음 명령의 결과를
`.sha256` 파일 내용과 비교하세요. 설치 도구도 묶음 안의 파일을 모두 검사합니다.

```powershell
Get-FileHash .\codex-discord-remote-버전-windows-x64.zip -Algorithm SHA256
```

설치만 확인하려면 다음 명령을 실행하세요. 도움말만 출력합니다.

```powershell
.\target\release\cdr-runtime.exe --help
```

`-DryRun`은 파일을 변경하지 않고 검사합니다.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1 -Prebuilt -DryRun
```

## 2. 준비됐을 때 별도로 설정하고 시작

이 단계는 선택입니다. 실행하면 Discord 봇 토큰을 입력받아 이 폴더의 `.env`에
저장하고, Windows 자동 실행 작업을 등록한 뒤 봇을 시작합니다.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\setup-discord-bot.ps1
```

자동 실행 작업을 등록할 권한이 필요합니다. 토큰은 채팅에 보내거나 공유하지 마세요.
실행 전 `.env.example`의 허용 서버·사용자·채널 설정을 확인하세요.

## 파일이 빠졌거나 기존 설치가 있는 경우

- `RELEASE_INCOMPLETE`: 파일이 누락됐습니다. 같은 릴리스 ZIP을 다시 모두 압축 해제하세요.
- `RELEASE_DAMAGED`: 파일 내용이 다운로드 묶음과 다릅니다. 오류에 표시된 파일을 확인하고
  원본 ZIP에서 새 폴더로 다시 압축 해제하세요. 설치 도구가 임의로 빌드하지 않습니다.
- Codex를 찾지 못하면 Codex 설치를 확인하거나
  `install.ps1 -Prebuilt -CodexExe '실제 Codex 실행 파일 경로'`로 지정하세요.
- 기존 봇 폴더 위에 ZIP을 덮어 풀지 마세요. 다른 버전의 실행 파일이 이미 설치돼 있으면
  설치 도구가 교체를 거부합니다. 기존 `.env`, 대화 기록, 연결 정보는 삭제하지 않습니다.
  실행 중인 봇의 업데이트는 별도의 검증된 배포 절차로 진행해야 합니다.

기본 봇 설치와 시작에는 개발도구가 필요 없습니다. 선택 기능인 Chrome 제어용
Pro 연결, 원격 MCP 서버, 개발·성능 진단은 각 기능 문서의 별도 준비 사항을 따릅니다.

## 배포본을 만드는 개발자

소스 저장소와 고정된 Rust 도구, Windows C++ 빌드 도구가 있는 환경에서 실행합니다.

```powershell
.\scripts\New-WindowsRelease.ps1 -Version 0.1.0
.\scripts\Test-WindowsRelease.ps1 -Archive .\dist\codex-discord-remote-0.1.0-windows-x64.zip
```

빌드는 공용 C 실행 구성요소를 실행 파일에 포함하며, 별도 VC++ 재배포 패키지에
대한 의존성이 남지 않았는지 검사합니다. 패키지에는 실행 파일 두 개, 설치·운영
스크립트, Codex 플러그인, 예제 설정과 안내가 들어갑니다. 개인 `.env`, DB, 로그,
첨부파일, 인증 정보는 포함하지 않습니다. 실행 파일은 Git 소스 이력에 넣지 않습니다.

`Windows release` GitHub Actions는 빌드·압축 해제·설치 검사를 수행하고 다운로드
아티팩트를 남깁니다. `v0.1.0` 같은 버전 태그에서는 같은 검사에 통과한 ZIP과
체크섬을 GitHub Release에도 첨부합니다.
