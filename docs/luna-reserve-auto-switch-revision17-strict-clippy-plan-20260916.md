# Luna Reserve revision17 — strict Clippy 실패 해소 계획

- 요청일: 2026-09-16 (Asia/Seoul)
- 대상: sim-pc-5060 / C:\repos\simdorei\codex-discord-remote-rust
- 사용자 요청: 실패가 남아 있으면 계획을 다시 세우고 수정한다.
- 상태: 구현 전 계획. 아래 검사는 아직 통과했다고 주장하지 않는다.
- 운영 봇, 운영 DB/설정/비밀, 실제 사용자 요청, 배포/재시작, commit/push는 범위 밖이다.

## 1. 문제와 완료 기준

이전 R15-1 집중 수정의 기능 시험은 성공했지만 `cargo clippy --offline -p cdr-store -p cdr-runtime --all-targets -- -D warnings`는 exit 101이었다. 일반 Clippy 성공이나 변경 파일의 경고 부재로 이 실패를 대신하지 않는다. 현행 코드를 기준으로 RED를 재현하고 원인을 수정한 뒤 같은 strict 검사와 workspace all-targets strict 검사를 재실행한다. 경고 억제 attribute, lint 등급 완화, 검사 target 삭제로 성공을 만들지 않는다.

## 2. 확인한 원인과 최소 수정

| 원인 | 수정 계획 | 보존할 동작 |
|---|---|---|
| Store usage_fence의 항상 성공하는 빈 migrate/schema_current 함수 | no-op 함수와 부모의 무의미한 호출 제거 | 부모가 담당하는 21개 column/index 및 notice schema 검사, SQL/마이그레이션 실패 전파 |
| 같은 match body 및 중첩되지 않은 or-pattern | 같은 값의 분기를 합치고 내부 패턴으로 정리 | auto/on, manual/off, unknown/missing mode의 기존 분기 |
| 같은 approval_http 파일을 여러 모듈로 로딩 | 기존 crate::test_support::approval_http를 alias로 재사용 | 각 시험의 새 loopback listener/독립 상태 및 실제 HTTP assertion |
| 단일 원소 slice 생성을 위한 clone | std::slice::from_ref와 slice 비교 사용 | 소유권 snapshot 전체 동등 비교, 모든 assertion |
| completion 함수 길이 | 최종 staging/delivery/journal cleanup 부분을 private helper로 추출 | 캡처한 같은 owner, await/DB/전송 순서, progress/Final 분기 |
| fixture의 긴 run closure | 동일 상태를 소유하는 fixture와 RPC별 helper 분리 | gate, settings 적용, persist, notification, reply 순서와 모든 fault injection |
| Goal mirror 시험 helper 길이 | 중복 fixture 요청을 helper로 추출 | early/late 두 순서, timeout, assertion 및 실제 observer/HTTP 경계 |
| 문서 주석 lint | 식별자 backtick 보완 | 실행 동작 변경 없음 |

## 3. 수행 순서

1. 기존 dirty 변경은 보존하고 소스 baseline SHA와 수정 대상 원본을 저장한다. 현재 strict 실패를 실제 재현한다.
2. 읽은 대상만 충돌 검사를 거쳐 수정한다. 예상하지 못한 소스 변경은 덮어쓰지 않고 다시 대조한다.
3. Store schema 회귀시험과 completion/fixture 관련 기존 회귀를 실행한다. 테스트 기대값을 완화하거나 제외하지 않는다.
4. 최종 후보에서 strict package/workspace all-targets Clippy, offline fixture build, Runtime lib, Store, app-server, 지정 Queue/Mirror integration, 변경 Rust 포맷/공백 검사를 실행한다.
5. 각 검사 전후 소스 동일성과 exit code, 시험 결과를 보존한다. 후속 lint/컴파일/시험 실패는 원인을 확인해 수정하고 최종 검사들을 다시 맞춘다.
6. 실제 실행 결과와 한계를 review notes 및 SHA manifest에 남긴다. 기능 수정/정적 품질 통과와 운영 적용 여부를 구분한다.

## 4. 회귀 보호

R15-1의 actual Reserve 검증 우회 방지, 기본 effort -> high -> medium, medium 미만 거절, 후검증, exact usage-fence, ordinary/manual/off 예외, 과거 대화/Goal 상속, 원래 실패 요청 자동 재실행 금지, 1회 Final/Failed 및 Queue/Mirror 소유권은 유지한다.

새 Windows/VM 및 24시간 시험을 새 필수조건으로 추가하지 않는다. 실제 운영 smoke를 실행하지 않았으면 실행했다고 표시하지 않는다. 기존 ignored는 통과 수에 포함하지 않는다.

## 5. 완료 기록

계획을 구현했고 최종 검증은 2026-09-16 23:58:48 KST 기준 PASS다. Store/Runtime 및 workspace all-targets strict Clippy는 모두 exit 0, warning/error 0이다. 지정 회귀시험은 818 passed / 0 failed / 기존 15 ignored이며, Runtime 시험 이름 426개를 비중복 구간 결과와 대조했다. 최종 1,304개 소스는 각 성공 검사 전후 snapshot과 모두 일치한다.

상세 수정·중간 실패의 해결·검수 결과·운영 미실행 범위는 `docs/luna-reserve-auto-switch-revision17-strict-clippy-review-notes-20260916.md`에 기록했다. 원래 구현 전 계획은 QA 폴더의 `plan-before-completion.md`에 보존했다. 운영 배포는 하지 않았다.
