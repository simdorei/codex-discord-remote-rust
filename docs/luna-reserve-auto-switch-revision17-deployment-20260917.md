# Luna Reserve revision17 — 5060 배포 완료 기록

- 사용자 승인: revision17 CODE PASS 후 “배포하자”.
- 대상: `sim-pc-5060`, `C:\repos\simdorei\codex-discord-remote-rust`.
- 새 런타임 시작: **2026-09-17 00:49:09.381 KST**.
- 정비 완료 증거 시각: **2026-09-17 00:49:30.993 KST**.
- 독립 재확인: **2026-09-17 00:53:57 KST**, 추가 통신 관측 **00:56:30 KST**.
- 결과: **DEPLOYED_VERIFIED / Phase=verified / runtime-proof-v1**.
- 운영상 남은 사항: 완료 알림 거절 및 사용 불가 Codex 스레드에 대한 미러 연결 3개. 전체 대화방 무오류나 Reserve 자동전환의 실환경 전체 시험 완료를 주장하지 않는다.

## 1. 실제 배포 결과

| 항목 | 직접 확인한 결과 |
|---|---|
| 실행 파일 | `target\release\cdr-runtime.exe` |
| 새 프로세스 | PID **29496**, 등록된 launch 및 completion의 PID·시작 시각과 정확히 일치 |
| 이전 프로세스 | PID **15928**의 원래 프로세스 신원 종료 확인 |
| 중복 실행 | cdr-runtime 인스턴스 **1개** |
| 설치물 | 빌드 후보·배포 요청·완료 영수증의 SHA-256과 설치된 실행 파일이 일치 |
| 하트비트 | 완료 기록의 서로 다른 두 값과 재확인 중 새로 증가하는 값을 확인. 00:53:57 확인 시 최신 값의 나이 5초 |
| 제어 상태 | runtime lock 및 drain identity가 새 프로세스에 결합되고 drain은 open. 정비·disable·stop·restart·prepare·ack·cutover 표시는 없음 |
| 백업 | 정지 전/후 SQLite snapshot 2개 및 기존 실행 파일 백업의 실제 SHA-256 검증 완료 |
| 운영 설정 | `.env` 해시가 등록 당시와 같음. 비밀값을 출력하거나 변경하지 않음 |
| 제품 소스 | 검수 기준 1,304개 파일의 경로 집합 및 SHA-256 유지 |

00:56:30 KST의 추가 6초 관측에서 **새 Discord history poll 성공 기록 17개**, 그중 비어 있지 않은 메시지 조회 15개를 확인했다. 해당 봇 로그 관측 범위에는 HTTP 403/40333 기록이 없었다. 이는 Discord 조회 경로가 작동한다는 근거이며, 모든 전송·모델 추론·자동전환 경로까지 검증했다는 뜻은 아니다.

## 2. 배포 후보 준비 및 안전 절차

검수 inventory `target/qa-source/reserve-strict-r17-20260916/source-final.json`과 현재 소스 1,304개를 직접 해시 대조했다. 별도 경로에서 다음 명령을 실행하여 exit 0을 확인했고, 빌드 전후 inventory도 같았다.

```powershell
cargo build --release --locked --offline --target-dir target-deploy-r17-20260917 -p cdr-runtime --bin cdr-runtime
```

빌드 시각은 2026-09-17 00:43:00~00:45:49 KST다. 후보의 실제 운영 설정 `--check-config`도 exit 0 및 `config_valid`로 통과했다.

마지막 verified 배포의 update-only operator 및 운영 스크립트 21개의 해시 일치를 확인했고, 그 operator의 실제 app-server preflight가 성공했다. 공개 등록 템플릿은 별도 QA 티켓에서 이 PC의 루트·기존 승인 알림 채널·모듈 경로만 구체화했다. 제품 소스나 운영 정비 엔진의 안전 검사는 변경하지 않았다. `-ValidateOnly` 통과 후 기존 live-handshake-v1 / runtime-proof-v1 정비 엔진을 사용했다.

이번에 818개 회귀시험을 다시 실행했다고 주장하지 않는다. 해당 시험과 strict Clippy는 revision17 검수 기록이며, 이번에는 그 소스 동일성을 확인한 뒤 릴리스 빌드와 실제 배포 검증을 수행했다.

