# 미러 정리·목록 누락 수정 및 검수 결과

- 날짜: 2026-09-17, Asia/Seoul
- 대상: sim-pc-5060 / `C:\repos\simdorei\codex-discord-remote-rust`
- 요청: 두 문제의 별도 수정 계획 수립 → 계획 검수 → 구현 → 회귀시험 및 코드 재검수
- 계획 판정: **PLAN PASS**
- 구현 판정: **이번 변경 범위 CODE PASS**
- 운영 상태: **이번 변경은 배포하지 않음. 실제 방 삭제·운영 DB 변경·봇 재시작·사용자 입력 재전송·commit/push 없음.**
- 검수 방식: 같은 세션의 반례 중심 자체 검수. 별도 사람이나 독립 에이전트가 수행한 검수라고 주장하지 않음.

## 1. 수정 A — 아카이브된 미러 방의 확정 미실행 기록 정리

기존 `pending_reason`은 실행되지 않았다는 명확한 결과가 있어도 `held` 입력을 모두 미완료로 간주했다. 일반 삭제 검사는 그대로 보수적으로 유지하고, 아카이브된 원본이 확인된 방에만 별도 경로를 추가했다.

예외 대상은 정확한 방·대화·사용자·원래 busy-choice 신원이 일치하는 만료된 Steer 사전 거절이다. `kind=interaction`, `state=held`, `phase=result_recorded`, 소유 job 없음, 소유권 영수증 없음, 원래 버튼의 `allow_steer=false`, 정확한 결과 종류와 JSON boolean `control_dispatched=false`가 필요하다. 잘못된 JSON, 문자열 `"false"`, 숫자 0, 다른 action/사용자/방/대화/소유자, 아직 만료되지 않은 선택은 허용하지 않는다.

`BEGIN IMMEDIATE` 안에서 후보 증거·정확한 매핑과 부모 방·다른 대기 항목을 다시 확인하고, 삭제 보호 기록과 증거 스냅샷을 함께 저장한다. 큐, intake, 다른 ingress, 결과·진행·goal outbox, 살아 있는 busy choice, 미확정·대상 불명 전달 영수증은 독립적으로 삭제를 막는다.

삭제 전 Discord ID/guild/종류/부모를 확인하고, 느린 조회 이후에도 원본이 같은 아카이브 시각과 rollout 경로를 가지며 파일이 존재하는지 다시 확인한다. 원본 복원·삭제·rollout 유실 또는 증거 저장 실패는 DELETE 전에 중단한다.

원래 ingress 행은 변경하거나 삭제하지 않는다. 알림 전달을 성공 처리하지도 않는다. `cdr_archived_cleanup_evidence`에 원래 payload/outcome과 전체 행의 스냅샷, 아카이브 관측, 정확한 close token을 보관한다. UPDATE/DELETE 방지 트리거와 ingress 조회 인덱스를 추가했다. 복구는 같은 증거와 삭제 보호 기록으로 연결된 원래 행에 대해 새 오류 알림을 생성하지 않는다.

Discord 삭제 성공 또는 권위 있는 방 부재 관측 뒤에만 정확한 매핑을 정리한다. 삭제 결과 유실, 완료 저장 실패에는 매핑과 `deleting` 보호 기록을 유지하며 다음 sync에서 재삭제하지 않는다. 명확한 삭제 거절에만 같은 token의 보호 기록을 해제한다.

### 실제 대상 방과의 관계

`1545155901659283497`의 종전 조사 문서에 기재된 두 사전 거절 유형을 오프라인 fixture로 재현했다. 이번 세션에서는 운영 DB를 다시 읽거나 실제 방을 삭제하지 않았다. 향후 배포 후 전체 미러 동기화에서 실제 당시 상태를 다시 검사해야 하며, 새로 생긴 대기 작업이나 바뀐 증거까지 무조건 무시하는 기능이 아니다.

## 2. 수정 B — 미러 목록 출처 누락과 일반 목록의 범위 안내

직접 확인한 실제 누락 조건은 기본 미러 동기화 및 미러 list/check가 `source='vscode'`만 허용하던 것이다. 별도 `load_mirror_root_threads()`를 추가하여 `vscode`, `cli`, `app-server`, `appServer`의 사용자 루트를 포함하도록 했다. 미아카이브·사용자 또는 구형 빈 provenance·비어 있지 않은 제목 조건을 유지한다. 내부/subagent 루트는 새로 가져오지 않으며, 이미 매핑된 활성 분기 대화는 계속 보존한다. 예전 VS Code 전용 reader 자체는 변경하지 않았다.

