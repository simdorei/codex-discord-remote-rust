# PR28 후속 브라우저 호스트 진단 diff — 독립 코드 검수

- 검수일: 2026-11-16 KST
- 대상 PC: `sim-pc-5060`
- 작업 폴더: `C:\repos\simdorei\cdr-browser-host-20261116`
- 작성자 보고서: `C:\repos\simdorei\cdr-browser-host-20261116\docs\pr28-browser-host-diff-20261116.md`
- 기준: `196b7d8`
- 검수 대상: 작성자 보고서에 포함된 전체 diff 및 같은 검증 폴더의 `final.patch`
- 크기: 39,596 bytes, 13 files, 712 insertions / 34 deletions

## 판정

**CODE PASS — 이번 브라우저 호스트 진단 추가 범위에 한정한다. 필수 수정으로 판정할 구체적인 신규 결함은 발견하지 못했다.**

| 판정 대상 | 결과 |
|---|---|
| 현재 diff의 진단 코드 | CODE PASS |
| 전체 workspace 시험 | FAIL: 3326 passed / 20 failed / 38 ignored |
| 실제 Chrome 연결 구현 | 미완료 |
| GUI 완전 종료 후 새 독립 실행 | 미완료·미검증 |
| 운영 배포·전체 릴리스 게이트 | 이번 검수로 승인하지 않음 |

이번 변경은 연결 정보가 어디까지 전달됐는지, 공식 도구가 어떤 알려진 오류를 보고했는지를 관측하는 코드다. 공식 호스트 제공·현재 봇 대화 등록·정보 갱신 경로를 구현한 변경은 아니다. 따라서 **실제 사용자 목표에는 코드 구현과 실사용 검증이 모두 남아 있다. 'QA만 남았다'는 판정은 맞지 않는다.**

## 검수 대상 일치 확인

5060 원격 파일의 내용과 Git 상태를 읽어 다음을 확인했다.

1. 현재 작업 트리의 HEAD는 기준 `196b7d8`과 일치한다.
2. `git diff --binary HEAD`와 보관된 `final.patch`가 바이트 단위로 일치한다.
3. formatting / clippy / build / workspace-tests에 보관된 네 patch도 모두 final.patch와 바이트 단위로 같다.
4. 네 실행 증거 JSON의 기준 소스가 현재 HEAD와 일치한다.
5. 검수 말미에도 현재 diff와 final.patch의 일치가 유지됐다.

원격 Git 상태에 있는 작성자 보고서는 추적되지 않은 문서이며, 코드 diff 13개 파일과 구분했다. 이 검수에서는 기존 운영 프로세스, DB, 설정, 브라우저 및 소스를 수정하거나 새 시험을 실행하지 않았다. 검수 결과 문서만 추가했다.

## 코드에서 확인한 사항

### 1. 기존 Windows 환경 전달 정책을 보존한다

`process.rs` 및 `process/browser_host.rs`의 기존 전달 알고리즘은 유지된다.

- 명시적 host override가 있으면 세 필드 전체의 출처를 그 override로 선택한다.
- 완전한 세 필드를 한 출처에서 전달하며, 다른 출처의 pipe/key/executor를 섞지 않는다.
- 부분 설정과 대소문자 별칭 충돌의 기존 보류·executor guard 동작을 보존한다.
- 추가된 것은 inherited / overrides / 최종 child environment의 상태 메타데이터다.
- 부모 process 환경을 다시 쓰는 코드가 추가되지 않았다.
- 비-Windows 실행 경로는 원래 환경 조립을 유지하며 launch 관측을 None으로 남긴다.

### 2. 요청 기록을 실제 쓰기 완료 뒤에 남긴다

`client.rs`의 기록은 기존 `write_complete()` 뒤에만 추가된다. 대상은 `thread/start`, `thread/resume`, `thread/fork`이다.

