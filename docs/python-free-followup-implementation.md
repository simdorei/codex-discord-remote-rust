# Approved follow-up implementation contract (2026-09-11)

Mode: test-first implementation in the separate Rust repository. Keep the live bot,
shared Python, configuration and state untouched. No deployment, commit or push in
this implementation step. The focused Pro plan and remaining findings are recorded
in `python-free-pro-review-20260911.md`.

Current handoff: R1/R3 code and focused verification are implemented. The final
current-PC supplement is verified against current source in both PowerShell 5.1
and 7. Final Pro re-review is blocked by connector selection; no production
approval is claimed. See the final handoff section below for the precise boundary
between the full historical run and the last focused Windows fixes.

| ID | Observable contract | Layer and independent oracle |
| --- | --- | --- |
| R3-A | Relative profiles follow PowerShell's calling FileSystem location, not its process directory; all four commands and saved settings agree | Real installer fixture in one PowerShell process after `Set-Location`; expected path constructed in Rust |
| R3-B | Windows case-insensitive rejection tests include Git Bash; POSIX checks exact reserved paths | Platform-aware fixture cases; this is an incorrect-oracle correction, not a Windows product regression |
| R1-O | Optional Python-absence test may remain not run; required dependency audit and complete per-operation process observation cannot be omitted or invented | Existing evidence consumer with independent JSON fixtures; reject missing/incomplete observation and interpreter launches |
| R1-Q | Invalid UTF-8 always fails; actual BOM and long-file exceptions must match an independently reviewed exact set, including paths and reasons | Existing checkpoint bundle checks against real temporary files and approved records; reject substitutions/duplicates/count drift |
| R1-S | Offline checkpoint creation does not require a prior recovery package or live deployment; live results remain separate and pending | Real checkpoint create/extract verification with pending post-deployment status; retain existing source/artifact mismatch cases |

Preserve Rust/PowerShell/rollback source fingerprints, file counts/bytes, all four
executable identities and offline safety checks. Do not combine historical passing
tests with a later partial run to manufacture final-source certification. Observation
of an exercised workflow is not proof that every portable/renamed interpreter is absent.

## Implemented boundaries

- `CdrInstallEnvironment.psm1` resolves relative paths through PowerShell's current
  FileSystem provider, then applies the existing reserved-runtime-directory guard.
- `EvidenceShape`, `EvidenceContract` and the focused `NativeEvidence` module require
  the optional/not-run absence declaration, source-bound dependency audit and all five
  complete observed operations. Unknown/duplicate/missing operations, failed commands,
  missing boundary canaries, uncovered time intervals and recognized Python starts fail.
- `Quality` measures all Rust and checkpoint rollback text, a deliberately explicit
  scope replacing the old ambiguous changed-text counters. Invalid UTF-8 still fails.
  Its measured exceptions must match the separately reviewed `QualityApprovals.json`
  exactly. That approval file is itself included in rollback hashing and packaging.
  The 19 existing production Rust files are unchanged from HEAD; no long-file waiver
  was granted to newly lengthened production code. The Korean PowerShell BOM remains.
- The offline checkpoint keeps live verification pending in a separate record. The
  existing Rust/PowerShell/rollback source and four-executable bindings are retained.
- `Test-CurrentPcNativeWorkflow.ps1` runs fixed native fixture/dry-run commands and
  records their actual observed descendants through `CdrNativeProcessObservation`.
  It checks source identity before publication and refuses to overwrite evidence.
  It produces a **gate supplement**, not a full workspace or deployment certificate.
  Real Chrome/Pro, live MCP networking and production Discord/restart evidence remain
  separate; the MCP offline step checks inventory and temporary OAuth storage only.

## Evidence at implementation review

- R3-A RED: the new real-installer test selected the process directory rather than
  its same-process `Set-Location` directory. GREEN: the same test plus six existing
  profile tests passed (7 tests; both save/no-save modes and four child profiles).
