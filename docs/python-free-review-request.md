# Rust-only migration: independent review request

Review mode: read-only. Do not edit files, run deployment, restart the bot, publish
code, archive threads or send Discord messages. Use the selected project's file
tools to inspect the actual working tree; do not treat this summary as proof.

Update after the initial review on 2026-09-11: see
`docs/python-free-pro-review-20260911.md`. Initial 6 Pro review and the focused follow-up
completed; the user-authorized fresh conversation recovered the old composer's attachment
failure. Follow-up verdict: R2 CODE PASS, R3 CODE REVISE, R1 PLAN PASS, LIVE HOLD. R3 still
needs a same-process PowerShell location fix and a non-Windows test expectation fix;
the Windows case was independently reproduced without writes. Earlier three meaningful
RED-to-GREEN regressions and 19 passing focused checks did not cover those cases. R1
checkpoint evidence alignment remains unimplemented. No final code/deployment approval,
current-source recovery package or production replacement exists.

## Goal and boundaries

The Discord Remote runtime, installation/setup, Pro helpers, MCP server, tests and
recovery must not require Python. Preserve first-prompt/first-reply behavior,
one-to-one mirror mapping, progress/final delivery, steering, archive and settings.
The implementation is in the separate Rust repository. The live original bot,
its config/state and shared system Python must remain untouched before deployment.
No commit or push is authorized.

The user approved removing the fresh-Windows/VM requirement on 2026-09-11. Review
Python independence from deliverable dependencies/call sites and exercised native
install/runtime/Pro/MCP paths on the current PC. Keep shared Python untouched. A
clean-Windows test is optional, not a deployment blocker; do not claim it was run.

## Areas needing close review

1. Native `admin` setup, discovery, inventory, backup, attachment sending and
   exact-job first-reply inspection. Check UTF-8/ACL preservation, no secret output,
   read-only DB paths and no ambiguous replay/delivery authority.
2. Native Pro hook/probe/conversation helpers, installed cache identity, lease
   fencing and the release collector. Ensure missing, stale, zero-test, ignored or
   malformed evidence cannot be called a pass. Browser/Pro live checks are separate.
3. Installer paths and process invocation: existing runtime refusal, Cargo output
   binding, cmd/ps1 argument preservation, output capture and atomic Pro-helper
   staging. Verify failure preserves the previous artifact and original error.
4. Rust-only restart/checkpoint/maintenance: correct process identity, state
   preservation, explicit recovery, no silent fallback and no stop-only deployment.
5. Python deletion closure and fixture independence. Frozen JSON/SQL is historical
   test input, not executable Python. External-user-project `python:test` discovery
   and read-only detection of an old writer are intentionally retained.

Primary code: `crates/cdr-runtime/src/admin`, `crates/cdr-pro/src`, installer/root
launchers, `scripts/CdrInstall*.psm1`, `scripts/RustMigrationCheckpoint.*`, native
maintenance contracts, `tests/browser`, and both CI workflows. The detailed evidence
and explicit pending gates are in `docs/python-free-migration.md`.

## Current evidence and nonclaims

- Latest workspace test phase: 1,889 passed, zero failed, 27 existing ignores (422
  summaries). QA then failed on two test-only Clippy raw-string warnings; the corrected
  workspace Clippy run passed. R2/R3 installer changes and their 19 focused checks came
  afterward. The final build/format/Clippy/Windows/shell rerun explicitly skipped unit
  tests. These are separate results, not a final-source full-workspace certificate.
- Collector follow-up: every requested suite must have a positive unignored result.
  Missing suite summaries, mixed positive/zero runs and explicit failed status cannot
  be classified as passing. Two regression tests failed before this correction.
- No production deployment or restart has occurred during this migration.
- No strict Python-unavailable Windows execution proof is claimed. The user made
  that test optional; this PC retains shared Python. PATH filtering does not prove
  absence of absolute/portable interpreters. Current-PC dependency and runtime checks
  remain required.
- The local cache cleanup blocker is resolved without bypassing the closure assertion.
- A separate Codex profile now has the actual Rust plugin installed and enabled at
  version `0.1.0+codex.20260911034456`. Its installed native helper verified Chrome and
  attached the exact OAuth connector in normal Chat with Pro selected. The production
  profile is still the old version and was not replaced. This is isolated helper QA,
  not proof that the live Discord `!pro` transport was deployed or tested.
- Follow-up defects fixed this turn: canonical probes now receive Chrome's lexical
  bindings (two actual-module Node regressions went RED to GREEN); both installers
  pass the selected Codex profile to every plugin command and reject invalid runtime
  directories instead of silently selecting the default profile (three regressions
  went RED to GREEN). Related groups passed 23 Pro and 14 installer tests separately.

Review priority: verify those fixes in `crates/cdr-pro/src/evidence/browser.rs`,
`connector_transcript.rs`, `install.ps1`, `install.sh` and
`scripts/CdrNativeProcess.psm1`. Also review how to update
`scripts/RustMigrationCheckpoint.EvidenceContract.psm1`: it still requires the old
`python_unavailable_execution` field and zero BOM/over-250-line counts. The user's
current contract permits an existing Korean Windows PowerShell BOM and a cohesive
single-responsibility file over 250 lines; no passing zero counts may be invented.
Recommend the smallest honest current-PC gate/recovery path, without a VM, broad
refactor, active-bot stop, or weakening identity/state/duplicate-send protections.

Return concrete blockers and high-risk regressions with file/line references,
separating code defects from missing execution evidence. Recommend the smallest
safe corrections and tests. Do not approve deployment without the pending gates.
