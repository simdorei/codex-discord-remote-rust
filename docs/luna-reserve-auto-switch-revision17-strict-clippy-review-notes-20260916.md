# Luna Reserve revision17 — strict Clippy 실패 해소 및 회귀 검수

- 요청일: 2026-09-16 (Asia/Seoul). 파일명의 날짜는 요청일 기준이다.
- 최종 검증 기준: 2026-09-16 23:58:48 KST / 2026-09-16T14:58:48Z.
- 대상: `sim-pc-5060`, `C:\repos\simdorei\codex-discord-remote-rust`.
- 요청: 남아 있는 실패를 기존 경고로 넘기지 않고, 다시 계획하여 수정한다.
- 판정: **CODE PASS / package 및 workspace strict Clippy PASS / 지정 오프라인 회귀 PASS**.
- 기존 strict Clippy 실패를 이유로 한 보류 사유는 해소됐다. 운영 적용은 하지 않았다.
- 이 문서는 수정 참여자의 직접 구현·실행·자체 검수 기록이다. 별도 검수자의 독립 승인이나 실운영 환경 전체 인증으로 표현하지 않는다.

## 1. 계획과 실제 수행 범위

먼저 `docs/luna-reserve-auto-switch-revision15-1-entry-validation-review-notes-20260916.md`와 실제 Clippy 로그, 해당 제품·시험 소스를 읽었다. 구현 전 계획은 다음 파일에 저장했다.

`docs/luna-reserve-auto-switch-revision17-strict-clippy-plan-20260916.md`

수정 전 소스 1,303개를 SHA-256으로 기록하고, 변경할 기존 파일 13개의 원본을 별도로 보존했다. 현재 strict 검사를 실제 재현하여 cargo exit 101과 Store의 5개 진단 위치를 확인했다. 일반 Clippy 로그에는 Runtime·시험·fixture 경고도 있었으므로 Store만 고치고 종료하지 않았다.

이 작업에서 **기존 Rust 파일 13개를 수정하고 새 시험 파일 1개를 추가**했다. 최종 1,304개 소스를 시작 시점과 비교한 변경 경로는 이 14개뿐이다. 기존 dirty 변경을 초기화하거나 다른 작성 흐름의 변경을 덮어쓰지 않았다. Cargo.toml, Cargo.lock, lint 정책, 의존성 버전은 변경하지 않았다.

## 2. 수정 사항과 동작 보존