일반 `!list` 및 slash list는 애초에 앱서버의 소유권 검증 결과로 목록을 필터링하지 않았다. 전체 로컬 DB 목록을 읽고, `thread/read` 관측을 부가 정보로 표시하는 구조다. 따라서 일반 목록의 권한 누락을 재현·해결했다고 과장하지 않는다.

일반 목록에는 표시 개수/전체 개수, 설정된 로컬 Codex DB의 미아카이브/아카이브 범위, 다른 PC/CODEX_HOME은 포함하지 않는다는 안내를 추가했다. 조회 실패·notLoaded·서버 없음·50개 관측 예산 초과에서도 목록 항목과 번호가 유지되는지 시험했다. 표시 번호와 기존 선택 참조 체계는 바꾸지 않았다.

미러 list/check도 새 출처 조건을 함께 사용하며 매핑 누락의 ID·제목·작업 경로와 조회 범위를 표시한다. 목록을 채우려고 resume/start/steer/stop을 호출하거나 다른 연결의 실행 소유권을 가져오지 않는다.

## 3. 계획 검수 및 구현 후 발견·보완

| 검수 지점 | 반례 | 최종 처리 |
|---|---|---|
| 일반 예외의 위험 | `control_dispatched=false` 하나만으로 다른 요청까지 무시 | 원래 신원·형식·만료·소유권을 모두 검사하는 아카이브 전용 경로 |
| 읽기 후 경쟁 | Discord 조회 중 입력·매핑·원본 상태 변경 | write transaction 안의 재검사와 마지막 아카이브 확인 |
| 증거 저장 실패 | 방은 지우고 감사 자료는 없는 상태 | 증거+삭제 보호 기록 원자 저장, ABORT 주입 시험 |
| 복구의 알림 재생성 | 원래 held 행 보존 후 재시작이 다시 POST | 정확한 audit+close token에 묶인 복구 예외 |
| 삭제 결과 유실 | 두 번째 sync가 삭제를 반복 | mapping/fence 보존, 실제 dispatch 1회 검사 |
| 내부 대화 유입 | 출처 확대가 subagent까지 신규 미러링 | 사용자 루트만 포함, 기존 매핑 분기는 별도 보존 |
| 목록 조회 중 생성 | 두 로컬 목록 조회 사이 새 대화가 생기면 map 인덱싱 panic | `get()` 기반 표시, ID 보존과 재조회 안내, 새 회귀시험 |
| strict Clippy | 긴 inspection 함수와 불필요한 값 소유 인수 | helper 분리와 참조 인수로 해결. 경고 억제 추가 없음 |
| 스키마 계약 | 추가 감사 테이블/인덱스가 기존 기대 목록에 없음 | 정확한 기대 목록만 확장, 기존 검사 유지 |

서로 다른 Codex DB와 미러 DB/Discord 전체를 하나의 트랜잭션으로 묶지는 못한다. 마지막 아카이브 확인 이후 외부 앱이 복원하는 극단적인 경쟁까지 원자적으로 차단했다고 주장하지 않는다. 원본 Codex 대화/rollout을 수정하지 않으며, 미확정 삭제는 재시도하지 않는다.

## 4. 실행한 검증

이번에 새로 추가한 회귀시험은 25개다. 먼저 기존 제품 코드에서 핵심 2개 시험의 실패를 확인했다. 하나는 `CleanupProtected { channel:31, reason:"ingress" }`, 다른 하나는 CLI/app-server 사용자 루트의 매핑 누락이 목록에 잡히지 않는 실패였다. 구현 후 두 시험과 확대 반례가 통과했다.

### 최종 결과 — 중복 집계 없음

| 범위 | 통과 | 실패 | 기존 제외 |
|---|---:|---:|---:|
| cdr-codex-state 패키지 | 29 | 0 | 1 |
| cdr-store 패키지 | 254 | 0 | 2 |
| cdr-runtime 라이브러리 전체 항목 | 427 | 0 | 3 |
| mirror_sync_contract | 48 | 0 | 0 |
| 목록·읽기 전용·쓰기 소유권 관련 통합시험 4개 target | 14 | 0 | 0 |
| **합계** | **772** | **0** | **6** |

