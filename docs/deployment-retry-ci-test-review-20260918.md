# 5060 배포 재시도 중 발견한 시험 오류 보완 — 2026-09-18

## 범위와 판정

사용자가 통합본의 교체·재시작 배포 재시도를 요청했다. 운영 상태를 먼저 확인했으며 기존 PID 28860과 실행 파일·환경·배포 후보의 해시가 준비 기록과 일치했다. 지난 정비는 등록되지 않았고 활성 정비·stop·drain 마커도 없었다.

원격 main `11237db6ba04`의 Windows contract 및 macOS smoke는 재조회 시 실패로 종료돼 있었다. 공개 job API에서 실패 단계가 `Verify pinned Rust workspace`임은 확인했지만 annotation은 종료코드만 제공했다. 아래 오류는 5060에서 직접 재현한 원인이다. 원격의 비공개 상세 로그까지 확인한 것으로 표현하지 않는다.

**이번 변경은 시험 코드 2개뿐이다. 해당 보완 범위 자체 CODE PASS / 지정 회귀 및 workspace strict Clippy PASS. 제품 코드·배포 스크립트·설정·DB는 변경하지 않았다. 이 문서는 실제 운영 교체 완료 기록이 아니다.**

## T1. 도움말/슬래시 전체 옵션 시험의 잘못된 조합

`help_slash_catalog_contract.rs`는 등록된 모든 옵션을 한 요청에 채웠다. `/settings`에 `auto_reserve=true`와 model/effort/speed를 함께 보내므로 제품의 정당한 `Unsupported` 거절을 성공 기대값과 비교하고 있었다.

일반 Settings 액션 검사는 모든 수동 옵션과 ref를 유지한다. 자동 정책은 별도 시험에서 등록된 Boolean 옵션과 전체 옵션 목록을 검증하고, true/false 각각에 대해 ref 있음/없음의 4개 정상 경로를 검사한다. 세 수동 옵션의 모든 비어 있지 않은 부분집합 7개에 대해 true/false와 ref 있음/없음을 조합한 28개 혼합 입력이 정확한 Unsupported 오류로 거절되는지도 검사한다. 자동 옵션을 그냥 제외하거나 제품 거절 조건을 삭제하지 않았다.

재현 증거: 이전 QA 폴더 `integration-deploy-20260918/retry-runtime-046-090.stdout.log`. 해당 대상은 수정 전 0 통과/1 실패였다.

## T2. 별도 PowerShell 시험 호스트의 출력 인코딩

`install_plugin_native_contract`를 콘솔 없는 QA 프로세스에서 실행하자 한글 경고 및 경로가 CP949 바이트로 출력돼 UTF-8로 읽는 시험 두 개가 실패했다. 설치 성공 여부만 검사해서 통과시키지 않았다.

공통 fixture가 테스트용 PowerShell 호스트를 만들도록 수정했다. 호스트는 제품 install.ps1의 실제 전체 parameter block을 그대로 사용하고, UTF-8 stdout 및 OutputEncoding을 설정한 뒤 **수정하지 않은 제품 install.ps1**을 `@PSBoundParameters`로 호출한다. 명시/생략 파라미터의 의미, 경로의 공백·한글·특수문자, 제품 PSScriptRoot, 오류 전파를 보존한다. 자동/수동 설정이나 실환경 설치 코드는 변경하지 않았다.

재현 증거: `integration-deploy-retry-20260918/runtime-086-140.stdout.log`. native installer 대상은 4 통과/2 실패였다. 같은 큰 그룹은 전체 실행 시간 제한에도 도달했으므로 부분 결과를 전체 통과로 집계하지 않는다.

## 최종 실행 증거

증거 기준 경로:

```text
C:\repos\simdorei\codex-discord-remote-rust\target\qa\integration-deploy-retry-20260918\final\
```

- `installer-native-final.json`: native installer 6 통과/0 실패/0 제외. 단계별 CLI 실패, 잘못된 inventory, 큰 양쪽 오류 스트림, 한글/특수문자 경로와 shim, dry-run 검사를 포함한다.
- `changed-test-boundaries-final.json`: 도움말·설정·설치 프로필·래퍼·UTF-8 등의 10개 target에서 31 통과/0 실패/0 제외.
- `strict-clippy-final.json`: `cargo clippy --offline --locked --workspace --all-targets -- -D warnings`, exit 0.
- 세 검사 모두 소스 1,673개의 실행 전후 SHA가 같으며 최종 source-baseline과 일치한다.

위 37개 시험은 중복 없는 두 실행의 합계다. 전체 workspace의 모든 시험을 이번 기록으로 통과했다고 주장하지 않는다. 선행 진단 실패·timeout 기록은 삭제하지 않았다. 경고 억제, ignore 추가, 기존 실패 기대값 삭제는 하지 않았다.

## 자체 리뷰

제품의 auto_reserve 혼합 거절을 보존하면서 명령 카탈로그의 모든 옵션을 실제 route_command → plan_slash 경로로 검증하는지 확인했다. false 값을 빠뜨리지 않았으며 ref 유무를 모두 검사한다. fixture 호스트는 제품의 파라미터 선언을 복사하므로 새 파라미터를 수동 목록에서 누락시키지 않는다. 주입되는 것은 테스트 호스트 인코딩뿐이고 제품 스크립트의 내용·종료 조건을 바꾸지 않는다. 기존 native 설치 실패 시험도 동일하게 실패 상태와 진단 보존을 검증한 뒤 통과했다.

최종 커밋 발행, release 후보와 소스의 연결, 실제 정비 등록·백업·drain·교체·재시작 결과는 별도 배포 재시도 기록에서 확인해야 한다. 원격 CI의 새 실행은 실제 결과가 나오기 전까지 pending이다.
