# Busy rejection ownership after restart

The TS/Rust comparison on 2026-10-20 identified a Rust recovery gap. A confirmed
preflight rejection with `control_dispatched: false` could become
`held/result_recorded` after restart and be selected as the parent of a new Busy
interaction. Queue acceptance also reassigned the old rejection to the accepted
prompt, so its missing confirmation could block the new prompt's final delivery.

Both parent selection and `record_busy_owner` now exclude exactly an unowned
preflight rejection whose result contains JSON boolean `false` and whose state
is `completed` or `held/result_recorded`. This matches the TS guard. The old
outcome and unconfirmed notification remain available; no receipt is fabricated
and no original request is replayed. Dispatched, uncertain, incorrectly typed,
or already-owned outcomes retain their confirmation barriers.

Validation on Windows:

- The two new recovery/ownership reproductions failed before the change.
- Busy choice retention, exact rejection proof, queue acceptance, route checks
  and uncertain-result preservation: 18 tests passed.
- Runtime Busy canonical retry, lifecycle and restart recovery: 4 tests passed.
- `cargo clippy -p cdr-store --test busy_control_retry_contract -- -D warnings`
  passed.

This source change does not repair historical DB rows by deleting them or
replaying their requests. Deployment state is separate from this source test
record.
