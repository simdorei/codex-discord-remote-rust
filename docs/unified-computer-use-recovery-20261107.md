# Unified Computer Use recovery fix — 2026-11-07

## Scope and observed failures

Base: 2f494328a102fe8c74064c8d2ce8645bb27f77b7, the clean source embedded in the running 5060 bridge binary. Work was isolated from the unrelated, dirty live checkout.

The bridge recognized only the legacy node_repl server for persisted local-tool approvals and targeted repair. The installed unified plugin exposes cua_repl. Recovery also restarted the bridge before the desktop refreshed native-pipe configuration, and did not identify an old Store package process after the registered package changed.

## Changes

- Apply the existing saved-grant/risk/input rules to exactly node_repl and cua_repl. No new grant is fabricated.
- Discover the unified server across inventory pages, prefer it, and use its documented getState initializer plus completion summary.
- Treat browser/native inventory errors as failed recovery; a successful inventory does not certify window reading or clicking.
- Keep exact thread, request, generation, process creation, and executable ownership checks.
- Recognize a still-running prior version of the same desktop package; relaunch the currently registered package.
- Start the desktop independently first and wait for its backend and all selected native-pipe endpoints before restarting the bridge.

## Verification

Regression failures were observed before implementation for unified saved-grant handling, unified repair discovery, and desktop-before-bot startup.

Final focused checks:
- 22 local-tool approval tests passed.
- 33 repair-action tests passed.
- 4 app-server repair dispatch tests passed.
- 16 recovery host/receipt/writer integration tests passed.
- Test-CdrToolsRecovery.ps1 passed, including isolated real named-pipe readiness checks.
- Workspace formatting check passed (11 packages, 564 roots, 75 batches); changed Rust files were formatted again after final refactoring.
- cargo clippy --locked -p cdr-runtime -p cdr-app-server --lib -- -D warnings passed.
- git diff --check passed.

This is focused local verification, not a claim that repository CI or the full test suite ran.

## Remaining live verification

The existing Windows native helper separately rejected Chrome with:
could not determine the current browser URL on Windows with enough confidence to enforce policy

The external bridge also lacked browser-host session metadata and getState reported that the browser access host was unavailable. This patch reports that failure accurately; it does not copy another session's identity, disable native URL checks, or claim that the vendor helper is repaired.

At review time the registered desktop was 26.1103.3517.0 while the running desktop and bridge app-server still referenced 26.1029.7803.0. The bridge CODEX_EXE configuration pointed to that removed package.

The desktop was then restarted into 26.1103.3517.0 without stopping the active bridge. Its new CUA runtime and both native-pipe configurations became ready after the old 35-second deadline, so recovery now waits up to 120 seconds while continuing to verify readiness. The live executable setting was updated and its launch-integrity binding was revalidated without changing capability policy or the database. Bridge deployment and an actual Chrome read/click must still be verified separately.

## Online backup contention discovered during deployment

The first prepared rollout stopped before changing the live runtime because its online snapshot returned SQLite `database is locked`. A direct retry reproduced the failure. The rusqlite convenience backup method stops at the first recoverable Busy/Locked step. Retrying alone also timed out against the live rollback-journal database because ongoing commits restarted the copy. The store now establishes one read transaction, then uses the incremental backup API with bounded Busy/Locked retries. Persistent contention still returns an error, and the existing snapshot wrapper removes the incomplete copy. It does not cancel another writer's transaction; in rollback-journal mode, that writer can briefly wait for the read copy to finish.

Three regression tests cover temporary contention, an unreleased writer with a bounded failure, and a 24 MiB database receiving continuous committed writes. The sustained-write test failed with retries alone and passed once the read snapshot was retained. Existing snapshot and legacy-store regression suites also passed (10 focused backup/store tests total). The updated debug executable created and validated a snapshot of the actual running bridge database in 3.77 seconds; no runtime stop was needed.

The branch was rebased onto main 0c664730af2eea843072842990259a847af62711. The 75 bridge/recovery tests were rerun along with 19 Codex selection, startup receipt, probe and tool-inventory tests (94 passed, one fixture-only test ignored). Together with the backup/store checks, 104 focused tests passed. Changed-file formatting and library Clippy checks passed. A broader store all-targets Clippy run encountered the pre-existing duplicate_mod diagnostic for tests/support/legacy_schema.rs in the unchanged async_questions test target; this is not a full-workspace lint pass. Current main's manual-only CI setting is preserved.

## Browser host environment forwarding — 2026-11-08

Windows app-server spawning previously removed `BROWSER_USE_ACCESS_VERIFYING_IDENTITY`
even when its caller supplied a complete browser host connection. The pipe and
executor survived, producing an incomplete connection. A real child-process
regression reproduced the missing verifying identity before the change.

The Windows spawn path now treats these three fields as one set:

- `BROWSER_USE_ACCESS_PIPE_PATH`
- `BROWSER_USE_ACCESS_EXECUTOR_ID`
- `BROWSER_USE_ACCESS_VERIFYING_IDENTITY`

A complete, nonempty set from the launch environment is forwarded unchanged.
Any explicit host override selects a new source for the entire set; missing
fields are not borrowed from the inherited host. Windows case aliases are
normalized, and conflicting aliases are treated as incomplete. Incomplete
sets do not supply a pipe/key; they retain an executor variable, including an
empty sentinel, so the browser service still checks access and reports the
missing host. No connection metadata is invented when none was supplied.

This only forwards host connection metadata. The existing browser service must
still verify the signed response for the current executor, session and turn.
It does not inherit parent Codex thread/session IDs, read another process's
identity into the child, create an authorization host, or grant browser access.

Validation: 9 new isolated Windows process tests, 4 existing environment/tool
tests, and 126 app-server library tests passed (139 total; 8 fixture-only entries
ignored). Focused library/test Clippy and `git diff --check` passed.

Live limitation: read-only inspection found all three host variables absent
from both the running 5060 bridge and its app-server. The installed unified
plugin supplies only an empty executor sentinel. This patch therefore cannot
restore that installation by itself. It remains a source-level candidate;
the live executable was not replaced or restarted for this change. A supported
host-provisioning/session attachment path is still required, and the separate
native Chrome URL-confidence failure remains unverified.
