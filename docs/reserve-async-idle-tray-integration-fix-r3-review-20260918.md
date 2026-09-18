# Reserve · Async question · Idle release · Tray 합본 수정 및 자체 검수 R3

- 날짜: 2026-09-18 KST
- 판정: **CODE PASS — R1/R2 수정 반영 후보의 merge용 코드 검수 통과**
- 검수 성격: 같은 작업자가 수행한 수정 후 자체 검수. 독립 검수나 운영 배포 승인으로 표시하지 않는다.
- PC: `sim-pc-5060`
- 작업 폴더: `C:\repos\simdorei\codex-discord-remote-rust\target\qa-source\reserve-async-idle-tray-integration-20260918`
- 브랜치: `integration/reserve-async-idle-tray-20260918`
- 기준 HEAD: `a5b291d`, 부모 `8b68180` / `33eafd4`
- 상태: 제품 코드와 시험을 worktree에 반영했다. commit, push, main merge, 배포, 운영 봇/트레이 재시작은 하지 않았다.
- 증거: 아래 QA 폴더의 실행별 check JSON, stdout/stderr, 실행 전후 SHA-256 inventory를 사용한다. 별도 통합 manifest JSON은 생성 미완료이며 존재한다고 주장하지 않는다.

## 1. 결론과 범위

이전 검수의 R1(상속된 Goal의 현재 질문을 과거 실행 세대로 검사하는 문제)과 R2(확실히 미전송 종료된 만료 기록이 유휴 해제를 영구 차단하는 문제)를 수정했다. 새 반례 19개를 추가하고 최종 소스로 Store 전체, App-server 전체, Runtime 전체 라이브러리 및 관련 integration 17개 타깃을 실제 실행했다. 해당 범위에서 남은 merge 차단 결함을 찾지 못했다.

이 판정은 지정된 합본 worktree의 변경 파일과 관련 호출 경로에 대한 판단이다. 전체 워크스페이스의 모든 Cargo 시험을 이번에 실행한 것은 아니다. 실제 Discord 서비스·Desktop UI·운영 트레이 아이콘 표시도 이번 검수 대상이 아니다. 로컬 native app-server fixture와 loopback HTTP fixture를 사용했다.

## 2. R1 수정 — 원래 실행 출처와 현재 질문의 제어 신원 분리

### 원래 job의 출처 보존

`app_server_generation`, `execution_generation`, attempt count, 원래 입력은 현재 질문의 generation으로 덮어쓰지 않는다. 실제로 붙은 turn의 `turn_observation_generation`은 현재 질문의 정확한 turn/generation 검증에만 사용한다. NULL인 기존 관측 값은 원래 세대와의 일치만 인정하며 현재 세대를 임의로 대입하지 않는다.

질문 후보 수집은 대상 스레드의 모든 non-pending job을 조사한다. 다른 세대의 중복 owner가 필터에 가려지지 않는다. 유일한 후보의 job·방·사용자·원래 generation·execution generation·attempt count를 고정한다. 이 단계는 데이터 보관일 뿐 UI·전송 권한을 만들지 않는다.

### handoff보다 빠른 질문 관측

inbox에 nullable INTEGER 열 `candidate_generation`, `candidate_execution_generation`, `candidate_attempt_count`를 추가했다.

승격은 같은 SQLite 트랜잭션에서 candidate job, thread, turn, 방, 사용자, 원래 실행 출처, 현재 관측 세대, Running 상태, goal_waiting=false, 유일한 활성 owner를 모두 검사한다. 질문 이벤트가 Goal handoff보다 먼저 도착하면 inbox에만 남고 정확한 handoff 이후 한 번만 승격된다.

기존 DB 마이그레이션은 세 열을 NULL로 추가할 뿐 과거 출처를 만들어 넣지 않는다. 기존 동일 세대 후보의 좁은 호환 경로는 유지한다. 출처를 입증할 수 없는 기존 cross-generation 후보는 자동 승격하지 않는다.

### claim부터 실제 전송·ACK 저장까지 동일 신원 검사

새 `async_question/ownership.rs`의 공통 검사로 정확한 job/turn/방/사용자와 현재 관측 세대를 확인한다. Steer claim은 유일한 현재 owner만 허용한다. 실제 write guard와 확인·거절 처리에는 기존 전체 job/question/policy seal을 유지한다.

활성 질문 응답은 해당 turn의 Steer로만 전송한다. Start용으로 새로 예약한 quarantine job은 여전히 질문의 현재 generation과 일치해야 한다. 오래된 runtime/generation, 다른 사용자·방, 달라진 turn, 중복 owner는 거절한다. 미확정 결과는 재실행 가능 상태로 돌아가지 않는다.

### Final이 질문 승격을 앞서는 경계

세대가 없는 delivery outbox 행만으로 질문 소유권을 새로 인정하던 fallback을 제거했다. 후보 수집 범위를 넓힌 뒤 이 fallback을 유지하면 오래된 질문이 같은 job/turn의 Final만으로 잘못 승격될 수 있기 때문이다.

