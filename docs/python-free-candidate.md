# Rust-only local candidate (not approved or deployed)

Updated 2026-09-11. Repository HEAD remains
`3b359b470fee412bf010825c0e64a6139ab03b3c`; changes are uncommitted.
This record is diagnostic, not a schema-2 workspace certificate or a recovery package.

Command: `cargo build --workspace --release --locked --offline -j 2`.
Latest release build exited 0 in 2m 37s. The build directory is external to both the
source tree and the production bot. The runtime and helper were staged in the Rust
repository and installed into an isolated Codex profile; no production artifact,
profile, config, database or process was replaced.

| Candidate | Bytes | SHA-256 |
| --- | ---: | --- |
| cdr-runtime.exe | 32757248 | 82F8E0C856460DE358A45D5EA2AD784AE441502D05EB54B5955F921E321E3D24 |
| cdr-offline-soak.exe | 5842944 | 98EC620B519A681EB3A05614B88CB2E1103B828EC06AA5BB238CF323902A3FCC |
| cdr-mcp-server.exe | 12153344 | F790EDCA6ED9FB77C2A2FD67DD427AFDC615591A49F80424768053AF5098E36E |
| cdr-pro-helper.exe | 5515264 | 9CC9B6BE4EC1BFFDFD09EAC5B52FDA99FF4B837436D60994BAA79AC2CF49D207 |

The latest workspace test phase passed 1,889 tests, with zero failures and 27 existing
ignores (422 summaries). The enclosing QA command failed in Clippy because the new
lexical-probe test had two unnecessary raw-string delimiters. Both were corrected;
workspace Clippy then exited 0. Installer review regressions were added afterward, so
this is not a final-source full-workspace certificate. Do not sum overlapping runs.

A bounded current-PC observation recorded 88 owned process starts, zero recognized
Python interpreter starts, and six successful command steps: observer canary, actual
isolated installation, native setup dry-run, native helper verification, 26 start/
restart/configuration fixture tests and three real local Rust MCP integration tests.
Shared Python remains installed. This does not prove absence of renamed interpreters,
unexercised paths or completion of the production restart. The initial live Chrome/
OAuth/6 Pro check is separate from the process-tree observation.

## Required before promotion

1. Cache cleanup is resolved: the user removed only the Rust repository `__pycache__`;
   absence and passing closure tests were verified. Shared Python and the original
   repository were not deleted by this task.
2. On this PC, audit Python dependencies/call sites and observe native install, start,
   restart and Pro/MCP checks. The user approved dropping the fresh-Windows/VM gate
   on 2026-09-11. Shared Python stays installed; clean-Windows proof is not claimed.
3. Initial Chrome/OAuth/6 Pro review succeeded with the actual isolated native helper.
   It returned CODE REVISE / verification REVISE / LIVE HOLD. The required focused
   follow-up was not sent: both the trusted primary and its one permitted retry returned
   `connector_picker_unavailable` in the modern follow-up composer. Preserve the mapped
   review and do not bypass attachment checks or claim final Pro approval.
4. Resolve the documented quality-evidence/exception records, run frozen-source
   native gates and generate actual schema-2 evidence; no passed fields are prefilled.
5. Build and verify a current-source Rust-only recovery package, then perform the
   scoped production cutover and verify identity, heartbeat and real Discord replies.

The original bot was last verified running as PID 49728 with a fresh heartbeat,
no stop marker and no restart marker. No commit, push or production restart occurred.

Review findings, accepted corrections and precise remaining work are recorded in
`docs/python-free-pro-review-20260911.md`.
