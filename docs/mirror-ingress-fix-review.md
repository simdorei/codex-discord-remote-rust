# Mirror ingress cleanup review and QA

## Current checkpoint (2026-09-13 19:15 KST)

Mirror product fix and regression tests are implemented in the isolated worktree
and staged as source on 5060. No release binary has been installed. Required Pro
code review is blocked at composer control; plan reviews alone do not authorize
deployment. The separate question/answer investigation is a plan, not a feature
implementation.

Second native QA run: workspace build, all-target unit/integration test command,
workspace clippy with `-D warnings`, formatting and diff checks passed. Collected
output contains at least 1,792 passing tests and 17 ignored tests, but one output
chunk was truncated, so these are NOT certified aggregate counts. No executed
test failed in this run; existing external/live ignores were not changed.

The full wrapper script exited 1 at setup-discord-bot.ps1 because its default
target/release/cdr-runtime.exe does not exist in this new worktree. Do not claim
`native_workspace_checks_passed`. Remaining PowerShell and Git Bash setup dry
runs passed separately with the supported explicit BinaryPath/--binary-path
pointing at the freshly built target/debug/cdr-runtime.exe. Shell syntax and
install dry-run passed. No token, .env, scheduled task or service was changed.
This verifies the wrapper behavior, NOT a release artifact or live deployment.

Next: restore Pro composer control and use the still-unused second focused
follow-up for code review; then build/pin a release candidate, rerun the default
wrapper checks against that artifact, and perform the authorized 5060-only
identity-bound update and live cleanup verification. Do not replay the historic
held sync request. Existing Issue1 busy/steer edits remain excluded.

## Scope and provenance

Isolated e401968 worktree. Deployment target: 5060 only.
The separately prepared busy/steer Issue 1 patch is excluded.
Product changes and regression tests are in progress in the isolated worktree.
Live data and deployed binaries have not been changed.

Plan consultation: https://chatgpt.com/c/6aa6647c-b4c0-83e8-a172-5997ef48dd64
Normal Chat, 6 Pro, exact Simdorei Local Project Oauth connector verified.
Initial consultation was read-only. Verdict: PLAN PASS (7m16s).
Pro read 12 relevant files; slash source inspection was interrupted by lost
connector selection. A focused follow-up is explicitly authorized to reconnect
the same PC and check slash and notification-loss boundaries. This is not CODE PASS.

Accepted requirements: NULL-safe narrow owned exclusion, typed protection
refusal in BOTH precheck and transactional begin, actual affected room identity,
no claim that earlier sync changes were rolled back, known refusal distinct from
unconfirmed notification, no mass history edits or uncertain replay.

New store fixture initially failed to compile because queue APIs require a
baseline array and generation, not a timestamp. Corrected fixture arguments;
this setup error is NOT recorded as contract RED.

The malformed future-state fixture also initially hit the schema's CHECK
constraint. It now explicitly bypasses CHECK constraints on the isolated test
connection only, to test a corrupt/forward-state record without relaxing production.

## MC-1 RED

`cargo test -p cdr-store --locked --test room_cleanup_handoff_contract`
After fixture corrections: 3 passed, 2 failed. Both settled-owned and explicitly
retracted-owned cases failed with Some("ingress") instead of None. Pending,
malformed/unconfirmed, and same-target/other-channel negative controls passed.

## Baseline checks (before new regression tests)

Working directory: D:/codex-work/cdr-mirror-ingress-20260913
CARGO_TARGET_DIR: same directory's target subdirectory.

- `cargo test -p cdr-store --locked --test room_cleanup_contract`
  PASS: 8 passed, 0 failed, 0 ignored.
- `cargo test -p cdr-runtime --locked --test mirror_sync_contract`
  PASS: 29 passed, 0 failed, 0 ignored.

These are existing baseline tests, not RED/GREEN proof for the reported bug.
MSVC emitted informational linker-output warnings; no build or test failed.

## Remote pre-deployment observation

2026-09-13 17:54 KST: clean e401968 on 5060; one bot PID 21664,
started 17:23:12 KST; heartbeat fresh. Cargo and Git available, C: free ~71 GB.
Receipt tr_b7f82633a5ac4a67. No stop/restart/deploy command sent.

One connector selection loss occurred during concurrent read-only review.
Codex rebound the same PC/folder and verified device_info before inspection.
No remote command with possible mutation was replayed.

