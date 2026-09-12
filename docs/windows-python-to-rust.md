# Windows: Python판에서 Rust판으로 전환하기

같은 Windows PC에서 기존 Codex 대화와 Discord 대화방 연결을 유지하면서
Python판 대신 Rust판을 실행하는 절차입니다. 다른 PC로 Codex 계정·대화 기록까지
옮기는 절차는 아닙니다. 다른 PC에서는 기기별 경로와 MCP 인증도 별도로 확인해야 합니다.

요약: **새 폴더와 설정 준비 → 설치·실행 파일 제작 → 구버전 정지 → 연결 정보 이전 →
신버전 설정·시작 → 송수신 확인**입니다. 설치 도구는 Python판 정지와 데이터
이전을 자동 수행하지 않습니다. 아래 단계는 순서대로 진행하세요.

## 1. 구버전을 유지한 채 Rust 실행 파일 준비

[README의 Windows 필수 항목](../README.md#windows-설치)을 먼저 준비하세요.
Python은 필요 없지만, 이 저장소는 소스를 배포하므로 Rust와 Windows C++ 빌드
도구가 필요합니다. 기존 Python 프로그램이나 공용 Python 설치를 삭제하지 마세요.

PowerShell을 열고 아래 두 경로를 **본인 PC의 실제 경로**로 바꿉니다.
`$RustRepo`는 기존 Python 폴더와 다른, 아직 없는 새 폴더여야 합니다.

```powershell
$ErrorActionPreference = 'Stop'
$OldRepo = 'C:\apps\codex-discord-remote'
$RustRepo = 'C:\apps\codex-discord-remote-rust'
if (-not (Test-Path -LiteralPath (Join-Path $OldRepo '.env'))) {
    throw 'The existing Python installation and its .env must be identified first.'
}
if (Test-Path -LiteralPath $RustRepo) {
    throw 'Choose a new Rust installation directory; do not overwrite an existing installation.'
}
git clone https://github.com/simdorei/codex-discord-remote-rust.git $RustRepo
if ($LASTEXITCODE -ne 0) { throw 'Git clone failed.' }
Copy-Item -LiteralPath (Join-Path $OldRepo '.env') -Destination (Join-Path $RustRepo '.env')
Set-Location -LiteralPath $RustRepo
```

새 폴더의 `.env`만 UTF-8로 편집합니다. 토큰을 채팅·Git에 붙여 넣지 마세요.

| 설정 | 전환 시 값 |
| --- | --- |
| `DISCORD_BOT_TOKEN`, 서버·사용자·채널 허용 목록 | 기존 값 유지. 새 봇을 만들 필요 없음 |
| `CODEX_HOME` | 기존 Codex 프로필 폴더 유지. 보통 사용자 폴더의 `.codex` |
| `CODEX_EXE` | 현재 PC에서 실제 존재하는 Codex 실행 파일. 오래된 경로가 명시돼 있으면 확인 |
| `CODEX_DISCORD_ROOT` | 새 Rust 설치 폴더의 절대 경로 |
| `CODEX_DISCORD_MIRROR_DB` | 새 Rust 폴더 아래 `discord_mirror.sqlite`의 절대 경로 |
| 로그·첨부파일 경로, 선택적인 MCP 설정 | 기존 값을 검토. 과거 첨부파일 경로를 없애지 않기 |

사용자·시스템 환경변수에 `CODEX_DISCORD_RUNTIME=python`이 남아 있다면
`rust`로 변경하고 PowerShell을 새로 열어 경로 변수도 다시 지정하세요.
Rust판은 `python` 선택을 자동 무시하지 않고 오류로 거절합니다.
설치 도구는 같은 프로필의 Codex 플러그인도 갱신합니다. 아래 명령 전에
Codex와 Discord 봇의 진행 중·대기 중 요청을 마치고 새 요청을 잠시 중단하세요.
`CODEX_HOME`은 이미 로그인해서 사용하던, 실제 존재하는 프로필 폴더여야 합니다.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1
if ($LASTEXITCODE -ne 0) { throw 'Installation failed; keep the existing bot unchanged.' }
.\target\release\cdr-runtime.exe --help
if ($LASTEXITCODE -ne 0) { throw 'The installed executable could not run.' }
```

이 단계는 `cdr-runtime.exe`와 Pro 보조 실행 파일을 빌드하고 Codex 플러그인을
설치합니다. 봇을 시작하거나 자동 실행 작업을 바꾸지는 않습니다.
플러그인이 바뀌므로 사용 중인 Codex 작업을 마친 뒤 Codex를 다시 열어야 합니다.
아직 `setup-discord-bot.ps1`이나 `codex-discord-bot.cmd`는 실행하지 마세요.

## 2. Python판의 작업을 마친 뒤 자동 실행과 봇 정지

진행 중·대기 중 요청과 보내지 못한 답변이 없는지 먼저 확인합니다.
전환 중에는 Discord에서 새 요청을 보내지 마세요.

Windows 작업 스케줄러에서 기존 `Codex Discord Bot` 작업의 **동작 경로가
`$OldRepo`를 가리키는지 확인한 다음 사용 안 함**으로 바꿉니다.
직접 만든 시작프로그램·추가 예약 작업이 있다면 기존 봇을 다시 켜지 않도록
함께 확인하세요. 예약 작업을 끄는 것만으로 이미 켜진 봇이 종료되지는 않습니다.

기존 봇 실행 창에서는 Ctrl+C로 종료하고, 숨김 실행 중이면 실행 경로와 PID
(프로세스 번호)를 확인해 **기존 봇만** 종료합니다. 트레이 아이콘만 닫는 것은
봇 종료가 아닙니다. 모든 `python.exe`나 Codex 앱을 일괄 강제 종료하지 마세요.
Python판이 종료됐음을 확인하기 전에는 다음 단계로 가지 마세요.

## 3. 연결 정보 백업 후 새 폴더에 복사

`discord_mirror.sqlite`에는 Codex 스레드와 Discord 대화방의 연결 정보 및
처리 기록이 있습니다. 이것을 빼면 새 봇에서 기존 연결을 이어받지 못합니다.
`.env`만 복사하는 것으로는 부족합니다.

아래 명령은 **새 실행 파일의 백업 기능으로 기존 DB를 읽습니다.** 원본을
변환하지 않고, 별도의 무결성 확인된 SQLite 백업을 만듭니다. 실제 DB 경로를
별도로 지정했던 PC는 먼저 구버전에서 사용하는 DB와 백업 대상이 같은지 확인하세요.
기본 대상은 `$OldRepo\discord_mirror.sqlite`입니다.

```powershell
$RustExe = Join-Path $RustRepo 'target\release\cdr-runtime.exe'
$newDatabase = Join-Path $RustRepo 'discord_mirror.sqlite'
if (Test-Path -LiteralPath $newDatabase) {
    throw 'A Rust database already exists; do not overwrite or merge it blindly.'
}
Push-Location -LiteralPath $OldRepo
try {
    $backupResult = & $RustExe --admin backup-store --repo-root $OldRepo
    if ($LASTEXITCODE -ne 0) { throw 'Database backup failed; do not start Rust.' }
} finally { Pop-Location }
$backupLine = [string]$backupResult
$backupPrefix = 'backup_created path='
if (-not $backupLine.StartsWith($backupPrefix)) { throw 'Unexpected backup result.' }
$snapshotPath = $backupLine.Substring($backupPrefix.Length)
Copy-Item -LiteralPath $snapshotPath -Destination $newDatabase
```

원본 폴더와 `.env`는 백업으로 남겨 두세요. 백업 폴더도 토큰·대화 기록이
포함될 수 있으므로 공개하거나 Git에 올리지 마세요.
실행 중인 DB의 `.sqlite` 본체만 복사하면 WAL(아직 본체에 반영되지 않은 변경)이
빠질 수 있으므로 위 백업 기능을 사용합니다. `-wal`·`-shm`을 새 DB 옆에 복사하지 마세요.

기존 `.codex` 프로필·대화 기록·프로젝트·첨부파일 폴더는 삭제하지 않습니다.
예전 첨부파일의 절대 경로를 대화가 참조할 수 있어, 원본 폴더를 바로 지우면 안 됩니다.
반면 구버전의 PID 잠금·heartbeat·stop/restart 표시 파일은 새 설치로 옮기지 마세요.

## 4. 설정 검사 후 자동 실행을 Rust로 전환

```powershell
Set-Location -LiteralPath $RustRepo
& $RustExe --env (Join-Path $RustRepo '.env') --check-config
if ($LASTEXITCODE -ne 0) { throw 'Configuration validation failed; do not start Rust.' }
powershell -NoProfile -ExecutionPolicy Bypass -File .\setup-discord-bot.ps1
if ($LASTEXITCODE -ne 0) { throw 'Setup failed; inspect the actual task and process state.' }
```

설정 도구는 기존 Discord 봇 토큰을 숨김 입력으로 한 번 다시 요청합니다.
새 토큰을 발급받을 필요는 없습니다. 채널 값도 기존 설정을 확인해 입력하세요.
이 명령은 `Codex Discord Bot` 예약 작업을 **새 폴더 경로로 다시 등록하고
실제로 시작합니다.** 따라서 실행 뒤 `codex-discord-bot.cmd`를 추가 실행하지 마세요.
작업 등록에 관리자 권한이 필요하면 해당 PowerShell을 관리자 권한으로 실행합니다.
설정 오류 후에는 작업이 일부 등록됐을 수 있으므로 무작정 재실행하지 마세요.

`--check-config`는 설정 형식 검사입니다. Discord 로그인·DB 전체 호환성·실제
답변 전달을 보증하는 검사는 아니므로 다음 단계가 필요합니다.

## 5. 실제 사용 확인

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\codex-discord-rust-status.ps1
```

- `bot_process`가 새 폴더의 실행 파일로 확인되고 heartbeat(작동 확인 신호)가 갱신되는지 확인합니다.
- 같은 토큰의 Python판이 다시 켜져 있지 않은지 확인합니다.
- Discord에서 `!help`, `!mirror check`로 응답과 기존 방 연결을 확인합니다.
- 기존 테스트 방에 짧은 요청을 보내 진행 표시와 최종 답변이 한 번씩 도착하는지 확인합니다.
- `!new` 다음 메시지가 새 스레드의 첫 요청이 되고 첫 답변까지 도착하는지 확인합니다.

Python DB 호환 자동 테스트는 보존된 버전 2 스키마를 검증합니다. 모든 과거
버전의 DB를 무조건 지원한다는 뜻은 아닙니다. DB 오류가 나면 오류 원문을 확인하고,
DB를 삭제해 빈 상태로 시작하거나 같은 요청을 반복 전송하지 마세요.

문제가 있으면 새 자동 실행을 끄고 Rust 봇이 종료됐는지 확인한 뒤 복구를
판단합니다. Rust로 새 요청을 이미 처리했다면 이전 DB로 바로 돌아가면 처리
기록이 갈라질 수 있으므로 두 DB를 보존하고 점검해야 합니다. 자동 Python
되돌리기는 제공하지 않습니다. 어느 시점에도 두 봇을 동시에 켜지 마세요.

## 이 안내에 대한 확인 범위 — 2026-09-12

기존 Windows PC의 별도 설치 폴더에서 Windows PowerShell로 실제 릴리스 빌드,
실행 파일 배치, 격리된 Codex 프로필의 플러그인 설치·목록 검증, 실행 파일
도움말 출력을 확인했습니다. 처음에는 검사용 프로필 폴더가 없어 등록이
거절됐고, 폴더를 준비한 뒤 이미 만든 실행 파일로 설치를 이어가 통과했습니다.
사용 중인 Codex 프로필과 운영 봇은 변경하지 않았습니다.

설치·설정 보존·덮어쓰기 방지·DB 백업 경로·기존 버전 2 DB 관련 테스트
14개가 통과했습니다. 빌드·형식 검사·Clippy·PowerShell/Git Bash 설치 예행
검사도 통과했습니다. 이번 문서 변경에서 전체 기능 테스트를 다시 돌리거나,
Python 미설치 새 PC·사용자의 실제 Python DB 전환·Discord 송수신을 새로
검증했다는 뜻은 아닙니다. 실제 전환 후에는 5번 확인이 필요합니다.