runtime은 4개 첫 답변 시험, 나머지 completion 78개, 다른 라이브러리 348개(345 통과+3 제외)로 나눴다. `--list`로 얻은 430개 이름과 최종 실행/기존 제외 430개 이름을 대조하여 누락 0·중복 0을 확인했다. Rust harness의 ` - should panic` 표시만 시험 이름 비교에서 정규화했다. 제외 항목은 통과 수에 넣지 않았다. 전체 workspace 테스트를 실행했다는 의미가 아니다.

추가로 `cargo clippy --offline --locked --workspace --all-targets -- -D warnings`가 최종 소스에서 exit 0으로 통과했다. 이번 변경 파일 22개는 파일 묶음별 `rustfmt --edition 2024 --config skip_children=true --check` 22/22 통과다. 최종 후보의 Rust 소스·Cargo 파일·관련 fixture 1,315개 SHA-256이 마지막 감사에서도 일치했다. 이는 해시 대조이며 1,315개 전체 소스의 행 단위 독립 검수를 주장하는 수치가 아니다.

### 중간 실패와 남은 제한도 보존

고병렬 completion 실행에서는 기존 첫 답변 시험 4개가 2초 제한을 넘겼다. 제품 코드, assertion, 2초 제한을 바꾸지 않은 단독/낮은 병렬 실행으로 모두 통과했다. 초기 실패 기록은 보존했다. 부하 민감성이 관측됐다는 사실과 최종 저병렬 통과를 구별하며, 원인을 완전히 입증하거나 무부하·장시간 운영을 검증했다고 하지 않는다.

복원 시험 fixture는 아카이브된 프로젝트 매핑을 처음부터 생략한 fixture였다. 활성 복원 시 그 부모 프로젝트 매핑도 복원하도록 고쳐 정상 활성 방을 검증했다. 삭제 방지 assertion은 유지했다.

`cargo fmt --all -- --check`는 Windows 명령줄 길이 제한(os error 206)으로 실행되지 못했다. 대신 1,298개 Rust 파일을 20개씩 읽기 전용 검사했으며 이번 변경 밖 13개 파일에서 포맷 차이가 나왔다. 해당 파일들은 임의 수정하지 않았다. 따라서 **전체 저장소 포맷 gate까지 통과했다고 보고하지 않는다.** 목록은 manifest의 `qualifications.out_of_scope_format_differences`에 있다. 원래 소스 정리까지 포함하는 배포 정책이라면 이 별도 항목을 처리해야 한다.

실제 Discord·설치된 app-server·운영 계정·특정 방의 현재 상태·다른 PC/CODEX_HOME 목록 통합은 시험하지 않았다. 운영 배포 및 방 삭제 완료 판정은 아니다.

## 5. 파일과 재현 근거

계획 및 계획 검수: `docs/mirror-cleanup-and-list-plan-20260917.md`

변경 전/후 파일 SHA, 최종 검사 영수증, 제외 사항: `docs/mirror-cleanup-and-list-manifest-20260917.json`

이 보고서: `docs/mirror-cleanup-and-list-review-20260917.md`

증거 폴더: `target/qa-source/mirror-list-fix-20260917/`

주요 기록: `baseline.json`, `before/`, `red-baseline.*`, `runtime-completion.*`(최초 시간 초과), `runtime-first-reply-isolated.*`, `runtime-other-final.*`, `runtime-completion-final.*`, `store-final.*`, `state-regression.*`, `mirror-final-source.*`, `list-integration-final.*`, `strict-clippy-final-source.*`, `fmt-final.*`, `fmt-batched.*`, `fmt-changed.*`, `runtime-inventory.*`, `candidate-source.json`, `candidate-diff.patch`, `final-verification-summary.json`.

실행/집계 도구: `run-check.ps1`, `check-format.ps1`, `final-audit.ps1`. 프로세스 시작·종료·stdout/stderr를 저장한다. 연결 선택 교체가 한 차례 있었으며, 같은 PC/root에 재연결한 뒤 실행 중 프로세스와 영수증을 확인하고 미확정 검사를 무작정 중복 실행하지 않았다. 이때 관찰된 별도 프로젝트의 release 빌드는 이 작업에서 시작하거나 중지한 것이 아니다.

**최종 결론: 요청한 두 문제의 코드 수정과 계획/구현 검수는 완료했다. 이번 변경 범위는 CODE PASS이며, 운영 적용과 실제 방 정리는 아직 하지 않았다.**
