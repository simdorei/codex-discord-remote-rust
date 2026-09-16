# Mirror cleanup ingress lifecycle fix

Status: ChatGPT Pro PLAN PASS; test-first implementation in progress.
Base: e401968 (isolated worktree; unrelated Issue 1 changes excluded).
Deployment scope: sim-pc-5060, existing Rust bridge only.

## Intent and boundaries

Archived Codex threads with no unfinished work must have their obsolete Discord
rooms removed by mirror sync. A historical prompt handoff is not active ingress.
Real pending work, unconfirmed delivery, uncertain execution, mapping changes,
and concurrent admission remain protected. Never erase journal records to unlock
cleanup, replay uncertain commands, close desktop Codex, or fork conversations.

Known cleanup refusal must produce one accurate error response, durably recorded
before delivery. It is not sync success and does not prove that earlier rooms in
the same sync were unchanged. A notification failure must remain distinguishable
from uncertainty about execution.

## Evidence

5060 room 1543282322131124247 maps to an archived source thread. The queue,
intake, final/commentary outboxes, live busy choices and goal progress were empty.
Four ingress rows are owned/result_recorded, with a prompt owner and confirmation
delivered. Each has a settled completion/v1 receipt matching its owner job.

The cleanup predicate nevertheless protects every state other than completed.
The ingress lifecycle intentionally retains owned after handoff/confirmation.
The archive admission check already treats owned+confirmed as handed off.

The failed sync command message:1548613080555061270 came from another room and
was left held/processing without an outcome by generic message custody teardown.
The secondary warning is therefore attached to the sync command, not evidence
of a new lost model turn.

## Proposed implementation

1. In the common transactional cleanup predicate, exclude valid confirmed prompt
   owners from unfinished ingress. Require a nonempty owner identity and correct
   owner kind; malformed, unknown, held, executing and unconfirmed rows stay
   protected. Retain every independent queue, intake, outbox, choice, goal and
   receipt guard and the cleanup/admission fence.
2. Introduce a typed pre-deletion protection refusal at both the read-only check
   and the final transactional begin() guard, preserving the actual room ID.
   Persist a known stopped
   outcome at message and slash execution boundaries before sending the error.
   Store sync_completed=false, delete_dispatched=false for the affected room and
   earlier_changes_possible=true; confirm only after successful delivery.
   Database, transport, uncertain deletion and other errors keep their current
   conservative treatment. No string matching or automatic command replay.
3. Preserve known stopped evidence through delivery failure and restart; do not
   label it successful or execution-unknown merely because notification failed.
4. Keep the existing held historical sync record for audit. No bulk reconciliation
   or hidden retry is part of this patch.

## Requirement-to-test matrix

| ID | Contract | Layer / independent oracle |
| --- | --- | --- |
| MC-1 | Settled owned history does not block cleanup | Real SQLite public lifecycle; pending_reason=None and begin succeeds without journal mutation |
| MC-2 | Pending/uncertain/unconfirmed work remains protected | SQLite table-driven lifecycle and independent guard fixtures; begin refused, no fence/deletion |
| MC-3 | Uncertain delete never becomes a definite refusal | Existing mirror cleanup race/cancellation/lost-response contracts |
| MC-4 | Same target via another room is protected; actual blocked room retained | Existing and new SQLite and synchronizer contracts |
| MC-5 | Both precheck and transactional refusal are typed; partial changes retained | Store begin and missing-room race after earlier rename |
| MC-6 | Known protection refusal has one durable error response | Message/slash worker integration with HTTP boundary substituted; persisted outcome and send count |
| MC-7 | Notification delivery and confirmation failures stay distinct | Receipt and ingress-confirm fault injection; no second-key ERROR |
| MC-8 | Notification failure/cancellation/restart preserves exact outcome | Durable state/receipt tests at pre/post notification boundaries; no command replay |
| MC-9 | Runners shows refusal and notification state accurately | Authorized read-only summary/detail tests; no payload mutation |
| MC-10 | Database/fence failures remain failures, never known refusals | Query and existing-fence fault tests; no delete/fake evidence |
| MC-11 | Reviewed binary is the only live 5060 bot and cleanup works | Hash/identity-bound update receipt, two heartbeats, real sync inventory |

## Verification and release gate

Pro plan review -> focused RED -> minimal fix -> same focused GREEN -> relevant
store/runtime/regression/native checks -> Pro implementation review -> stage and
hash-check on 5060 -> supported normal maintenance update -> runtime verification.

Use the repository's existing tests. No weakened assertions, skipped failing
tests, or replacement framework. Record test commands and exact results.
No unit result alone establishes live Discord correctness.

Deployment must preserve source/DB backups and pending requests. Preflight must
not stop a healthy bot on failure. A transport interruption is not permission to
retry a possibly completed update; inspect its durable receipt and live identity.
If a new decision or external input is necessary, stop before mutations and
report the exact blocker, retaining the old runtime where possible.
