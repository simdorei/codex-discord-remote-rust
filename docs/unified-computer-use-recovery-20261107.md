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

At review time the registered desktop was 26.1103.3517.0 while the running desktop and bridge app-server still referenced 26.1029.7803.0. The bridge CODEX_EXE configuration pointed to that removed package. Deployment and an actual Chrome read/click must be verified separately.

