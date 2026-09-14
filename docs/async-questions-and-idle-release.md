# Async question replies and bot-owned idle release

This update preserves the resident app-server transport and one-to-one thread
mapping. It does not transfer ownership away from an active Codex Desktop writer.

## Async questions

Supported asynchronous question items are persisted with their original thread,
turn, item and question identities. Discord choice controls bind the answer to
one exact question. The selected answer is not inferred for other questions.

Answer dispatch and recovery preserve uncertain outcomes instead of automatically
replaying an input. Outstanding questions, answer inbox entries and queued work
prevent the optional idle-release operation.

## Releasing an idle bot subscription

After a bot-owned turn finishes, the runtime atomically records its final delivery
and an eligible release candidate. A background worker may unsubscribe only with
fresh terminal/idle evidence, an acknowledged observation journal, no outstanding
work and an explicitly absent or completed Goal.

An unsubscribe acknowledgement is not proof that the thread was unloaded or that
an operating-system writer lock was released. Uncertain results remain visible
and block unsafe automatic mutations; they do not authorize request replay.

After a confirmed unsubscribe acknowledgement, a subsequent request must first
complete one durable resubscription. Lost responses or failed persistence do not
authorize a duplicate resume or start. Exact child-exit evidence is retained until
its durable journal succeeds, including when close must be retried.

There are at most 128 unresolved release records. Unresolved records are never
evicted; up to 32 settled records are retained. Diagnostic text is bounded to
512 Unicode characters per record. The existing `!doctor` report includes a
read-only summary of idle-release state.

## Verification and limitations

Automated regression tests cover exact question binding, duplicate protection,
atomic final/candidate persistence, queued-successor cancellation, release and
resubscription uncertainty, actual transport write phases, isolation of unrelated
work, and retrying child-exit journal failure without replaying RPCs.

Before publication, the deployed product-source bytes matched the reviewed
snapshot. Native runtime and app-server library suites passed, as did the selected
runtime integration regressions and store/Discord suites. Code review required
retaining the sealed client until both cleanup and exit-journal persistence
succeeded; the reproduced regression passed after that correction.

Deployment process/hash/heartbeat checks do not establish real Desktop writer
handoff. A genuine user-message and subsequent unload/continuation check are still
required for that claim. No fork, forced Desktop shutdown, lock-file/history
database edit, alternate transport or Python fallback is introduced here.

Machine-specific deployment tickets, credentials, local review transcripts and
generated binaries are not part of this source update. Build and update using the
existing README instructions; no additional dependency is introduced.

## Publication verification

The isolated publication checkout passed the workspace build, all workspace
library tests (498 passed, 8 pre-existing ignored), and 15 focused integration
tests covering question outcomes, idle-release persistence and shared HTTP
fixture consumers. Workspace/all-target Clippy with `-D warnings`, formatting,
and Windows/shell installer dry-runs also passed. Setup dry-runs explicitly used
the temporary QA executable and did not change credentials or register tasks.

The broader `cargo test --workspace --all-targets` attempt did not finish its
link stage because the test machine ran out of disk space (LNK1180). It is not
claimed as a complete pass. Temporary generated artifacts were cleaned up.

Publication checks also corrected test-only lint issues by factoring setup,
sharing one fixture module, using explicit imports and checked conversions.
No assertions, timeouts or lint rules were weakened. The 61 reviewed non-test
source files still matched their accepted SHA-256 hashes. These test-only
cleanups do not change the deployed bot's runtime logic.