- R3-B is a platform-oracle correction, not a manufactured Windows RED. Windows case
  variants remain tested in PowerShell and Git Bash; native Linux execution is not claimed.
- R1 RED: `evidence_integrity_15` rejected the truthful optional/not-run record with
  `python_unavailable_execution must be passed`. GREEN: actual checkpoint creation and
  extracted verification passed while live status remained pending and input unchanged.
- Four current-PC integrity tests initially passed. The fifth positive two-shell
  case caught PowerShell 7 JSON timestamp materialization as UTC `DateTime`; handling
  that representation fixed the positive case, including approved exceptions and real
  invalid UTF-8 rejection. Full regression rerun is in progress, not yet called a pass.
- The actual process observer test passes, including a command exiting 7 producing no
  passing record. This is added coverage, not claimed pre-implementation RED.
- The first expanded native workflow exposed a stale shell-test separator expectation;
  exact normalized single-key equality replaces its substring check. The focused test
  passes and still verifies original unrelated settings and conflicting-artifact refusal.
- A producer configuration initially selected an MCP library target containing zero
  tests. The positive-test guard correctly rejected it; the fixed workflow names the
  actual two inventory and three OAuth-store tests, without weakening that guard.
- Workspace Clippy with `-D warnings` passed. Changed operational PowerShell files parse.
- A real completed supplement is in
  `docs/rust-migration/evidence/current-pc-followup-20260911.json` (ignored, public-safe).
  All five operations completed: install 829 owned starts, setup 8, Pro helper 14,
  start/restart fixtures 33, MCP offline 15; total 899, recognized Python starts zero,
  boundary canaries verified. The independent dependency audit ran both required tests.
  Quality: 1,233 Rust files and 1,357 scoped text files checked; invalid UTF-8 zero,
  Rust BOM zero, operational BOM one, 19 reviewed long production Rust files.

## Subsequent local checks before Pro code re-review

- The first full run reached 318 summaries (1,533 passes) and stopped on two old
  checkpoint assumptions. The approval JSON is now explicitly packaged/hashed as
  data, outside the PowerShell-tool parser list. The stale-source test now supplies
  an internally consistent stale audit/observation to exercise the actual-source
  guard, instead of being rejected by the earlier internal-consistency check.
- An additional meaningful RED reproduced accepting an observation bound to different
  installation/recovery source. Audit and observation now also bind to the same
  canonical rollback-record SHA-256 as the gate. Rust fixtures independently implement
  that frame; both PowerShell shells exercise its equality and mismatch checks.
- The combined checkpoint run passed **49 tests**: native checkpoint 2, base 16,
  evidence integrity 19, rollback completeness 6 and staged/archive binding 6.
- The observer waits for both boundary canaries **and all owned termination events**
  within a bounded deadline. It still rejects incomplete observation. Both the real
  canary/failing-command test and deterministic delayed-stop/foreign-PID-reuse test pass.
- The first 899-process supplement above is historical, preceding the added rollback
  binding; it is not a final-source certificate or the latest consumer's input.
- A repeat observer run overlapped full-workspace tests using the same Windows Cargo
  directory and hit `os error 5` while replacing the in-use debug Pro helper. No passing
  supplement was published. Run `Test-NativeWorkspace.ps1` and
  `Test-CurrentPcNativeWorkflow.ps1` **sequentially** on a shared Windows build directory.
  This was verification scheduling, not a production bot stop or a silent retry.

The final full-workspace rerun and final-source supplement are pending; Pro is asked for
code review only, not final-source or deployment approval. No production runtime,
profile, database or shared Python was changed; no production restart, commit or push.

## Focused Pro code follow-up and remaining observation fixes

Pro returned **R3 CODE PASS / R1 CODE REVISE / LIVE HOLD** in
https://chatgpt.com/c/6aa3a4a0-cbcc-83e9-a916-faadd63d6289 . Its two R1 counterexamples
were independently reproduced as failing tests before changing the observer:
boundary canaries alone could replace a missing command start, and foreign PID
reuse could erase the previous owned instance's unconfirmed stop.

