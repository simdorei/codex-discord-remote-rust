# Mirror cleanup and list — implementation plan and pre-implementation review

Date: 2026-09-17 (Asia/Seoul)
Device: sim-pc-5060
Root: C:\repos\simdorei\codex-discord-remote-rust
Status: PLAN PASS after the counterexamples below; implementation and code verdict recorded separately.

## Authorization and boundaries

The user requested two separate fixes, plan review, implementation, tests, and code review. This change does not deploy/restart the running bot, call real Discord, access production DB/config/secrets, delete room 1545155901659283497, change archived Codex originals, resend an input, commit, or push. Existing dirty changes are preserved; capture a per-file baseline before editing. Tests use temporary SQLite databases and fake transports.

## A. Archived mirror cleanup blocked by known non-dispatch outcomes

Observed implementation: room_cleanup::pending treats held ingress as unfinished even when the durable outcome proves busy_control_preflight_rejected/control_dispatched=false. Existing assessment docs/mirror-room-1545155901659283497-cleanup-assessment-20260917.md reports two such expired Steer interactions. This is prior evidence, not a new production DB inspection.

Implement an archived-only path; do NOT relax ordinary pending_reason/begin, orphan cleanup, or exact-absent cleanup. Candidate exclusions must match the exact room AND target, kind=interaction, state=held, phase=result_recorded, confirmation_delivered=0, no owner_kind/owner_id, a nonempty canonical busy-choice identity, no canonical ownership receipt, a structurally valid frozen BusyChoice matching actor/channel/target/canonical identity, busy_action=steer, expired finite choice, and exact typed outcome kind plus JSON boolean false. Malformed/NULL/string/number proof never qualifies. No successful notification is inferred.

Use the same BEGIN IMMEDIATE transaction for exact mapping verification, re-reading exclusion evidence, all independent pending guards, an append-only evidence snapshot, and durable deleting fence. No expired timestamp or archived flag alone authorizes deletion. Check queued work, intake, final/progress/goal outbox, live busy choices, unknown receipts and unattributable receipts independently. An unrelated ingress on the same target still blocks.

Before dispatch, pin exact Discord ID/guild/parent/thread type and recheck the source still has the same archived identity and archive timestamp and a preserved rollout. Validate the archive again inside the begin callback after Discord lookup; no async gap is added between that check and the guarded dispatch. Different DBs cannot provide an atomic cross-application archive transaction: source restoration after the last check is not claimed impossible; original Codex data is never modified.

Keep the original ingress rows unchanged; store exact payload/outcome and a metadata snapshot bound to the close token in an append-only audit table. Preserve notification uncertainty. Recovery must recognize that exact audited/fenced disposition and not create a new notification in a closed room. This is not permission to execute/replay or reset a claim. If a delete is rejected, release only that token; if unknown, retain fence/mapping/evidence and never automatically re-DELETE. Retire the exact mapping only after confirmed deletion (or an authoritative missing room observation).

### A plan review counterexamples and required tests

1. Blanket ignoring control_dispatched=false would allow a wrong kind, malformed bool, wrong target or different actor. Require full typed proof and exact frozen identity.
2. A read-only precheck can become stale. Repeat candidate/pending/mapping checks in the write transaction; test late ingress and proof mutation during slow channel lookup.
3. A room could be reparented or a source restored. Pin parent/ID and recheck source after lookup; test no deletion.
4. Preserving held rows without recovery handling could re-stage a notice after deletion. Audit-bound recovery skips that original only; test original bytes/fields unchanged, no new outbox.
5. Audit persistence failure must roll back the fence and prevent DELETE. Test SQLite ABORT.
6. Lost DELETE response or failure to persist completion cannot remove mapping or retry. Test one dispatch and retained evidence/fence.
7. Live queue, delivery, goal, busy choices, shared room, schema/read errors remain blocking. Existing guards/tests remain enabled.