| 위치 | 수정 | 직접 대조한 보호 경계 |
|---|---|---|
| `crates/cdr-store/src/reserve_policy/usage_fence.rs` | 항상 성공하는 빈 `migrate()` 및 항상 true인 `schema_current()` 제거. auto/on 및 manual/off의 중첩 or-pattern 정리 | 기존 claim, fence, SQL, unknown/missing mode 거절은 그대로 유지 |
| `crates/cdr-store/src/reserve_policy.rs` | 제거한 no-op 호출 정리. 같은 INTEGER 기본값을 갖는 match arm 병합 | 부모의 21개 column·recovery index 검사와 두 notice schema 검사 유지. 실제 마이그레이션 오류 전파 유지 |
| `crates/cdr-runtime/src/completion_worker.rs` | 긴 `finish_with_owner()`의 최종 전송 부분을 `finish_owned_terminal()`로 추출 | 같은 캡처 owner를 전달. completion staging → delivery await → running 상태 확인 → journal 정리 순서 유지. progress 분기는 기존 위치에서 반환 |
| `crates/cdr-runtime/src/completion_worker/recovery.rs` | 문서 주석의 `goal_waiting` 식별자에 backtick 추가 | 실행 코드 변경 없음 |
| `crates/cdr-runtime/tests/fixtures/app_server/reserve_auto.rs` | 긴 run closure를 상태를 소유하는 Fixture와 RPC별 메서드로 분리 | gate, settings 적용, 영속화, notification, reply 순서 및 fault injection 보존 |
| `crates/cdr-runtime/src/completion_worker/reserve_inheritance_tests.rs` | 중복 HTTP 모듈 대신 기존 shared test-support 모듈 alias 사용 | 각 시험이 별도 loopback listener와 상태를 생성하는 기존 fixture 동작 유지 |
| `crates/cdr-runtime/src/reserve_auto/native_tests/notices.rs` | 같은 HTTP 모듈 중복 로딩 제거 | 실제 HTTP·receipt·remap assertion 유지 |
| `crates/cdr-runtime/src/reserve_auto/native_tests/restart_notice.rs` | 같은 HTTP 모듈 중복 로딩 제거 | 재시작 이후 원래 요청 재전송 금지 및 notice custody 시험 유지 |
| `crates/cdr-store/tests/goal_inheritance_contract.rs` | singleton 비교의 불필요한 clone을 slice reference로 교체 | owner 전체 동등 비교와 모든 assertion 유지 |
| `crates/cdr-store/tests/goal_progress_owned_contract.rs` | 같은 clone 정리 | stale owner 및 journal/progress 보존 assertion 유지 |
| `crates/cdr-runtime/src/completion_worker/goal_inheritance_guard_tests.rs` | 같은 clone 정리 | 재시작 및 미부착 turn 보호 assertion 유지 |
| `crates/cdr-runtime/src/completion_worker/goal_inheritance_regression_tests.rs` | 같은 clone 정리 | 대기 owner 전체 비교와 기존 오류 설명 유지 |
| `crates/cdr-runtime/src/completion_worker/goal_mirror_boundary_tests.rs` | 중복 Goal fixture 요청을 helper로 추출 | early/late 두 순서, timeout, observer-before-processor, 실제 HTTP gate와 모든 assertion 유지 |
| `crates/cdr-store/tests/reserve_schema_lint_contract.rs` | 새 회귀시험 5개 | no-op 제거가 실제 schema 검사를 약화시키지 않는지 확인 |

경고 억제용 allow/expect 추가, lint 등급 완화, 검사 target 삭제, 기존 시험 제외 또는 기대값 완화는 하지 않았다.

### Fixture 분리에서 확인한 순서

- `complete`: 기존 turn 갱신 → persist → 선택적 final-answer event → turn/completed → 응답.
- `update_settings`: active 작업 거절 → 설정 적용 → after_settings 변경 → 선택적 settings event → persist → 응답.
- `start_turn`: Goal/거절 조건 → sequence 및 시작 설정 기록 → turn 저장 → persist → turn/started → 응답.
- `rate_limits`: 현재 응답을 먼저 구성한 뒤 after_rates를 다음 응답에만 반영. 일반 한도 필드 생략/null 시험 옵션 유지.
- 재시작용 저장에는 settings/turns/sequence만 들어가며, 계정·quota 옵션을 과거 관측으로 복원하지 않는다.

## 3. R15-1 및 기존 계약 유지

Reserve의 실제 모델 관측·진입 검증·effort 선택을 담당하는 기존 제품 파일은 이번 작업에서 변경하지 않았다. 해당 소스가 baseline과 동일한 것도 SHA 비교로 확인했다. 기존 native 회귀시험을 현행 fixture와 함께 다시 실행했다.

따라서 다음 보호 계약을 제거하거나 느슨하게 만들어 strict 검사를 통과시킨 것이 아니다: 실제 Reserve의 ordinary 정책 우회 방지, 기본 effort → high → medium, medium 미만 거절, 적용 후 재검증, exact usage-fence 보존, 실제 ordinary/manual/off 예외, 이전 대화·Goal 상속, 실패했던 원래 입력의 자동 재실행 금지, Final/Failed 및 Queue/Mirror 소유권.

## 4. 추가한 schema 회귀시험 5개

`crates/cdr-store/tests/reserve_schema_lint_contract.rs`:

| 시험 | 확인 내용 |
|---|---|
| `every_usage_fence_column_is_still_required_and_repaired` | 6개 usage-fence column을 각각 제거하면 schema_current=false. 마이그레이션 후 복구 |
| `repeated_migration_preserves_pending_fence_and_schema_version` | 반복 마이그레이션이 pending claim의 정확한 ID·실패 revision·현재 revision·reason과 schema_version을 변경하지 않음 |
| `notice_tables_and_indexes_remain_required` | notice table 및 index 누락을 검출하고 복구 |
| `merged_integer_column_definition_preserves_defaults_and_affinity` | 병합한 두 column의 기본값 0과 INTEGER affinity 유지 |
| `migration_still_propagates_database_write_errors` | read-only 성격의 query_only DB에서 마이그레이션 실패를 반환. 쓰기 허용 후 정상 복구 |

모두 메모리 내 SQLite를 사용한다. 이번 RED→GREEN의 대상은 strict Clippy이며, 이 5개 시험을 수정 전 기능 실패 재현으로 주장하지 않는다.

## 5. 실제 최종 검증 결과

실행 PC는 5060이며, cargo/rustc 1.97.1과 해당 Clippy로 확인했다. 독립 빌드 디렉터리는 `target-revision17-strict`다.

| 검사 | 결과 |
|---|---:|
| Store + Runtime all-targets strict Clippy | **exit 0 / warning 0 / error 0** |
| 전체 workspace all-targets strict Clippy | **exit 0 / warning 0 / error 0** |
| Runtime lib: Reserve 모듈 제외 구간 | 333 passed / 0 failed / 기존 3 ignored |
| Runtime lib: Reserve 모듈 구간 | 90 passed / 0 failed / 0 ignored |
| **Runtime lib 합계** | **423 passed / 0 failed / 기존 3 ignored** |
| Store package | **251 passed / 0 failed / 기존 2 ignored** |
| app-server package | **124 passed / 0 failed / 기존 10 ignored** |
| 지정 Queue/Mirror integration 5개 target | **20 passed / 0 failed** |
| **중복 없는 회귀시험 합계** | **818 passed / 0 failed / 기존 15 ignored** |
| 오프라인 fixture binary build | exit 0 |
| Runtime cargo check | exit 0 |
| 변경 Rust 14개 rustfmt check | exit 0 |
| 변경 Rust 14개 공백·파일 끝 검사 | 오류 0 |

실제 strict 명령은 다음과 같다. `--all-features`를 실행했다고 주장하지 않는다.

```powershell
cargo clippy --offline --target-dir target-revision17-strict -p cdr-store -p cdr-runtime --all-targets --message-format=json -- -D warnings
cargo clippy --offline --target-dir target-revision17-strict --workspace --all-targets --message-format=json -- -D warnings
```

Runtime은 직렬 실행을 유지하며 `--skip reserve_auto::`와 `reserve_auto::`라는 서로 배타적인 구간으로 실행했다. `--list`의 426개 시험 이름과 두 실행 로그의 이름 집합을 직접 대조해 **누락 0·중복 0**을 확인했다. 기존 ignored 3개는 통과로 계산하지 않았다. 다른 package를 포함한 기존 ignored 합계는 15개다.

Queue/Mirror target은 `queue_backoff_recovery_contract`, `queue_generation_submission_contract`, `queue_recovery_contract`, `queue_recovery_resilience_contract`, `session_mirror_starting_contract`다.

## 6. 소스 동일성 및 증거

증거 경로:

```text
C:\repos\simdorei\codex-discord-remote-rust\target\qa-source\reserve-strict-r17-20260916\
```

비교 범위는 루트 Cargo.toml/Cargo.lock 및 crates 아래의 모든 .rs/.toml/.sql이다. **10개 최종 검사 각각의 전후 snapshot, 총 20개 snapshot의 1,304개 경로·SHA가 모두 최종 소스와 일치**한다. 예상하지 않은 경로 추가·삭제·수정은 없다. 로그 SHA도 실행 receipt와 다시 대조했다.

