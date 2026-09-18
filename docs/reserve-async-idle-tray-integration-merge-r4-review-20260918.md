# Reserve · 비동기 질문 · 유휴 해제 · 미러 · 트레이 통합 확정 리뷰 R4

## 판정과 범위

**CODE PASS / 최종 통합 소스의 main fast-forward 반영 가능.** 같은 작업자가 수행한 통합 확정 및 자체 재검수다. 이전 R3 독립 코드 검수의 결과를 다른 버전이나 실제 운영 배포 성공으로 확대하지 않는다.

대상은 5060의 `target/qa-source/reserve-async-idle-tray-integration-20260918/` 작업 폴더다. 운영 저장소 루트의 실행 파일·스크립트·DB·환경 설정은 변경하지 않는다. 실제 커밋·main 반영·원격 push 결과는 아래 증거 폴더의 `candidate.json`, `publication.json`, 최종 `manifest.json`으로 구분한다. 이 문서 자체는 실행 파일의 배포 승인서가 아니다.

## 1. 이번에 통합 확정하는 변경

기준 `a5b291d0ff21`은 이미 `8b68180`(비동기 질문·유휴 해제)과 `33eafd4`(Reserve·Goal·미러/list·트레이)를 두 부모로 갖는 merge commit이다. 마지막 R3의 Rust 파일 14개만 미커밋 상태였다. 원격 main `8b68180`은 이 기준 커밋의 조상임을 다시 확인했다. 따라서 R3 전체와 아래 패키지 버전 수정을 하나의 후속 커밋으로 기록하고 main을 fast-forward하면 모든 필수 기능 이력을 유지한다. 기존 merge commit을 amend/rebase/squash하지 않는다.

R3 제품 코드 9개와 시험 5개를 모두 포함한다. 특히 새 `async_question/ownership.rs`와 새 시험 4개를 빠뜨리지 않는다. R3 검증 당시의 소스 inventory 1,599개를 현재 파일과 다시 해시 대조해 모두 일치함을 확인했다. 최종 재검증은 SQL·JSON·기타 파일까지 포함하는 Git 관리 대상의 문서 제외 파일 **1,673개**를 실행 전후에 고정했다.

다른 로컬 통합 초안 `7020cf4`도 같은 두 부모를 갖지만 현재 후보와 50개 코드/스크립트 파일에서 차이가 나는 별도 구현이다. 검수된 R3를 다시 바꾸게 되는 중복 통합은 하지 않는다. 해당 초안과 다른 사용자 자료는 삭제하거나 덮어쓰지 않는다. 이번 범위는 지정된 기능들의 승인 기준 및 R3 수정 전체이며, 이름이 다른 모든 과거 실험 브랜치를 무조건 합치는 작업이 아니다.

## 2. 추가로 발견하고 수정한 main 통합 문제

실제 CI와 같은 `cdr-pro-helper verify-cachebuster`를 원격 main 기준으로 실행하자, 변경된 플러그인 컴파일 입력에 비해 버전이 증가하지 않았다는 오류로 종료코드 2가 나왔다.

수정 파일:

```text
plugins/codex-discord-remote/.codex-plugin/plugin.json
```

버전은 `0.1.0+codex.20260911034456`에서 `0.1.0+codex.20260918082031`로 갱신했다. 버전 외 JSON 필드는 동일함을 별도로 검증했다. 검사 함수나 CI를 우회·삭제하지 않았다. 수정 전 실패는 `cachebuster-precommit.json`과 로그에 보존했고, 수정 후 `final-cachebuster`가 종료코드 0이다. 버전 수정 전의 성공 시험은 최종 합계에 더하지 않고, 수정 후 동일 최종 소스로 다시 실행했다.

## 3. 코드 대조 결과

| 경계 | 확인 결과와 근거 |
|---|---|
| 상속된 질문의 현재 실행 신원 | `async_question/ownership.rs`, `inbox.rs`가 원래 실행 세대와 현재 turn 관측 세대를 구분한다. 유일한 정확한 job/turn/사용자/방만 질문 제어 권한을 얻는다. |
| 질문 선도착과 Final 경합 | `delivery.rs`에서 job 삭제 전에 같은 트랜잭션으로 질문을 확정한다. 세대 없는 outbox만으로 과거 질문을 승격하는 fallback은 없다. 저장 실패 시 Final과 job 삭제도 rollback된다. |
| 실제 전송 및 ACK | `dispatch.rs`, `guard.rs`가 claim 이후 신원 변경·중복 owner를 거절한다. 미확정 결과를 자동 재전송 가능한 상태로 바꾸지 않는다. |
| 만료 질문의 유휴 해제 | `idle_release.rs`는 dispatch 흔적이 전혀 없는 expired 질문만 차단에서 제외한다. tombstone, Unknown, 진행 중 답변, Reserve fence와 정리 보호는 유지한다. |
| 스키마 호환 | `async_question/schema.rs`는 신규 출처 열을 NULL로 추가하고, 과거 cross-generation 후보에 출처를 만들어 넣지 않는다. |
| 트레이·Reserve 원본 유지 | R3 Rust 14개와 이번 plugin 버전 외에 제품 동작을 추가 변경하지 않았다. 기존 트레이 스크립트는 기준과 동일하다. |

