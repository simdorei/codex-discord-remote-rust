# Mirror cleanup and list deployment — 2026-09-17

User authorized deployment of the previously approved mirror cleanup/list source and commit/push. Tray changes are a separate, NOT DEPLOYED work item.

## Verified result

- Device/root: sim-pc-5060 / C:\repos\simdorei\codex-discord-remote-rust.
- Status: DEPLOYED_VERIFIED, live-handshake-v1 / runtime-proof-v1.
- Runtime started: 2026-09-17 06:11:42 KST; verification: 06:13:16 KST.
- New PID: 28860; old exact PID/start identity 29496 exited; runtime instance count 1.
- Candidate, installed executable, launch journal, completion identity, fresh advancing heartbeats match.
- Pre-stop and post-stop SQLite snapshots and baseline executable backup verified by SHA-256.
- Operational .env hash unchanged; no secrets displayed or committed.
- No maintenance/disable/stop/restart/drain/cutover marker remains.
- Build: cargo build --release --locked --offline --target-dir target/mirror-deploy-20260917 -p cdr-runtime --bin cdr-runtime, exit 0.
- Configuration check, existing pinned update-only operator preflight, registration ValidateOnly, final deployment verification: exit 0.
- Reviewed source inventory 1,315/1,315 and 22 change hashes match; build before/after match.
- No real-user prompt, manual room delete, failed input replay, or tray launch was requested by this deployment.

## Connection interruption and notification limitation

The initial registration/worker MCP call disconnected. It was NOT replayed. After 5060 reappeared, the lost selection was rebound to the same PC/root. Registered operation, completed receipt, installed artifact, process identity, backups, environment and advancing heartbeats were reconciled. The parent worker exit receipt is absent, so success is based on durable completion plus a separate exit-0 verification, NOT an assumed initial command exit code.

Discord completion notification is rejected: HTTP 403 / code 40333, no receipt. This does not undo runtime-proof completion. No repeated POST, stop or redeploy was performed for that rejection. Actual model inference, all Discord sends and actual archived-room cleanup are not covered by the deployment smoke.

## Reproducible records

Local evidence: target/qa-source/mirror-deploy-20260917/ (candidate-build, source-before/after-build, deployment-request, registered-operation, completion-receipt, deployment-final-verification, stdout/stderr and verification script).
Portable source hashes: docs/mirror-cleanup-and-list-source-manifest-20260917.json.
The original plan/review/manifest remain unchanged. Prior 772 passed / 0 failed / 6 ignored is the reviewed regression evidence, not newly rerun tests during this deployment.
Whole-workspace format qualification in the original review is retained.

Git scope: reviewed cumulative Rust source needed by Reserve/inheritance/cleanup/list, the already-deployed maintenance notification channel pins, and selected review/deployment records. Unrelated async-question probe/docs, release build directories, live DB/config/logs/backups are not staged.

Execution receipts: build tr_6237a1c07bd149d0; preflight tr_34fc2881522943ec; post-reconnect observation tr_a35556efc6a941ba; final verification tr_51db58815b5440db.
## Remote branch qualification

origin/main advanced separately to 8b68180 (async questions / idle release, 2 commits). This deployment is NOT a merge of that unreviewed combination. Publish the approved snapshot on release/mirror-cleanup-list-20260917; no force push or main rewrite. The local source snapshot also lists the unrelated question_transport_probe.rs example, deliberately not committed; it is not linked into the deployed runtime.