## Focused plan follow-up

Completed 2026-09-13 (6m47s), same consultation: PLAN PASS. Pro read the
5060 slash/message custody and delivery paths. Additional accepted requirements:
slash outcome persistence must update custody's result_recorded flag; confirmation
write failures must be separated from uncertain delivery; use a bounded single
initial-response PATCH for slash refusal; restart recovery recognizes the persisted
refusal, suppresses only its extra Saved notice, and never fabricates confirmation.
Ordinary unknown outcomes retain their existing hold/notice behavior. This is still
not implementation approval.

## MC-1 GREEN and MC-5 RED

After the narrow NULL-safe predicate change: handoff contracts 5/5 and original
room cleanup contracts 8/8 passed. No live journal rows were rewritten.

MC-5 transactional refusal test failed as intended before the typed-error change:
actual Integrity("room 31 protected by queued requests"), expected CleanupProtected.

## Implementation checkpoint / focused QA

2026-09-13 18:38 KST, before remote source staging:

- MC-5 GREEN: handoff/guard contracts 8 passed. Includes all independent pending
  guards and true DB/fence errors. Busy fixture initially omitted allow_steer;
  its setup failure was corrected and is not contract RED evidence.
- MC-8 RED: restart replaced the known refusal reason with the generic runtime
  ended message. After the narrow persisted-outcome branch: 2 new + 8 original
  custody tests passed, including ordinary/malformed outcome controls.
- MC-6 MESSAGE RED: real executor/worker sent generic error with no stored
  refusal. After the fix, all 3 message worker tests passed (normal, receipt
  failure, ingress-confirm failure). Store/HTTP boundaries are real/substituted.
- MC-6 SLASH RED: after correcting the fixture's command name to bridge_sync,
  no refusal outcome existed before PATCH. The earlier unregistered-command
  fixture error is not RED. After the fix, 3 slash tests passed (normal finish,
  confirm failure, interruption during PATCH/recovery).
- Mirror sync integration: 32 passed (29 original + 3 public-lifecycle, pending
  precheck and missing-room race cases). Keeps earlier rename and pending map.
- Runners inspection: 7 + 1 passed; known refusal is not shown as successful,
  exact outcome and original payload survive inspection, other actors refused.
- `cargo clippy -p cdr-runtime -p cdr-store --all-targets --locked -- -D warnings`
  PASS. Fixed new checked-conversion/float-bit-comparison lints, split refusal
  delivery from execution and interaction error reporting into focused modules.
  Shared test-only slash HTTP helper explicitly allows reuse without that helper.
- Package-by-package rustfmt completed. A combined formatter invocation exceeded
  the Windows argument limit; no formatting check was bypassed.

Full native workspace checks and Pro implementation review are pending.
No implementation approval or live cleanup success is claimed at this checkpoint.

5060 watchdog independently restarted e401968 at 18:07:30 KST because its old
heartbeat went stale. New PID 48740 is the only bot; fresh heartbeat and clean
e401968 source were checked before staging. Codex did not request that restart.
The separate async-question diagnosis is documented but is not in this code fix.

## Full QA and Pro connection checkpoint

The first full native run stopped at
`interaction_idempotent_wiring_contract`: its source-level assertion still
searched the old interaction_worker.rs location after error reporting moved to
interaction_worker/error_report.rs. Updated the contract to assert the module
wiring and the same idempotent-delivery requirements in that new file. No
delivery-safety assertion was removed. Focused rerun: 2 passed, 0 failed.
This is a refactor-sensitive wiring-test failure, not additional product RED.

All 36 staged source/test/document files were read back on 5060 and their
SHA-256 values matched the corresponding writes. Source staging is not binary
deployment. The new wiring-test update must also be staged before code review.

The canonical Pro conversation was retained. Chrome page control repeatedly
timed out; opening the same canonical conversation in a new tab briefly restored
observation, but the permitted connector recovery ended with status=failed,
failed_stage=composer. No implementation review prompt was sent. The initial
plan plus first focused follow-up passed; the second focused code review remains
unused. Do not deploy without that required review. No runtime stop, binary
replacement, live journal edit or command replay was initiated.

A bounded local blocker event is pending because the reporter's ingest token
and identity settings are missing. No hub or Kakao delivery is claimed.
