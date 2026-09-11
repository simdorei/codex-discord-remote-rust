# Stop-identity follow-up: test-first correction

Date: 2026-09-11. Mode: test-first implementation and focused Pro re-review.
Scope: the one remaining R1 finding from
https://chatgpt.com/c/6aa3b2d3-94c8-83ee-88b3-d55698797add .
No production deployment, restart, profile update, shared-Python removal or Git publication.

## Intent contract

An incompatible process termination must not complete an owned process or publish
a passing observation. Missing/ambiguous termination remains pending until the
existing bounded failure; actual Windows name truncation must remain supported.
This is bounded observed-path evidence, not lossless system-wide process tracing.

| ID | Observable contract | Layer / independent oracle | Status |
| --- | --- | --- | --- |
| S1 | Foreign stop without its start leaves the old PID pending; only its real late stop completes it | Actual reducer, fixed event sequence and expected pending identity | RED then GREEN |
| S2 | Reject incompatible/empty/arbitrarily shortened names and a known different parent; accept exact names or the verified ASCII 14-character truncation | Actual reducer, literal positive/negative cases | RED then GREEN |
| S3 | Split start/stop snapshots cannot publish a pass; timeout and clean up only owned subscriptions/events | Public observer with real reducer; only process/WMI boundaries substituted | RED then GREEN |
| S4 | A later correct stop permits one record with that stop, not the earlier foreign stop | Same public observer, second-drain event sequence | RED then GREEN |
| S5 | Keep actual alias/long-name, normal/failing command, delayed event and PID reuse behavior | Existing five Windows observer regressions | Five passed |

## Windows evidence before implementation

Explicit Windows PowerShell 5.1 observation showed `cmd.exe` unchanged and the
long test executable's stop name as `native_process` (14 ASCII characters).
Both stop records had `ParentProcessID=0`, despite an identified parent. Therefore
zero is unknown, not a mismatching parent. Nonzero contradictory parents can be
rejected as additional evidence; parent equality cannot be required unconditionally.

The initial shell probe actually ran PowerShell 7, where Register-WmiEvent is
unavailable. The explicit 5.1 rerun observed the two records above, then its extra
helper `--help` probe exited 2 because that CLI does not accept that invocation.
Neither setup error is counted as a RED regression or a product defect; owned
subscriptions were cleaned in finally. Existing synthetic `worker` stop text
does not represent truncation of the short `worker.exe` name and will be corrected
to the full short name while retaining the original delayed-stop assertion.

## Evidence and outcome

Command (target directory `D:/codex-work/cdr-rust-publication-20260910/target`):

`cargo test --offline -j 2 -p cdr-runtime --test native_process_stop_identity_contract -- --test-threads=1`

Before product changes, all four tests failed for their intended assertions:
foreign stop completed the old PID, incompatible stop was accepted, split queries
published a false pass, and the published record used the foreign stop at 10.
After the minimum observer change, the same four tests passed (zero failures or
ignores). The five existing observer regressions also passed, including actual
native processes and WMI cleanup. Five related checkpoint/source-binding suites
passed 49 tests (2 + 16 + 19 + 6 + 6), zero failures/ignores: total 58 passing tests.

Product change: retain PID-generation/event-order matching, require a compatible
nonempty stop name (exact case-insensitive match or verified ASCII 14-character
prefix), and reject a nonzero contradicting parent. Unknown parent 0 is accepted.
Rejected stops neither remove the candidate nor fill its termination obligation.
No producer retry duration, consumer gate, source-binding rule or launcher changed.

The direct workspace `cargo fmt --all --check` invocation failed before formatting
with Windows `os error 206` (command length). Changed-file formatting passed; the
repository's existing bounded formatting runner and Clippy are checked separately.
The old 894-start observation and 1,902-test run are historical, not certificates
for this newly changed source. Final-source release gate/recovery/live cutover
remain separate from this focused code review. The completed Pro outcome is below.

## Final local verification of this correction

- `scripts/Test-RustFormatting.ps1` passed the existing per-package workspace
  check; both changed Rust files also passed the explicit edition-2024 check.
- `cargo clippy --offline -j 2 --workspace --all-targets -- -D warnings` passed.
- `git diff --check` and strict UTF-8 readback of all three changed code files passed.
  They remain 141, 202 and 166 lines respectively; no new long-file exception.
- The final-source five-workflow observation completed and was saved once to
  `docs/rust-migration/evidence/current-pc-stop-identity-20260911.json`.
  It observed 829 / 8 / 12 / 33 / 9 owned starts (891 total), with zero recognized
  Python starts. These are actual observed counts, not an expectation copied from
  the older 894-start run. Independent dependency-audit checks also passed.
- Saved record contract, current Rust/PowerShell/rollback source binding and
  measured quality passed in PowerShell 5.1.19041.6456 and 7.6.6. Quality covers
  1,234 Rust and 1,358 scoped text files: zero invalid UTF-8, zero Rust BOM, and
  the same 19 approved pre-existing long production Rust files.

Rust source fingerprint:
`E6ACEF20133D16AC3B8BE1CBDAA346C89E9A9315D37CFE3C42F6807871B2339B`.
Rollback-source record fingerprint:
`46AA2AE8EEE942AFA7F70ACF9B40E8FC6E33555E4C8C1666D113016AA1C57126`.

The record remains `current_pc_gate_supplement`, not a full-workspace or production
certificate. Actual production bot, profile, databases and shared Python were not
changed. No production restart, commit or push was performed.

Focused Pro request sent 2026-09-11 08:31 UTC:
https://chatgpt.com/c/6aa3bc43-f468-83ee-8f74-021477f11d00 .
The old conversation's exact-OAuth picker failed; the native skill's one permitted
catalog retry verified normal Chat, 6 Pro and the exact OAuth attachment. Five
Chrome tabs were preserved within the limit. The canonical mapping was saved.
The interactive shell could not launch through the Windows app alias; a non-UI
mapping worker retained the lease only in memory, consumed a public URL-only
handoff file and exited successfully. No lease token or credential was printed
or persisted in that file. Product source has remained frozen during review.

## Completed Pro re-review — CODE PASS

The final answer and response actions were observed at 2026-09-11 08:37:42 UTC,
after 6m 8s of Pro processing. Verdict: **R1 CODE PASS / LIVE HOLD**. No mandatory
residual code fix or additional regression test was requested for this finding.

Pro confirmed the three-stage reducer counterexample is blocked, verified ASCII
truncation and unknown-parent handling are retained, and S3/S4 exercise the real
producer/reducer and reject the actual false record rather than merely asserting
a stubbed `ready` value. Pro kept its source review distinct from Codex's executed
58 tests and did not treat the failed monolithic fmt command or old evidence as
current-source certification. No new accepted recommendation requires more code.

Codex independently checked the final logic against the actual RED/GREEN evidence
and compared Pro's observer SHA-256 with the local file; they match exactly:
`3643D5343D6042703631DA9C1E452EB9097C15C4830B095CD0FA5DAE0B35F9E7`.
The requested correction and re-review are complete. Remaining release gate,
recovery packaging and live cutover are separate from this completed code task;
no production deployment/restart, commit or push was authorized or performed here.