핵심 파일은 `baseline-source.json`, `before/`, `baseline-confirm.json`, 각 검사명 `.json/.stdout.log/.stderr.log`, `source-final.json`, `final-verification-summary.json`, `candidate-diff.patch`, `changed-rust-paths.json`, `run-check.ps1`, `final-audit.ps1`이다. patch는 기존 13개 파일의 비교이며, 새 시험 파일은 manifest와 원본 경로로 별도 포함한다.

리뷰 manifest:

`docs/luna-reserve-auto-switch-review-manifest-revision17-strict-clippy-20260916.json`

manifest에는 변경 14개 파일의 시작/최종 SHA, 검사 결과, 최종 소스 목록 및 증거 파일 해시를 기록한다. 이는 전체 배포 설치물·운영 설정까지 포함하는 release-package manifest는 아니다.

## 7. 중간 실행·집계 문제와 처리

최종 통과만 남기고 이전 실패 로그를 삭제하지 않았다.

1. 초기 긴 baseline 호출은 도구 시간 초과로 최종 exit receipt가 없었다. 별도 `baseline-confirm`을 실행하여 source 동일 상태의 cargo exit 101을 확정했다.
2. 도중 커넥터의 working directory가 crates 하위로 바뀐 것을 확인했다. 정확한 5060/root로 복원하고, 잘못 놓인 자체 QA runner만 정확한 위치로 옮겼다. 소스 변경은 절대 root와 예상 SHA를 확인하며 수행했다. 다른 PC나 운영 설정을 수정하지 않았다.
3. 최초 workspace --offline 검사는 `axum v0.8.9` 캐시 부재로 실패했다. `cargo fetch --locked`로 잠금 버전의 의존성만 확보한 뒤 **같은 오프라인 strict workspace 검사**가 통과했다. Cargo.lock과 Cargo.toml은 그대로다. 네트워크 사용은 이 의존성 확보에 한정하며, 시험의 백엔드·Discord HTTP는 로컬 fixture다.
4. 최초 Runtime 통합 호출은 180초 도구 제한으로 중단됐다. 완료 exit/result를 얻지 못한 실행은 최종 집계에서 제외했다. 직렬 두 구간으로 다시 실행하고 전체 시험 목록과 일대일 대조했다.
5. QA 변경 경로 JSON이 중첩 배열로 저장돼 첫 diff 집계가 거절됐다. 보존된 13개 수정 기록과 새 시험 경로로 14개 고유 문자열 배열을 재구성했고, 원래 메타데이터도 보존했다. 소스·기대값 변경은 없었다.
6. 최종 시험 이름 대조에서 Rust의 정상적인 ` - should panic` 출력 주석 3개를 이름으로 오인했다. 그 고정 출력 접미사만 정규화한 뒤 426/426, 누락·중복 0을 확인했다. 해당 시험 결과 자체는 모두 ok였으며 assertion/should_panic/ignored를 변경하지 않았다. 최초 집계 스크립트도 보존했다.

## 8. 최종 판단과 운영 경계

**남아 있던 strict Clippy 실패는 실제 수정으로 해결됐다. 같은 package 명령과 더 넓은 workspace all-targets 명령 모두 경고 없이 통과했고, 변경된 현행 소스로 818개 회귀시험을 통과했다. revision17의 수정 및 자체 코드 검수는 PASS다. 더 이상 기존 Clippy 실패를 예외로 남겨 둔 상태가 아니다.**

실제 계정·설치된 실제 app-server·운영 Discord 경로의 smoke는 이번에 수행하지 않았다. 운영 봇의 중지·교체·재시작·배포, 운영 DB/설정/비밀/사용자 대화 접근, 실제 Discord 전송, commit/push는 하지 않았다. 새 Windows/VM이나 24시간 시험을 이번 요청의 새 필수조건으로 추가하지 않았다.