기존 요청 본문이나 model, effort, sandbox, approval, cwd, MCP 설정을 변경하지 않는다. 기록은 '요청이 쓰였다'는 사실이며 서버가 해당 thread 연산에 성공했다는 판정으로 사용되지 않는다.

### 3. 진단의 의미가 보수적이다

`browser_host_diagnostics.rs`는 `UNKNOWN / ABSENT / EMPTY / NONEMPTY / AMBIGUOUS`를 구분한다.

- 실제 관측하지 않은 worker environment는 UNKNOWN으로 유지한다.
- thread override에 값이 없는 사실을 worker 환경의 누락으로 바꾸지 않는다.
- 같은 필드의 nested/dotted/case 별칭 중복을 임의 우선순위로 해석하지 않는다.
- 일반 문자열이나 성공 출력에서 host 접근 성공/실패를 추정하지 않는다.
- 도구 목록의 inventory VERIFIED를 브라우저 접근 성공으로 확대하지 않는다.

### 4. 알려진 오류만 제한적으로 분류한다

`item/completed` 중 `mcpToolCall`, `status=failed`, `node_repl` 또는 `cua_repl`의 `js` 도구, `error.message`라는 조건이 모두 맞아야 분류한다. 메시지 첫 줄의 알려진 문구를 정확히 비교한다.

일반 페이지 내용·성공한 도구 출력·다른 도구의 오류는 이 경로에서 브라우저 상태를 만들지 않는다. 감싸진 오류나 결과 본문에만 실린 오류를 놓칠 수 있는 제한은 문서에 명시되어 있다. 인식하지 못한 오류를 성공으로 바꾸지도 않는다.

### 5. 비밀값과 저장량을 제한한다

새 진단은 raw host 값, key, 오류 본문, URL, 실제 thread/turn ID를 보관하지 않는다. 고정된 상태 enum·오류 단계·ID 존재 여부와 로컬 요청 식별자만 남긴다.

thread request 및 tool failure 이력은 각각 최근 16건으로 제한한다. 최근 요청 목록을 특정 오류의 정확한 소유 thread/turn 증거라고 주장하지 않는다. 실제 턴의 상관관계는 별도 통제된 실행 증거가 필요하다.

### 6. 원래 notification과 RPC 경로를 유지한다

`transport.rs`는 원래 state 기록과 notification broadcast를 유지한다. 새 진단에 오류가 분류되지 않아도 원래 notification은 전달된다.

state 임시 lock, diagnostics lock, lifecycle lock의 해제 경계를 확인했다. 실제 stderr 출력은 lifecycle/state/diagnostics lock 밖에서 수행하며, 출력 오류를 transport close로 바꾸는 분기는 추가되지 않았다.

로그 출력 자체는 동기식 stderr 쓰기다. 출력 대상이 장시간 막히면 같은 stdout 수신 task의 후속 처리가 지연될 수 있다는 일반적인 한계는 남는다. 현재 확인된 관리형 실행은 stderr 파일 리디렉션이고 관련 fixture는 pipe를 읽으므로, 이번 검수에서 이를 실제 장애나 필수 수정 결함으로 확정하지 않았다.

### 7. 진단 결과를 기능 완료로 보고하지 않는다

`admin/codex_tools.rs`는 기존 inventory 보고서에 관측만 추가한다. `access_check`와 `browser_discovery`는 NOT_CHECKED이며 추가 모델 턴이나 host RPC를 호출하지 않는다.

작성자 보고서와 `headless-computer-use.md`도 정상 호스트 제공·대화 등록 계약, 실제 worker 상태, GUI 독립 브라우저 동작을 미완료로 명시한다.

## 실행 증거 검수

아래는 작성자가 이미 실행한 시험의 저장 로그를 이번 검수에서 다시 읽고 집계한 결과다. 이번 검수에서 시험을 새로 실행한 결과가 아니다.

