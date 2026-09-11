# Python-free migration: independent review and remaining work

Date: 2026-09-11. Status: not approved for production; no production stop or replacement in this review.
No new Windows installation or shared-Python removal is required by the user.

Current status: the approved R1/R3 changes and the subsequent incompatible-stop
finding have been implemented. The [final focused Pro re-review](https://chatgpt.com/c/6aa3bc43-f468-83ee-8f74-021477f11d00)
returned **R1 CODE PASS / LIVE HOLD**, observed 2026-09-11 08:37:42 UTC. Prior R3
approval stands. No mandatory residual fix or new test was requested for this issue.
The detailed sections below preserve historical decisions, not new pending code work.

The four new regressions failed for the intended defect before the code fix, then
passed unchanged. Existing observer tests and related checkpoint tests bring the
focused total to 58 passed, zero failed/ignored. Per-package workspace formatting,
Clippy, and the new source-bound five-workflow observation passed (891 owned starts,
zero recognized Python starts); saved-record contract/source/quality readback passed
in PowerShell 5.1 and 7.6.6. The monolithic fmt command's Windows length failure was
not relabeled as a passing run. Final-source full release and live checks remain
separate. See `python-free-stop-identity-fix.md` for the exact contract and evidence.

Pro's reviewed observer hash matches the actual local file. The final answer and
response actions were observed; no Pro command execution or production change was
authorized. The correction and requested re-review are complete, not deployed.

## Previous focused re-review — 2026-09-11 08:00 UTC

[ChatGPT 6 Pro re-review](https://chatgpt.com/c/6aa3b2d3-94c8-83ee-88b3-d55698797add)
completed after 9m 40s: **R1 CODE REVISE / LIVE HOLD**, with **one mandatory code
finding**. The previous two exact counterexamples are blocked; prior R3 approval
and the evidence/quality/source-binding decisions were not reversed. Pro did not
count unfinished release or live checks as additional code defects.

The native skill's single permitted connector retry succeeded through the exact
OAuth catalog after the saved conversation's picker failed. Normal Chat, 6 Pro,
the named OAuth attachment, `sim-pc-3060` and the Rust project were verified. The
new canonical URL was saved; the old conversation and unrelated tabs were preserved.
The final answer and response actions were observed. This was source review by
Pro, not command execution or live testing by Pro.

### R1 remaining: incompatible foreign stop falsely completes an owned instance

`scripts/CdrNativeProcessObservation.psm1:27-32` matches a stop using PID and event
order alone. If the old owned stop and the reused foreign PID's start are absent
from the current collection, a foreign stop can fill the old instance's
`stop_tick`. Its incompatible name and parent are ignored. The shared object then
passes command matching and `ready` at lines 51-55. The producer can publish a
completed record at lines 87-112; the consumer cannot recover the discarded stop
identity from that record.

This is not a demand for lossless system-wide tracing. Start events are queried
before stop events. A foreign start/stop pair arriving between those two queries
can leave only the stop in the current snapshot, even without permanent event
loss. Early `ready=true` then prevents the next collection from correcting it.

Codex independently ran the actual reducer read-only, using root PID 42, command
PID 52 (`worker.exe`, creation 3, exit 6), canaries at 1/2 and 7/8, and the foreign
`python.exe` stop at 10 (parent 999):

| Input progression | Actual result | Meaning |
| --- | --- | --- |
| Foreign stop only; both old stop and foreign start absent | `ready=true`, no remaining instances, matched stop 10 | Reproduced false completion |
| Add foreign start at 9 | `ready=false`, remaining `52@3` | Existing generation tracking works when that start is present |
| Add actual old stop at 6, delivered late | `ready=true`, matched stop 6 | Reordering can complete with the correct stop |

Local Windows class metadata also confirms that stop events expose
`ParentProcessID` and `ProcessName`. One intermediate inline diagnostic had a
missing function brace and did not execute; the corrected three-case run above
exited 0. No product source was changed for either diagnostic.

Accepted minimum follow-up: preserve generation/order matching, reject clearly
incompatible stop identity while retaining the verified long-name truncation
compatibility, and leave ambiguous termination pending. Add the actual reducer
counterexample plus a split-query/publication test with the real reducer, checking
timeout, no success record and subscription cleanup. Retain the real alias,
long-name, delayed-event, PID-reuse and successful/failing-command regressions.

The existing 894-start record is not claimed to have experienced this race, and
the earlier 1,902-test run is still not final-source release certification.
This turn changed only review/handoff/pending-status documentation: no code fix,
build, deployment, bot restart, production profile change, commit or push.

## Completed initial review

[ChatGPT 6 Pro review](https://chatgpt.com/c/6aa37b7f-35bc-83e9-bc42-0de7b419297c)
completed in normal Chat using exactly Simdorei Local Project Oauth. Device
`sim-pc-3060` and the separate Rust repository were explicitly selected. The review
was read-only: no Pro command, file mutation, deployment or Discord send was authorized.
The actual native helper came from the installed/enabled isolated plugin's `source.path`,
version `0.1.0+codex.20260911034456`. The production profile was not changed.

Pro's decision was CODE REVISE / verification REVISE / LIVE HOLD. Its code findings
were independently checked locally; its source-based examples are not claimed as
test executions performed by Pro.

## Findings and local disposition

### R1: obsolete checkpoint evidence requirements — pending implementation

`scripts/RustMigrationCheckpoint.EvidenceContract.psm1` still requires
`python_unavailable_execution = passed` and zero BOM/over-250-line counts.
The real checkpoint generator calls this consumer. That contradicts the user's
current-PC acceptance contract and the existing PowerShell 5.1 BOM requirement.

Accepted minimum change; focused Pro follow-up returned PLAN PASS, not implementation
or deployment approval:

1. Keep the existing workspace evidence structure and actual command/exit-code records.
   Record the strong no-installed-Python test as optional/not run; require current-PC
   deliverable/call-site audit and observed interpreter launches for exercised workflows.
2. Continue rejecting invalid UTF-8. Record actual BOM/long-file counts plus exact
   reviewed paths and reasons, and reject entries that do not match the reviewed set.
   Do not invent zero counts or split unrelated pre-existing files indiscriminately.
3. Add native contract cases for a truthful accepted record, missing observation,
   unreviewed exceptions and source/artifact mismatches. Preserve existing source
   fingerprints, source sizes/counts, rollback/PowerShell source binding and all four
   executable hash/size checks. Do not elevate `-SkipUnitTests` to full-workspace proof.
4. Produce evidence only from the completed final-source commands. Build/extract/verify
   the current-source Rust recovery package before a controlled production replacement.

Follow-up implementation contract: optional absence testing must explicitly record
`required=false`, `status=not_run` and its reason. Required audit/observation must identify
each exercised operation, command/exit code, observation interval, observer canary and
completion; missing or interrupted observation is not zero interpreter starts. Validate
exception paths, counts, BOM/line data and reviewed reasons against an independently
approved exact set, rejecting omissions, duplicates and same-count substitutions.

Keep the offline workspace gate free of live-network claims. The checkpoint generator
consumes final-source evidence, creates the recovery package, then verifies its fresh
extraction; the finished package is not a prerequisite of its own input gate. Separate
isolated live Pro/MCP records from offline evidence. Post-deployment response/restart
proof is a separate record bound to the checkpoint hash, not an edit to frozen evidence
or a prerequisite for creating the offline checkpoint. Preserve all source/artifact
bindings and add rejection tests through the existing evidence-integrity contracts.

### R2: saved Windows profile parsing — CODE PASS after focused review

`Get-EnvFileValue` compared an untrimmed key and did not handle single-quoted values
like the Rust environment reader. A valid `CODEX_HOME = '...'` could be missed, causing
installation into an inherited/default profile. The new regression failed with the
wrong actual child profile. Key trimming, case-sensitive lookup and quote trimming
now match the relevant Rust reader behavior; invalid non-assignment lines are ignored.

### R3: profile identity and runtime-directory refusal — CODE REVISE

Relative profiles were passed unchanged to Codex, then saved unchanged even though
the bot starts from another directory. A new test reproduced the wrong relative value.
Relative-path support is preserved: resolve once from the installer's calling directory,
then use that same absolute value for configuration and all four plugin commands.

Git Bash uses its bundled `realpath -m` then `cygpath -am`, each as a checked assignment.
The intermediate step is needed because `cygpath` alone rejects a nonexistent `x/..`
component. POSIX paths use existing shell/awk tools for lexical normalization.
Normalize and reject reserved runtime directories before any installation writes.
Windows case, separator, trailing-slash and version-subdirectory handling is consistent.

Three new test functions reproduced meaningful failures: spaced/quoted saved profiles,
versioned runtime-bin rejection, and caller-relative profiles with/without env writes.
All six profile tests passed after correction. The final broader recheck passed 17
installer tests and two lexical-probe tests, zero failures. Build, formatting, Clippy,
Windows and shell checks with `-SkipUnitTests` also exited 0. The earlier full-workspace
total did not include these new regressions and is not relabeled as final-source proof.

The focused follow-up found two remaining cases, independently checked locally:

- **R3-A, Windows caller location:** `CdrInstallEnvironment.psm1:52` uses .NET
  `GetFullPath`, which can resolve relative paths from the process directory instead
  of PowerShell's current FileSystem location after `Set-Location`. A read-only local
  reproduction retained the process directory at the Rust repository, moved the
  PowerShell location to `D:\codex-work\cdr-python-free-qa`, and resolved
  `review-relative-profile`. Expected: that QA directory's child. Actual: the Rust
  repository's child. `matches_caller_directory=false`; neither path was created.
  Resolve once using the calling PowerShell FileSystem location. Add same-process
  location-change tests, with and without environment-file writes, checking all four
  child profiles and the saved value. Existing child-process `current_dir` tests do
  not exercise this distinction.
- **R3-B, non-Windows test expectation:** `install_profile_contract.rs:138-165`
  demands case-insensitive Windows-path rejection on non-Windows hosts too, while
  `install.sh:75-96` intentionally retains POSIX case sensitivity. Source inspection
  confirms the mismatch; no native Linux test run is claimed. Guard Windows case
  variants by host `cfg!(windows)`, retaining Git Bash coverage on Windows; preserve
  exact reserved-path cases for POSIX. Do not condition only on the wrapper selector.

Neither follow-up correction has been implemented in this review-only turn.

## Focused follow-up completed in a user-authorized new conversation

[ChatGPT 6 Pro focused follow-up](https://chatgpt.com/c/6aa3847c-6898-83e8-8f9a-53514cecfc19)
completed after 8m 3s. Recorded 2026-09-11. The existing conversation again returned
`connector_picker_unavailable`. The user explicitly authorized a fresh conversation
on connection failure. The installed native helper's permitted retry succeeded through
the exact OAuth catalog (`verified_catalog_launch`); normal Chat, 6 Pro and the inline
Simdorei Local Project Oauth attachment were verified before sending. The prompt's
literal device/project tag was verified, and the canonical new URL was saved in the
local conversation map. The previous conversation remains intact.

Pro returned **R2 CODE PASS / R3 CODE REVISE / R1 PLAN PASS / LIVE HOLD**. The final
answer and response actions were observed, not inferred from thinking updates. This
was the requested focused follow-up to the initial review, not a replacement broad
audit. Pro performed source review; the Windows reproduction above was performed
locally by Codex. No Pro command execution, source modification, runtime replacement,
production restart, commit or push occurred. Connection recovery does not establish
that the old conversation's modern-composer attachment defect is fixed.

## Current evidence and nonclaims

- Latest full test phase: 422 summaries, 1,889 passed, zero failed, 27 existing ignores.
  QA then failed on two test-only Clippy raw-string warnings; corrected workspace
  Clippy exited 0. Later installer changes mean this is not final-source certification.
- Actual isolated native installation, setup dry-run and helper verification passed.
- Bounded WMI start/stop observation: 88 owned process starts, observer canary seen,
  zero recognized Python starts. The 26 runtime/start/restart fixture tests and three
  local Rust MCP integration tests passed inside that observed workflow. It does not
  claim clean-Windows proof, renamed-interpreter detection or a live production restart.
- Original production bot: verified PID 49728, fresh heartbeat, no stop/restart markers.
  No production plugin update, runtime replacement, commit or push occurred.
- The SHA-pinned release `cdr-mcp-server.exe` also served actual loopback HTTP requests
  from a new isolated OAuth database: health 200, OAuth metadata 200, unauthenticated
  MCP initialization 401. Process ID 30536 and its exact artifact were verified, then
  only that owned test process was terminated. No VPS or production state was used.
  The first diagnostic assertion incorrectly expected an added slash in the issuer;
  correcting the fixture to the exact configured issuer made it pass without a product
  change. This smoke is not a full authenticated file request through that executable
  or a graceful production restart.
- Blocker events are retained under ignored `.blocker-pending`; hub delivery was not
  possible because no ingest credential was configured. No credential was printed.

Remaining: R3-A/R3-B corrections and code re-review; R1 evidence consumer/recording
implementation and truthful final-source checks; current-source recovery package;
controlled installation/start/restart and Discord behavior/readback checks. The focused
review connection is no longer a blocker. Its previous pending event is marked resolved
locally; the final-gates event retains the actual unfinished work without claiming hub
delivery.
