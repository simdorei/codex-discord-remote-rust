# Independent local tools verification - 2026-10-13

## Scope and decision

The scoped independent Computer Use and Chrome integration checks passed with
the ChatGPT desktop application closed. The isolated commit candidate passed
its focused regression and static checks. This is not a whole-worktree or
whole-product certification.

- Candidate base: `3ebf4223b75b9d36237a73a78ce52ce794aee4ca` (remote main).
- Candidate branch: `fix/independent-local-tools-20261013`.
- Five production files and three test files are included; unrelated dirty
  changes, operational databases, credentials, backups and raw logs are not.
- The source hashes and exact commands are in
  `independent-local-tools-manifest-20261013.json`.
- Seven scoped source files are byte-identical to their deployed-tree versions.
  `client.rs` retains main's existing formatting; formatting both versions to
  stdout produced identical text. Neither deployed source was reformatted.

## Behavior

- `AppServerConfig::new` passes the independent native-tool option on Windows.
- Initialization advertises `mcpServerOpenaiFormElicitation`.
- Windows child environment inheritance excludes desktop-owned pipe and
  session identity variables. Explicit environment overrides remain supported.
- The server-request worker verifies the original request's thread, turn,
  queue ownership, mapping and resident generation before local-tool handling.
- Only node_repl, low-risk, empty-schema form requests for the computer-use or
  browser connector receive the scoped acceptance response. Other form shapes
  are cancelled; other request methods retain their existing handling.
- Responses use the existing exact occurrence/generation authority check.
  This does not create persistent grants or retry user requests.

## Fresh candidate QA

These are the new isolated-candidate runs, not a second count of the earlier
working-tree results.

| Scope | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| App-server library and two integration targets | 131 | 0 | 8 |
| Local-tool policy/worker tests | 5 | 0 | 0 |
| Seven approval/recovery/component integration targets | 35 | 0 | 0 |
| Total | 171 | 0 | 8 |

The ignored entries are excluded from the pass count. The entire workspace
test suite was not run.

- Offline native test-fixture build: exit 0.
- Production library/binary strict Clippy: exit 0.
- New app-server integration target strict Clippy: exit 0.
- Rustfmt check on the eight scoped source files: exit 0.
- Patch whitespace check: exit 0.
- Cargo used a shared target cache, without modifying the production binary.

### Exact Cargo commands

- `cargo test --offline --locked -p cdr-app-server --lib --test independent_tools_contract --test startup_budget_contract` (exit 0).
- `cargo build --offline --locked -p cdr-runtime --bin cdr-offline-soak` (exit 0).
- `cargo test --offline --locked -p cdr-runtime --lib server_request_worker::local_tools_tests -- --nocapture` (exit 0).
- `cargo test --offline --locked -p cdr-runtime --test approval_actor_contract --test server_request_recovery_contract --test server_prompt_contract --test component_reply_contract --test bound_component_selection_contract --test approval_text_contract --test server_request_occurrence_identity_contract` (exit 0).
- `cargo clippy --offline --locked -p cdr-app-server -p cdr-runtime --lib --bin cdr-runtime --bin cdr-offline-soak -- -D warnings` (exit 0).
- `cargo clippy --offline --locked -p cdr-app-server --test independent_tools_contract -- -D warnings` (exit 0).

### Other checks

`rustfmt --check --edition 2024 --config skip_children=true <eight manifest source paths>`

`git diff --check`

The private command logs are identified by SHA-256 in the manifest. They are
retained locally rather than publishing machine-specific raw output.

## Live app-server-only evidence

- New runtime start: 2026-10-13T06:40:57.8661283Z.
- Runtime PID 30900 and its direct app-server child PID 31372 remained unchanged
  throughout reconciliation.
- The child command includes the independent native-tool option.
- Installed runtime SHA-256:
  `b5f3ee049f6eefb82859cafb88bcb3b491a7aa4c02e9f937df474c01f3c3df99`.
- ChatGPT process count was zero and ChatGPT Watchdog remained disabled.
- The live heartbeat advanced.
- Computer Use clicked the dedicated test Notepad document and entered new
  text, verified from a fresh screenshot. The accessibility tree was null;
  this proves screenshot/input operation, not native accessibility support.
- Chrome extension control created one disposable tab, read Example Domain,
  clicked its observed link, and confirmed the IANA destination. The extension
  reported the profile label `hyunae`; a separate account-alias check was not
  performed.
- The test tab was closed. All four pre-existing user tabs were preserved.
- Two confirmed completion-delivery receipts postdate the runtime restart.
  The latest observed delivery was 2026-10-13T07:00:28.228000+00:00.
- The original final receipt, cancellation/hold evidence and stopped queue
  rows were preserved. The historical recovery audit remained present and
  its original attempt was not prepared for replay.

The live artifact was built from the operational working tree; the isolated
Git candidate was separately tested. This report does not equate every
unrelated operational-tree file with the published candidate.

## Deployment-helper reconciliation

The legacy handoff receipt remains FAILED at its final verification step.
Its verifier waited 120 seconds for `discord_ready` in an empty stdout log.
That log condition did not establish whether the new runtime or tools worked.
The old failure has not been rewritten as a successful script run.

The later independent check verified the exact installed binary and process
identities, advancing heartbeat, preserved durable evidence, new Discord
delivery receipts and actual tool actions. Its private proof SHA-256 is:

`cd4c02f3748c149b7698030d2e4a7b5893a9906bf9fbf4ed8f7e6d4ea10c14b9`

The one-shot wrapper had already exited when cleanup was authorized; the
runtime and app-server survived with their original new PIDs. The task's
25-minute limit was changed to unlimited and the completed one-shot task was
disabled to prevent accidental reuse. No live bot or app-server was stopped
or restarted during reconciliation. No production database was written and
no request was replayed.

## Limits

- No external Pro review is claimed.
- Real tool operations succeeded, but a distinct live auto-accept log event
  was not independently observed. The acceptance and refusal boundaries were
  exercised by the native fixture tests.
- The obsolete helper's stdout capture behavior was not repaired in the
  production launcher. Future deployments must not reuse that one-shot
  log-marker verifier as a readiness oracle.
- This change does not install a new desktop application or replace Chrome
  with a CDP/debug-port integration.