## B. List visibility vs execution ownership

Direct code findings:
- Plain !list / slash list uses load_recent_threads(0), NOT thread/list or thread/loaded/list. Runtime thread/read states annotate entries and do not filter rows. Do not claim a writer-ownership omission was reproduced here.
- Default mirror sync and mirror inspection use load_user_root_threads(), whose SQL admits only source='vscode'. CLI and app-server user roots without a mapping can be omitted.

Add a separately named mirror-root query accepting known interactive local sources vscode/cli/app-server/appServer and user/legacy-empty root provenance. Exclude archived and spawned/internal roots; retain existing mapped active forks independently. Preserve the legacy vscode-only reader rather than silently changing unrelated callers. Use the new query consistently for default sync and mirror inspection.

Plain !list keeps the complete local DB inventory and stable numbering/reference resolution. Add explicit displayed/total count, local configured CODEX_HOME/DB scope, active-vs-archived scope, and a notice that runtime observations are not execution ownership verification; other PCs/homes are not silently merged. Mirror list/check shows missing mappings with identifiable metadata and the same source scope. Never resume/steer/stop to populate a list or infer global idle from the bot's observation.

Official protocol cross-check (accessed 2026-09-17): https://developers.openai.com/codex/app-server/ — thread/read is read-only without resuming; stored thread/list differs from loaded list; default thread/list sources are cli/vscode and archived=false. The bot's direct-SQL inventory remains distinct from that API's defaults.

### B plan review counterexamples and required tests

1. Broaden source selection without excluding subagent roots => accidental mirroring. Test roots across sources, internal roots excluded, existing mapped forks retained.
2. Treat a failed/notLoaded observation as no thread => hidden records. Test row retention for error/notLoaded/no-server and beyond probe limit, with no resume/start RPC.
3. Change displayed numbering using a filtered list => !use wrong target. Preserve full local list ordering and reference mapping; tests assert displayed/total and IDs.
4. Listing must remain read-only: no implicit schema initialization, sync, claim, mapping creation or writer takeover. Existing prefix/slash connected contracts stay active.

## Validation and final review

Create failing regression(s) on the pre-change source, then apply the narrow changes. Run affected cdr-store/cdr-codex-state packages, mirror-sync and connected list tests, existing cleanup/race/inspection/recovery contracts, fmt, and strict workspace all-target Clippy where feasible. Report exact executed totals and exclusions; do not recycle revision17 totals. Review production call wiring, transactional rollback, uncertainty, source restoration, false proof and writer separation independently of happy-path tests. Record any remaining limitation explicitly. Plan review here is same-session adversarial self-review, not a separate external reviewer.

## Final plan-review disposition

PLAN PASS retained after implementation review. The archived begin transaction additionally checks the exact parent mapping. The mirror inspection formatter uses a checked lookup: a root created between independent inventory reads remains listed with recheck-needed status instead of panicking. Ordinary cleanup/absent-source/orphan paths remain conservative.

Implementation and same-session adversarial review completed: docs/mirror-cleanup-and-list-review-20260917.md. Final selected regression: 772 passed, 0 failed, 6 existing ignored. Runtime inventory coverage 430/430, no missing/duplicate test identities. Source checksum check 1,315/1,315. Workspace strict all-target Clippy passed; all 22 changed Rust files passed formatting checks. Whole-tree formatting is NOT green: cargo fmt hit Windows os error 206 and batched read-only checks identified 13 out-of-scope pre-existing formatting differences, left untouched. Four initial parallel completion tests hit their two-second wall-clock timeout; unchanged tests passed in the final isolated/low-parallel run. Full logs retained; no assertion or deadline relaxed.

No deployment, live room deletion, production DB mutation, restart or user-request replay was performed. Cross-PC/other-CODEX_HOME aggregation and active-writer takeover are not implemented. The narrow code verdict is not an already-completed production rollout or live smoke verdict.