신규 19개 반례의 실제 코드와 실행 결과를 대조했다. Store 17개는 현재 질문 소유권·Final 원자성·마이그레이션·만료 기록을, Runtime 2개는 native 알림→질문 버튼→정확한 Steer 1회 및 잘못된 사용자/방/새 resident 거절을 검사한다. 추가 필수 제품 수정이 필요한 구체적 반례는 이번 대조에서 찾지 못했다.

## 4. 최종 재실행 결과

| 범위 | 통과 | 실패 | 기존 제외 |
|---|---:|---:|---:|
| Store + app-server 전체 패키지 | 441 | 0 | 12 |
| Runtime 전체 lib + 지정 integration 17개 | 578 | 0 | 3 |
| Pro helper + Discord 전체 패키지 | 203 | 0 | 0 |
| **중복 없는 합계** | **1,222** | **0** | **15** |

Runtime 578개는 lib 453개와 integration 125개다. 125개 안에 트레이 29개와 mirror sync 48개가 포함된다. 신규 반례 19개도 위 합계 안에 포함하며 별도로 더하지 않는다. 처음 실행한 같은 시험과 집중 재실행은 합계에서 제외했다. 기존 제외의 이름은 최종 원본 로그와 manifest의 ignored 목록에 남긴다.

추가 최종 게이트:

- Runtime·offline fixture·Pro helper의 같은 target 디렉터리 사전 빌드: exit 0.
- `cargo check --offline --locked --workspace --all-targets`: exit 0.
- `cargo clippy --offline --locked --workspace --all-targets -- -D warnings`: exit 0.
- 전체 Rust 포맷: 11 packages / 458 target roots / 62 batches, exit 0.
- main 기준 플러그인 cachebuster 검사: 수정 후 exit 0.
- 최종 staged diff와 커밋 소스의 일치는 커밋/인계 영수증에서 별도로 확인한다.

전체 workspace의 모든 integration 시험, macOS 실행, 실제 Discord/Desktop/Explorer UI, release 빌드, 운영 교체는 이번 검사 범위가 아니다. 새 Windows/VM 또는 24시간 시험을 추가 배포 조건으로 만들지 않는다.

## 5. 증거 패키징 보완

증거 폴더는 이 worktree 기준 `target/qa/integration-merge-r4-20260918/`다.

`Run-MergeCheck.ps1`은 빈 stdout도 실제 빈 문자열로 보존하고 종료코드·timeout·출력 수집 완료 여부를 따로 기록한다. 출력을 읽지 못했거나 소스가 달라지면 성공으로 집계하지 않는다. `source-baseline-before-plugin-version.json`과 `plugin-version-fix.json`으로 허용한 단일 버전 변경을 명시한다. 최종 명령은 `final-*` 영수증만 채택한다. 이전 실패·초기 시험은 원본 그대로 남긴다.

최종 manifest는 고정 commit/tree/조상, 최종 소스, 실행별 before/after, 실제 로그, QA용 debug 실행 파일 해시를 연결한다. debug 산출물을 release 배포본이라고 표시하지 않는다. 보고서가 자신의 commit hash를 포함하는 순환 참조는 만들지 않으며, 실제 commit 식별자는 외부 candidate 영수증에 기록한다.

## 6. main 반영 원칙

원격 main이 고정한 기준에서 움직이지 않았는지 다시 확인하고, 새 후보가 모든 필수 조상을 포함하는지 검사한다. 운영 루트 checkout은 유지한 채 격리된 임시 main worktree에서 `git merge --ff-only`로 반영한다. 강제 push, 전역 reset/clean, 미확정 기록 삭제, 운영 프로세스 중지 또는 재시작은 하지 않는다. 원격 선두 이동이나 새 파일 변경이 발견되면 해당 단계는 중단하며, 완료 여부를 추정하지 않는다.

**최종 결론: 지정 기능 전체와 R3 수정은 함께 main으로 통합 가능하며, 추가로 발견한 패키지 버전 누락도 수정·재검증했다. 실제 운영 배포와 새 원격 CI의 완료 여부는 소스 병합 완료와 구분한다.**