- `Invoke-CdrNative` now optionally returns the actual child PID, executable name
  and kernel creation/exit times through a separate reference, without changing
  existing stdout or error behavior. No process command lines or credentials are observed.
- Observation requires the native command's unique PID lifecycle, matching name
  and direct parent, plus both identified canaries. Creation metadata is recorded.
  Multiple owned lifecycles for that PID are rejected as ambiguous.
- Current PID ownership and outstanding termination obligations are separate. A
  foreign reused PID or its stop cannot erase the old instance's missing stop.
  Late-delivered events are re-ordered on each bounded drain; missing stops produce
  the original incomplete-observation error without publishing evidence.
- WMI `TIME_CREATED` is a **notification timestamp**, not kernel process creation.
  On this PC a start notification arrived about 0.9 seconds after native exit.
  Tests therefore allow notifications after exit but inside the observation window;
  they do not pretend those timestamps equal kernel creation/exit times. This is
  bounded observed-path evidence, not proof of lossless system-wide process tracing.
- Gate consumers require positive command identity, the observed start/stop pair,
  native lifetime and coherent observation bounds. Missing/false/invalid identity
  fields are rejected in PowerShell 5.1 and 7; canary-only positive fixtures were fixed.
- All four observer tests pass, including actual successful/failed commands,
  missing command with visible Python child, ambiguous PID reuse, late notifications,
  foreign reuse before an old stop, and bounded missing-stop rejection with no
  success record or leaked subscription.
- The second full workspace run passed its tests but Clippy rejected one new test
  helper's `push_str(format!(...))`. It now uses `writeln!` without allocating a
  temporary string. A final full rerun and the sequential observation are required.

This last re-review is focused on these two corrected observation paths, not a new
architecture review. Production deployment and live checks remain outside this step.

The focused re-review could not be sent: the installed helper returned
`connector_picker_unavailable`; the one connector retry permitted in this Codex turn
was already used for initial fresh-chat recovery. No further chat, connector retry
or replacement model was used. Local tests continue, but final Pro code approval
remains pending. The configured blocker hub has no ingest token on this machine;
its public-safe event is retained locally instead of claiming a delivered alert.

## Final local implementation handoff

The final workload exposed two Windows details that the original short `cmd.exe`
case did not cover. `cargo.exe` is reported by Windows as `rustup.exe`, and a long
executable's stop-trace name can be truncated (`cdr-pro-helper.exe` became
`cdr-pro-helper`). The first real observation correctly refused to publish a pass.
A new executable-alias/long-name regression reproduced the refusal before the fix.

`Invoke-CdrNative` now caches the OS process name while the launched process exists,
instead of assuming the requested filename is that name. Stop events match the
current ordered PID generation, not the lossy stop-name field. Pending obligations
from older generations remain separate. All five observer regressions passed after
these corrections, including missing-start, missing-stop, foreign PID reuse, actual
command failure, late notifications, aliases and truncated stop names. Clippy with
warnings denied passed again on this final code.

Verification records are deliberately not combined into a fictitious final gate:

| Record | Actual result | Boundary |
| --- | --- | --- |
| `followup-native-workspace-20260911-03.log` | Full build/tests/Clippy/format/dry-runs passed; 1,902 tests, zero failures, 27 default-ignored entries | Precedes the last alias/stop-name fixes; not a final-source workspace certificate |
| Final `native_process_observation_contract` | Five tests passed, zero failures | Final observer and optional launcher identity path |
| `current-pc-followup-20260911-03.json` | All five native workflows completed; 894 owned starts, zero recognized Python starts | Final-source offline gate supplement, not a full release or deployment certificate |
| Copied supplement readback | Contract, Rust/PowerShell/rollback binding and actual quality all passed in PowerShell 5.1 and 7.6.5 | Independent consumption of the saved final record |