대신 `delivery.rs`의 완료 트랜잭션에서 정확한 job을 삭제하기 전에 `reconcile_job_in`으로 질문 소유권을 확정한다. 질문 확정·Final outbox 생성·job 제거는 같은 트랜잭션에 속한다. 질문 저장 실패를 주입하면 Final과 job 삭제도 rollback되고 inbox는 남는다. 이미 확정된 질문은 완료 뒤에도 기존 소유권을 유지한다.

## 3. R2 수정 — 확실히 미전송 종료된 기록만 idle 차단에서 제외

`idle_release::bot_idle_on`에서 다음을 모두 만족하는 질문만 종료된 미전송 기록으로 인정한다.

```text
state = expired
chosen IS NULL
dispatch_mode IS NULL
reply_job_id IS NULL
accepted_turn_id IS NULL
preparation_json IS NULL
```

질문 ID와 tombstone은 삭제하지 않는다. expired라는 상태값만 있고 dispatch 흔적이 남은 기록은 계속 차단한다. inbox는 자체적으로 답변을 dispatch하지 않으므로 정확히 expired인 행만 제외한다.

observed, open, unsupported, unknown, dispatching, 알 수 없는 inbox 상태, 남은 큐 작업, Reserve pending fence, dead-generation/archive/cleanup fence는 계속 차단한다. 관측 gap이나 안전 검사를 제거하지 않았다. retention/supersede의 만료 범위를 일괄 확대하지 않았다.

## 4. 새 반례 19개

| 파일 | 추가 시험 | 주요 확인 |
|---|---:|---|
| `crates/cdr-store/tests/async_inherited_owner.rs` | 10 | G1→G2 질문 처리 전체 경로, handoff 순서, 원래 출처 변경, 중복 owner, 오래된 질문, Final 경합, rollback, claim 후 신원 변경 |
| `crates/cdr-store/tests/async_candidate_migration.rs` | 2 | legacy NULL 보존·새 권한 미생성, 잘못된 관측 신원 거절 |
| `crates/cdr-store/tests/async_expired_idle.rs` | 5 | 정상 만료 후 idle, inbox 만료, dispatch 흔적·Unknown 보호, 다음 Final의 Candidate 생성 |
| `crates/cdr-runtime/src/component_worker/async_inherited_tests.rs` | 2 | 실제 native 알림·HTTP 버튼·Steer 1회, 중복 클릭, 다른 사용자/방, 동일 숫자 세대의 새 resident 거절 |
| **합계** | **19** | 기존 시험의 성공 조건과 제외 항목을 약화하지 않음 |

Native 양성 시험은 원래 job generation=0/execution=0, 현재 resident generation=1/turn observation=1의 상속 상태로 알림이 handoff 전후에 도착하는 두 경로를 확인했다. 두 질문의 버튼이 표시되고 선택한 질문만 정확한 turn에 Steer 1회 전송된다. 중복 클릭은 확인 표시만 복구하며 다른 질문은 open으로 남는다. Start, resume, fork, settings/update, rate-limit 설정 준비 호출은 발생하지 않는다.

## 5. 최종 실제 실행 결과

아래는 최종 소스에 대한 겹치지 않는 범위이다. 초기 시도와 집중 시험은 중복 집계하지 않았다.

| 범위 | 통과 | 실패 | 기존 제외 |
|---|---:|---:|---:|
| `cdr-store` 전체 | 305 | 0 | 2 |
| `cdr-app-server` 전체 | 136 | 0 | 10 |
| `cdr-runtime` 전체 라이브러리 | 453 | 0 | 3 |
| Runtime integration 17개 타깃 | 125 | 0 | 0 |
| **합계** | **1,019** | **0** | **15** |

Runtime integration 125개에는 트레이 3개 타깃의 29개와 mirror sync 48개가 포함된다. 신규 반례 19개도 위 합계에 포함된다.

### 최종 실행 기록

공통 증거 폴더: `target/qa/integration-fix-r3-20260918/`

| 실행 이름 | 범위 | 결과 |
|---|---|---|
| `store-app-server-current` | `cargo test --offline --locked -p cdr-store -p cdr-app-server --no-fail-fast -- --test-threads=4` | 441 pass / 0 fail / 12 ignored |
| `runtime-merge-current` | Runtime lib + 아래 17개 integration 타깃 | 578 pass / 0 fail / 3 ignored |
| `clippy-workspace-r2` | `cargo clippy --offline --locked --workspace --all-targets -- -D warnings` | 종료 코드 0 |
| `native-build-current` | `cargo build --offline --locked -p cdr-runtime --bins` | 종료 코드 0, 후보의 debug 빌드만 수행 |
| `format-workspace.log` | 기존 `scripts/Test-RustFormatting.ps1` | 11 packages / 458 Cargo target roots / 62 batches 통과 |
| `git diff --check` | 변경 파일 공백 검사 | 종료 코드 0 |