## 3. 연결 단절과 결과 재확인

실제 등록 및 engine 실행 요청에서 MCP가 `selected local bridge is disconnected`를 반환했다. 이 응답만으로 미실행이나 실패를 단정하지 않았고 같은 배포 명령을 반복하지 않았다.

5060이 다시 온라인이 된 뒤 동일 PC/root에 재연결했다. `registered-operation.json`이 존재했고, 정확히 같은 operation과 candidate hash의 정비 완료 기록이 verified였다. 설치물·실행 중 PID/시작 시각·launch journal·하트비트·백업·설정·제어 표시를 별도로 대조하여 완료를 확인했다.

원래 터미널 부모의 `maintenance-worker-exit.json`은 없다. 따라서 최초 원격 호출의 exit 0을 근거로 삼지 않는다. **영속 정비 기록과 실제 실행 상태에 근거한 최종 검증 스크립트가 exit 0**으로 통과했다.

## 4. 남은 운영 사항

### 4.1 완료 알림 거절

해당 operation의 `notification.json`은 다음 상태다.

- Status: `rejected`
- HTTP status: **403**
- Discord code: **40333**
- Receipt: 없음

따라서 Discord에 배포 완료 알림이 전달됐다고 주장하지 않는다. 배포 성공과 알림 성공은 별개로 관리되며, 이번 알림 거절로 봇을 다시 중지·롤백하거나 동일 알림을 자동 재전송하지 않았다. 구체적인 거절 원인은 이번 기록만으로 확정하지 않는다.

### 4.2 미러 연결 3개

새 런타임 로그에서 다음 오류가 반복 관측됐다.

```text
session mirror poll failed for 3 target(s)
mapped Codex thread is unavailable
```

따라서 이 3개 연결까지 정상이라고 확정하지 않는다. 배포 전부터 있던 상태인지 이번 변경으로 새로 발생한 회귀인지는 이번 관측만으로 판정하지 않았다. 연결·대화 기록을 임의 삭제하거나 해당 사용자 입력을 재실행하지 않았다. 사용자 메시지 본문이나 대상 스레드 식별자는 이 보고서에 기록하지 않는다.

## 5. 증거 위치

```text
C:\repos\simdorei\codex-discord-remote-rust\target\qa-source\reserve-deploy-r17-20260917\
```

주요 파일:

- `candidate-build.json`, `candidate-build.stdout.log`, `candidate-build.stderr.log`
- `source-before-build.json`, `source-after-build.json`
- `candidate-config.stdout.log`, `candidate-config.stderr.log`
- `deployment-request.json`, `register-reviewed-candidate.ps1`
- `registered-operation.json`, `reconnection-observation.json`
- `verify-deployment.ps1`, `deployment-final-verification.json`
- `completion-receipt.json`, `post-deploy-observation.json`

중요한 실행 receipt:

- 소스 해시 대조: `tr_f9477ea5f8904e9e`
- operator 및 21개 스크립트 일치·실제 preflight: `tr_23ba5aec0e434d7b`
- 릴리스 빌드: `tr_557f56eb19e9491c`
- 후보 설정·등록 ValidateOnly: `tr_6b48bc2477454769`
- 재연결 후 완료 관측: `tr_2b2cee0305df486c`
- 최종 배포 검증 exit 0: `tr_28ae7305d92944c6`
- 추가 Discord 조회 관측: `tr_c18fb3ebe28548d0`

배포 준비 QA 스크립트의 최초 JSON 배열 수집 오류는 빌드 전에 거절됐고, 원본 스크립트를 보존한 뒤 배열 읽기만 수정했다. 실제 제품 소스 개수와 경로는 처음부터 1,304개로 같았다. 최초 최종확인 호출은 검사를 마친 뒤 결과 객체의 PowerShell boolean 표기에서 실패하여, 올바른 boolean을 사용한 별도 `verify-deployment.ps1`로 모든 검사를 다시 수행하고 exit 0을 확인했다. 이 두 문제를 제품 코드 결함이나 성공한 실행으로 포장하지 않는다.

실제 사용자 진단 프롬프트 전송, 실패했던 입력 재전송, 한도 소진을 강제로 유발하는 자동전환 시험, Git commit/push는 수행하지 않았다.