| 항목 | 로그와 patch 대조 결과 |
|---|---|
| Formatting | exit 0, 같은 최종 patch |
| Clippy | exit 0, 같은 최종 patch |
| Workspace build | exit 0, 같은 최종 patch |
| 새 진단 모듈 단위 시험 | 9개 모두 ok |
| Windows host environment 회귀 | 9 passed / 0 failed / 1 ignored |
| 전체 workspace | 3326 passed / 20 failed / 38 ignored, exit 101 |

Windows 환경 시험의 ignored 1건은 별도 자식으로 실행하는 `environment_driver` fixture이다. 바깥의 실제 환경별 시험 9개가 해당 격리 fixture를 실행한다.

전체 로그 570개 test-result 절을 직접 합산한 값이 작성자의 총계와 일치했다.

### 전체 시험의 20건 실패

| 분류 | 건수 | 확인한 근거와 한계 |
|---|---:|---|
| ETW 접근 거부 | 2 | StartTraceW status=5, 기준 코드 probe에서도 재현한 로그 존재 |
| WMI 접근 거부 | 4 | Access denied, 기준 코드 capture에서도 재현한 로그 존재 |
| 교차 프로세스 조회 실패 | 1 | OpenProcess query/synchronize 실패 기록. 해당 기준 사례 전체 재실행 증거는 없음 |
| PowerShell 7 실행 파일 없음 | 13 | pwsh.exe program not found 계열 실패 |

관련 기존 PowerShell/C# 입력 14개가 기준과 동일하다는 비교 증거와 ETW/WMI baseline 로그를 확인했다. 이 변경의 신규 기능 회귀로 단정할 증거는 없지만, 전체 기준 시험을 모두 다시 실행한 것도 아니다. 따라서 **20건을 임의로 통과 처리하거나 전체 게이트 PASS로 확대하지 않는다.**

### 실제 설치 CLI를 사용한 inventory

별도 보관된 candidate의 inventory는 VERIFIED였으나, 실제 브라우저 접근과 발견은 NOT_CHECKED였다. child launch의 host 세 필드는 ABSENT였고 worker는 UNKNOWN이었다. 이는 해당 격리 진단 프로세스의 관측이며 운영 봇 전체나 실제 Chrome의 성공 증거가 아니다.

개발 candidate의 내장 source identity는 unknown이다. 작성자는 이를 숨기지 않고 기준 소스·patch·빌드 명령·실행 파일 해시의 별도 증거로 식별하고 있다. 이번 결과를 그대로 운영 교체 후보의 provenance 승인으로 확대하지 않는다.

## 사용자 목표에 대해 남은 일

1. 공식 호스트가 연결 정보를 제공하는 지원 경로와 현재 봇 대화의 등록·허가·갱신 계약을 확정한다.
2. 확인된 계약에 맞춰 필요한 전달·등록 코드를 구현한다. 현재 diff에는 해당 구현이 없다.
3. 실제 모델 턴에서 Chrome 읽기·클릭·입력 결과를 검증한다.
4. 앱 실행 중 / 창만 닫음 / GUI 및 소유 helper 완전 종료 / 새 app-server·worker 시작을 나눠 검증한다. 독립 실행 완료에는 계획의 L3와 L4가 필요하다.
5. 전체 시험의 환경 제약을 갖춘 검증 환경에서 실패 게이트를 정리한다.

또한 이번 diff는 app-server transport가 닫힌 상태의 자동 복구, heartbeat 기반 watchdog의 보완, SQLite hot-journal 재시작 복구 순서를 변경하지 않는다. 별도로 기록된 5060 Discord 먹통 사고의 재발 방지 수정 여부는 이 CODE PASS에 포함되지 않는다.

## 검수 결론

진단 추가 패치로는 수용 가능한 코드다. 공식 Chrome 제어 및 앱 완전 종료 후 독립 실행이라는 최종 기능은 계속 미완료이며, 전체 시험과 실사용 검증까지 통과한 상태도 아니다.
