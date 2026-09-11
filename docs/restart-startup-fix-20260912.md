# Startup and deployment failure contract

Mode: test-first implementation, limited to startup budgets and maintenance diagnostics.

The requested outcome is to apply the new runtime without duplicate bot processes,
while retaining the running bot if pre-stop checks fail. No queue repair, silent
transport fallback, forced shutdown, unrelated cleanup, or memory-test recertification.

| ID | Contract and independent oracle | Layer | Evidence |
| --- | --- | --- | --- |
| INIT-1 | A fixture responding to initialize after 12 seconds becomes healthy and answers an echo. | App-server integration | RED: initialize timed out after 8000 ms. GREEN: same fixture connects, echoes, and closes. |
| INIT-2 | A fixture never answering initialize fails with the real timeout within 35 seconds and its owned child is closed. | App-server integration | GREEN: exact 30000 ms timeout and child EOF witness; no indefinite wait. |
| INIT-3 | The initialization budget is 30 seconds; enclosing startup budgets allow 45 seconds, and the maintenance command reserves startup, wait, close and scheduling headroom. | Wiring and maintenance integration | RED: 30-second native deadline. GREEN: 120-second command cap, wait budget at most 60 seconds, insufficient remaining budget starts no child; shared 45-second Rust startup wiring reviewed. |
| ERR-1 | A nonzero native preflight exit persists its stderr reason and exit code in the operation state, with no drain, stop, install or launch. | Maintenance integration | RED: original stderr reason missing from saved state. GREEN: same native failure retains reason and exit code, with zero deployment actions. |
| ERR-2 | UTF-8 diagnostics survive; configured test secrets never appear in console or state; saved error size remains bounded. | Maintenance integration | GREEN: reason, synthetic credentials (including equal-length values), 100 KB output, and Korean variants. Redaction-source failure also explicitly withholds raw text. |
| LIVE-1 | Verified candidate hash, one new process, old process absent, increasing fresh heartbeats, and an actual Discord completion receipt. | Live deployment | Pending independent post-turn maintenance; read-only startup and all 25 mapped/managed thread reads passed beforehand. |

The previous scheduled operation is halted before any shutdown or installation.
Its evidence must be archived intact before a separately owned replacement operation
is armed. A scheduled task or a copied candidate alone is not deployment completion.

## Verification on 2026-09-12

- `cargo test --workspace --all-targets --locked --offline -- --test-threads=2`:
  1911 passed, 0 failed, 28 ignored across 427 suites. Ignored child fixtures and
  pre-existing opt-in tests are not claimed as independently run live tests.
- Per-package formatting check, `git diff --check`, and workspace/all-targets
  Clippy with `-D warnings`: passed. Workspace-wide formatting exceeds the Windows
  argument limit, so the repository's existing per-package checker was used.
- Runtime release build: passed. Personalized update operator: 5 tests passed
  and release build passed; its cleanup path does not modify databases or Discord.
- PowerShell and Git Bash install/setup dry runs: passed; no live install implied.
- New runtime and operator connected to the actual local app-server. Preflight
  refused only the still-running controlling request, as required before shutdown.
- Read-only inspection: 25 thread reads succeeded; the diagnostic app-server
  closed normally. This is not a new-user-request delivery test.
- Independent observer's Discord response-shape contracts: 8 passed; the known
  bot-authored reply was read back exactly once before arming.

Candidate runtime SHA-256:
`6795CEF54E26D07D398EADD5E2B4AF47828EB25B02AAF72D47E81672A6E5F148`

Personalized operator SHA-256:
`0FA8038C4339CFC91B28B5EA9AC4C1045F0A2D7405BA63FDE324B555E86BE021`

The existing long-duration memory eligibility result is not changed or recertified
by this scoped startup/deployment repair. Actual runtime replacement and notice
delivery must be confirmed by the operation receipt and independent observer.