17개 타깃: integrated_completion_contract, luna_reserve_auto_switch_contract, luna_reserve_settings_contract, control_send_boundary_contract, control_turn_contract, queue_generation_submission_contract, queue_recovery_contract, queue_retry_policy_contract, session_mirror_contract, session_mirror_dedup_contract, session_mirror_starting_contract, mirror_readonly_contract, mirror_sync_contract, new_first_turn_contract, windows_tray_contract, windows_tray_reliability_contract, windows_tray_revision3_contract.

각 최종 Cargo 실행의 `.check.json`, `.stdout.log`, `.stderr.log`, `.before.json`, `.after.json`이 증거다. 각 check는 종료 코드 0과 source_unchanged=true를 기록한다.

Inventory 범위는 Git tracked/untracked non-ignored 파일 중 .rs/.toml/.lock/.ps1/.psm1/.vbs/.cmd/.sh이며 docs를 제외한다. 이 범위 **1,599개 파일**의 SHA-256을 각 실행 전후에 비교했다. 최신 native-build-current의 before/after도 동일 범위의 최종 스냅샷이다. 모든 종류의 저장소 파일을 포괄하는 inventory라고 주장하지 않는다.

## 6. 중간 실패 및 증거 패키징 제한

- 수정 전 R2 반례는 `red.log`에서 2 pass / 3 fail로 재현했다. R1만 따로 실행하려던 수정 전 요청은 보안 상태 판정에서 차단돼 실행 증거로 집계하지 않았다. R1의 최종 Store/native 시험은 실제 실행해 통과했다.
- 초기 native 시험은 cmd 기반 환경에서 `environment variable name contains '='`로 fixture 기동 전에 실패했다. 제품 조건은 바꾸지 않고 QA 실행기를 직접 ProcessStartInfo 방식으로 보정했다. 사용자·시스템·운영 환경 설정은 수정하지 않았다.
- 첫 strict Clippy는 새 문서 주석의 SQLite backtick 누락 한 건으로 실패했다. 주석 수정 후 전체 workspace/all-targets strict 검사와 최종 시험이 통과했다.
- 단일 `cargo fmt --all -- --check`는 Windows error 206으로 실행되지 못했다. 기존 저장소 스크립트가 모든 Cargo 대상·edition을 유지하며 인자만 8개씩 나누는 동일 rustfmt 검사를 수행했고 전체 통과했다.
- 마지막 통합 manifest 생성 스크립트는 빈 stdout 집계 처리에서 실패했다. 후속 보완·실행 요청은 도구의 보안 상태 판정에서 차단되어 반복하거나 우회하지 않았다. 따라서 **`reserve-async-idle-tray-integration-fix-r3-manifest-20260918.json`은 생성 완료 파일로 제공하지 않는다.** 이는 이미 완료된 코드 시험을 실패로 바꾸는 것은 아니지만, 별도 통합 manifest의 완성·검증을 주장하지 않는다. 실제 인계 증거는 위 원본 실행별 로그와 SHA 스냅샷이다.

## 7. 변경 파일 14개

제품 코드 9개:

```text
crates/cdr-store/src/async_question.rs
crates/cdr-store/src/async_question/dispatch.rs
crates/cdr-store/src/async_question/guard.rs
crates/cdr-store/src/async_question/inbox.rs
crates/cdr-store/src/async_question/observe.rs
crates/cdr-store/src/async_question/schema.rs
crates/cdr-store/src/async_question/ownership.rs          # 신규
crates/cdr-store/src/delivery.rs
crates/cdr-store/src/idle_release.rs
```

시험 파일 5개:

```text
crates/cdr-store/tests/async_inherited_owner.rs            # 신규
crates/cdr-store/tests/async_expired_idle.rs               # 신규
crates/cdr-store/tests/async_candidate_migration.rs        # 신규
crates/cdr-runtime/src/component_worker/async_inherited_tests.rs  # 신규
crates/cdr-runtime/src/component_worker/async_choice_tests.rs    # 모듈 연결 추가
```

트레이 PowerShell/VBS/CMD 제품 파일과 Reserve 제품 코드는 변경하지 않았다. 원래 실행 출처 보존, 미확정 답변 재전송 금지, usage fence, cleanup 보호, 실제 resident/runtime 검사를 유지했다.

## 8. 인계

PASS는 위 신규 untracked 파일까지 포함한 미커밋 수정 후보 전체를 대상으로 한다. 기준 HEAD만 병합하거나 기존 파일만 선택하면 수정본이 아니다.

마이그레이션으로 입증 불가능한 옛 cross-generation 후보는 보류한다. 과거 Final만으로 현재 resident의 유휴 해제 권한을 부여하지 않는다.

실제 release 빌드·운영 교체·재시작·실서비스 메시지·트레이 아이콘 실표시 검수는 하지 않았다. 운영 저장소 루트가 아니라 지정 통합 worktree만 수정했다. **main merge, commit/push, 배포는 수행하지 않았다.**