The 27 default-ignored entries comprise 21 child-process fixture entries and six
separate live checks. They are not silently counted as passed live verification.
Full-run logs and the original supplement are in `D:/codex-work/cdr-python-free-qa/`.
The byte-identical, ignored repository copy for later Pro inspection is
`docs/rust-migration/evidence/current-pc-followup-final-20260911.json`.

Final observed starts: installer 829, setup dry-run 8, offline Pro helper 14,
start/restart contracts 33, offline MCP contracts 10. Every command's actual identity
and start/stop notifications were verified, and the independent dependency audit
passed both required tests. Quality remains 1,233 Rust / 1,357 scoped text files,
zero invalid UTF-8, zero Rust BOM, one reviewed operational BOM and 19 unchanged
reviewed long Rust files. Shared Python remains installed; renamed interpreters,
unexercised paths and live Pro/MCP/Discord success are not claimed.

Final Rust source fingerprint:
`9251EA3668F02822695B1B7E71B999E489DF2FAF26C58E227520B68BD14D8CBD`.
Final rollback-source record fingerprint:
`4F84C5EFB53A36604086CA3DDF5EC6499289FC9D6FDD76542F22419C398E38BA`.

Before release, finish the focused Pro re-review, run the complete final-source
workspace/release gate, create and verify its recovery package, and perform the
separately authorized live/cutover checks. The production bot/profile/state were
not replaced, stopped or restarted; no commit or push was performed in this step.

## Subsequent focused re-review — 2026-09-11 08:00 UTC

The user-requested retry completed in
https://chatgpt.com/c/6aa3b2d3-94c8-83ee-88b3-d55698797add . Pro returned
**R1 CODE REVISE / LIVE HOLD**, with one mandatory residual observer finding.
The original two counterexamples are blocked and R3 remains approved. The earlier
connector failure is recovered; it is no longer the reason for missing approval.

The broad foreign-stop claim above requires qualification: it holds when the
foreign start is already in the collected events. If that start is missing from
the snapshot, the stop reducer can attach an incompatible foreign stop to the old
owned PID. Separate start/stop queries and early completion make this reachable
without requiring permanent event loss. Codex independently reproduced
`ready=true` with the wrong stop at 10, `ready=false` after the foreign start at 9
was added, then correct completion only when the old stop at 6 was added late.

Next code work is scoped to compatible stop-identity validation that preserves
Windows name truncation, plus actual-reducer and split-query/no-publication
regressions. Details and independent disposition are in
`python-free-pro-review-20260911.md`. The current 894-start observation is not
asserted to contain this race; it does not override the residual code finding.
Final-source full gate, recovery package and live/cutover remain separate work.

Only review records were updated in this retry. Product source, production bot,
profile and shared Python were not modified; no restart, commit or push occurred.

## Stop-identity correction completed — 2026-09-11 08:37 UTC

The user subsequently authorized correcting the residual finding and re-reviewing
it. That work is complete: [Pro returned R1 CODE PASS / LIVE HOLD](https://chatgpt.com/c/6aa3bc43-f468-83ee-8f74-021477f11d00),
with no mandatory remaining code fix or additional regression requirement.
`CdrNativeProcessObservation.psm1` now rejects incompatible stop names and known
contradictory parents while preserving the verified ASCII 14-character truncation
and unknown parent 0. Four new actual-reducer/producer regressions were RED before
the fix and GREEN afterward; five existing observer and 49 checkpoint tests passed.

Workspace per-package formatting and Clippy passed. A fresh five-workflow record
observed 891 owned starts, zero recognized Python starts; both PowerShell consumers
verified its contract, actual source binding and quality. This record supersedes
the older observation only for the current-PC supplement, not full release approval.
See `python-free-stop-identity-fix.md` for source hashes, commands, the Pro verdict
and evidence boundaries. Pro's observer hash matched the actual local file.

The production bot/profile/databases and shared Python were not changed. Final
release gate, recovery packaging and live cutover remain separate work; no
production deployment/restart, commit or push occurred in this correction turn.
