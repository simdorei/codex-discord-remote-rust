# Python-free migration acceptance and evidence

Status: implementation in progress, not deployed.

Latest workspace test phase: 422 suite summaries, 1,889 passed, zero failed and 27
existing ignores. QA then failed on two unnecessary raw-string delimiters in the
new lexical-probe test. Both were corrected and workspace Clippy passed. Initial
6 Pro review then identified installer profile defects and obsolete checkpoint
conditions. The installer defects were reproduced and corrected; the final focused
recheck passed 17 installer and two lexical-probe tests. Build/format/Clippy/Windows/
shell checks with `-SkipUnitTests` also passed. These separate runs are not a new
final-source full-workspace certificate. Production remains unchanged.

Initial native Chrome/OAuth/6 Pro review and the required focused follow-up completed.
The user-authorized new conversation recovered the old conversation's attachment
failure: R2 CODE PASS, R3 CODE REVISE, R1 PLAN PASS, LIVE HOLD. Remaining R3 cases are
PowerShell's same-process location change and a non-Windows test expectation mismatch;
the first was reproduced locally without writes. No follow-up product fixes or deployment
were performed in the review turn. See `docs/python-free-pro-review-20260911.md` for the
new conversation and the approved checkpoint plan. Historical evidence below is retained;
earlier pending statements are superseded by this dated status.

The approved outcome is a project that needs no Python interpreter for installation,
configuration, execution, Pro/MCP helpers, recovery, or automated verification.
The separate Rust repository is the implementation workspace. The existing live bot,
configuration, database, shared system Python, and unrelated projects are preserved.
VPS production deployment and new native operating-system support are not included.

Acceptance update approved by the user on 2026-09-11: do not install a VM or require
a fresh Windows environment. Preserve shared Python. Required proof is deliverable
dependency/call-site inspection, native install/start/restart tests and real Pro/MCP
checks, including observation of interpreter launches in the exercised workflows.
A clean-Windows/no-installed-Python test is optional and remains explicitly not run;
PATH filtering alone is not represented as that stronger proof.

## Requirement contracts

| ID | Observable contract | Test boundary | Status |
| --- | --- | --- | --- |
| PF-01 | Setup/discovery/inventory/backup work without Python | Rust management CLI with cleared environment; Windows wrapper in isolated fixture | Initial tests green |
| PF-02 | UTF-8, existing settings and secrets survive setup; failed validation does not write | Real HTTP adapter against local fake Discord; real temporary env files | Initial tests green |
| PF-03 | Pro evidence hooks and helper workflows work without Python | Hook input/output plus receipt consumers and browser script contracts | Native contracts, initial and focused real Chrome/OAuth/6 Pro reviews completed; user-authorized new conversation required; live Discord transport not deployed |
| PF-04 | Tests use independent frozen contracts and Rust fixtures, not executable legacy Python | Deliverable dependency/call-site audit, native workspace checks and observed process launches on this PC | Bounded native workflow observed 88 owned starts and zero recognized Python starts; final-source certification pending |
| PF-05 | New/mirror/progress/final/steer/archive/settings/usage behavior remains intact | Existing Rust behavior contracts and controlled Discord QA | Pending |
| PF-06 | Restart and rollback use verified Rust artifacts and preserve state | Offline ownership/backup/recovery tests before live cutover | Pending |
| PF-07 | Installed executable identity, heartbeat and Discord response are verified | Local deployment readback | Pending |
| PF-08 | Default setup paths work without an explicit folder or binary override; absolute Cargo paths with spaces stay absolute | Real Windows PowerShell/Git Bash wrappers and native CLI in isolated temporary folders | Three new regressions RED to GREEN; 30 related tests pass |

## Implementation order

1. Reuse Rust discovery, plugin verification and SQLite backup; replace setup entry points.
2. Port Pro/MCP support commands and hooks; retain only required OS/browser glue.
3. Replace interpreter-driven fixtures and CI, and remove superseded Python files only
   after their supported contracts have independent replacement coverage.
4. Run native dependency/call-site and behavior regression checks on this PC. Investigate existing CI failures;
   do not skip tests or weaken assertions to obtain a pass.
5. Establish a Rust-only known-good recovery path, then controlled local deployment.

## Evidence so far

- `cargo test --locked -p cdr-runtime --test admin_cli_contract -- --test-threads=2`
  RED: 7 tests failed because `--admin` was unsupported. One nonmutation assertion was
  already green and was strengthened to require the actual invalid-value diagnosis.
  GREEN: all 8 tests passed after implementation.
- `python_free_installer_contract`: initial fixture was missing the old release manifest;
  that setup defect was corrected before product changes. The meaningful RED then showed
  the old installer trying to execute the deliberately nonexistent `PYTHON_EXE`, and the
  setup wrapper lacking a Rust binary entry point. GREEN: both wrapper tests passed.
- Combined `admin_cli_contract`, `admin_setup_contract`, `python_free_installer_contract`:
  14 passed, 0 failed. Four setup tests are characterization/added coverage, not claimed RED.
- A direct Windows PowerShell-to-Rust stdin check carried Korean correctly and rejected
  an invalid channel before network or config writes. Only a synthetic token was used.
- `cargo fmt --all` hit Windows command-length error 206. Changed files are formatted
  with direct, bounded `rustfmt --edition 2024` invocations instead.

No production process was stopped, no real token was entered, and no repository push
or deployment has been performed by this migration work.

## Additional implementation evidence

- Native Pro helper: 36 all-target tests plus 3 helper CLI contracts passed. Hooks,
  conversation creation leases, crash/restart fencing and exact probe bytes are covered.
- An isolated real Codex profile installed a synthetic local plugin containing the
  native helper. Cached executable SHA-256 matched its source, including when `*.exe`
  was gitignored. The production Codex profile was not changed.
- A Windows ACL regression demonstrated that atomic env replacement dropped a
  protected DACL. The native staging helper now copies access rules **before** writing
  secret contents, retaining the original protected/inherited behavior. All 5 setup
  tests pass. The first ACL fixture failed to load an inherited PowerShell module;
  the fixture was corrected to use .NET before the meaningful RED was measured.
  API basis: [Microsoft GetNamedSecurityInfoW](https://learn.microsoft.com/en-us/windows/win32/api/aclapi/nf-aclapi-getnamedsecurityinfow)
  and [SetNamedSecurityInfoW](https://learn.microsoft.com/en-us/windows/win32/api/aclapi/nf-aclapi-setnamedsecurityinfow).
- Windows launcher/status/restart/watchdog: 2 new contract failures before removal
  of the legacy branches; 4 new contracts plus 9 existing watchdog contracts pass.
- Native test server ports: 291 runtime unit tests and 28 focused integration tests
  passed (5 pre-existing fixture/interactive ignores unchanged). This includes new
  first reply, mirror, ownership, goal/final handoff, steering, approval and settings.
- Legacy store and OAuth contracts: 8 tests passed using independent SQL fixtures;
  details and explicitly retired runtime-only coverage are in
  `fixtures/parity/python-free-fixtures.md`.
- POSIX setup stdin protocol: 2 tests failed on unsupported `--input-lines` before
  implementation; both pass, including malformed input and non-disclosure checks.
- Shell wrappers: 2 tests pass. The first RED included a fixture Bash path error;
  discovery now resolves Bash from the actual Git installation instead of a fixed
  `C:\Program Files` path. Windows execution is verified with Git Bash, not claimed
  as new native macOS UI support.
- Tray: 4 characterization tests passed before changes; the removed-runtime mode
  regression failed. After removing the legacy branch all 5 native contracts pass,
  retaining process/path binding, restart identity and failure/exit-code checks.

## Install, backup and recovery ports

- Installer staging now refuses an existing different runtime before changing its
  selection/configuration. A first-install staging regression and the shell staging
  contract both went RED to GREEN. Current shell wrapper suite: 3 passed.
- Offline admin backup shares the runtime's root/home/relative DB path resolution.
  All 3 new path contracts went RED to GREEN; backup output is an absolute path.
- Cutover is Rust-only and interrupted recovery requires explicit `-Recover`.
  No failed transition silently starts a different runtime. Two new contracts went
  RED to GREEN; 8 existing cutover contracts and 12 ported safety cases pass.
- Checkpoint inventory, manifest, hash evidence and extracted checks include four
  artifacts: runtime, offline soak, MCP server and Pro helper. Workspace evidence is
  schema 2 with native gates. The old evidence schema is not silently accepted.
  Extracted verification runs only native offline admin/helper probes, not a bot.
- Checkpoint process checks bind to the target repo or its runtime-lock PID. This
  permits checking an isolated fixture while the production bot in another repo
  keeps running, but still detects this repo's externally located executable.
  Legacy Python writer detection is retained as a **read-only safety check**, not
  as permission to launch Python. All 44 checkpoint contracts pass.
- Native maintenance suites: 78 test functions / 120 parameterized scenarios ported
  from existing wrappers passed, plus the liveness/error-redaction case (3 scenarios).
  Recovery safety: 12 passed. Restart transaction/entry: 9 passed. Maintenance entry
  ownership/policy checks: 3 passed. Assertions were kept, not replaced by skipped tests.
- The real native snapshot integration passes: read-only source SHA is unchanged,
  snapshot integrity/schema/data match, pre/post-stop receipts reference distinct
  snapshots, no state DB/app-server or stop marker was created. Data are synthetic.

## Native attachment sender

- `cdr-runtime --admin send-attachment` replaces the standalone sender, supporting
  direct channels, active/archived thread refs, work-thread alias and UTF-8 captions.
  Mapping reads never initialize or migrate the bot database. Ambiguous references
  and duplicate room ownership fail visibly rather than selecting another room.
- Four CLI tests failed on the missing native command before implementation and now
  pass. Three target/store tests and six real local-HTTP transport tests also pass.
  These verify multipart bytes, UTF-8, caption-file precedence, permission denial
  before upload, redacted bounded errors, redirects blocked, response loss without
  retry, and exact message/channel/filename receipts. This is not live Discord QA.
- The sender has a local safety bound of 10 attachments / 100 MiB combined and 2000
  UTF-16 content units; actual Discord limits/errors remain authoritative. Automatic
  mentions are disabled. It neither retries nor falls back after an uncertain send.
- Restart QA now invokes the verified Rust sender, durably records the send attempt,
  and preserves the actual error if notification fails. A three-scenario wrapper
  contract went RED to GREEN, including changed-artifact refusal and no second send.
- Two cached dependencies (`mime_guess` 2.0.5, `unicase` 2.9.0) were added for standard
  multipart MIME handling. No dependency versions were upgraded.

At this stage the remaining gates included source/test caller closure and Python-file
removal (now completed below), native CI evidence generation, full workspace checks with interpreter execution unavailable,
Pro/browser and Discord functional QA, and controlled production deployment. The live
bot is still unchanged at the original repo; these results are not deployment claims.

## Additional native ports and full-workspace findings

- Cachebuster: 8 contracts pass, including calendar validation, strict version
  increase, untracked packaged files and compiled-helper dependency changes.
- Browser connector: all 46 original test methods were ported to Node's built-in
  test runner; the Rust suite invokes them without a Python wrapper. Strict menu,
  draft, duplicate connector, delayed result and stale-generation cases are retained.
- Native hook retry/order evidence: 6 additional contracts pass. Plugin packaging
  and exact frozen composer instruction: 3 pass. CI wiring: 5 native contracts pass.
- Default MCP Dockerfile and Compose now use the same pinned Rust image path as
  their `.rust` aliases; four container configuration contracts pass. Docker is not
  installed locally, so an image build/start is **not** claimed as verified.
- The two remaining legacy queue probes now inspect raw persisted SQLite fields,
  independently of the Rust queue decoder. Five store tests pass with no interpreter.
- `active-queue-count`: two new CLI contracts went RED to GREEN. The retired queue-only
  restart still refuses mutations; its explicit read-only check uses the native CLI.
- Restart boundary: 15 ported contracts pass. Deployment recovery: 4 pass, including
  an actual exclusive lock released by killing only the test's own helper process.
- Operator path pins: shared checks are tested with all four environment overrides
  against synthetic invalid-SQLite files. The one-off production-env opt-in test is
  replaced by an always-run native test; no real database or RPC is touched.
- A full workspace run reached the archive-writer test, then exposed a native fixture
  log race: readers tried to parse an incomplete NDJSON record. Two deterministic
  regressions went RED to GREEN; malformed complete records still fail. The original
  archive-writer test and all 291 runtime unit tests pass after the fixture fix.
- Non-Windows file-session activation was coupled to Windows UI initialization.
  UI initialization is now deferred until an actual UI request; the exact platform
  error remains visible. Three injected-platform lifecycle tests plus dispatcher
  tests pass on Windows. Native macOS execution remains a separate CI check.
- Native `list-threads`/`archive-thread` reuse the bot's existing lifecycle guards.
  Four contracts pass (including invalid UUID, missing store and persisted archive
  verification); the archive-used skill no longer invokes the legacy interpreter.
  No live thread was archived during these fixture tests.

## Source removal and final native utility ports

- Baseline audit found 1,244 tracked Python sources (583 under `tests/`, 604 root
  modules and 57 plugin/MCP/tool modules), all unchanged from commit
  `3b359b470fee412bf010825c0e64a6139ab03b3c`. They were removed only from the separate
  Rust repository. The historical 1,195-file count in the frozen parity contract is
  a prior snapshot, not the current removal count. Git retains the removed sources.
- Eleven Python-only dependency/launcher files were also removed: requirements,
  the embedded-interpreter release manifest, MCP Python dependency locks and six
  retired launcher modules. Checkpoint source packaging no longer requires the
  Python manifest or copies unused Python launcher fixtures.
- The execution tool rejected deletion of the ignored `__pycache__` directory.
  The user subsequently removed it. Read-only verification confirms absence, and
  both repository-closure checks pass without weakening or skipping the assertion.
- Installer compatibility: six plugin-install tests pass, retaining command-failure,
  all ten invalid-inventory cases on both wrappers, warning-stream separation,
  Korean/special-character paths, Windows cmd/ps1 shims and large-output capture.
  A failure must actually reach the inventory verifier, not merely fail earlier.
- Codex selection persistence: six scenarios pass. Inherited or automatically found
  executable paths are not saved; an explicit selection is saved and unrelated UTF-8
  settings survive. Cargo's output directory cannot be disguised by a custom binary
  argument. Existing runtime artifacts are never overwritten by ordinary installation.
- Pro helper staging: two tests pass after meaningful RED. Verification occurs on a
  temporary file before atomic publication; a failed hash check preserves the prior
  helper, and staging the same file is idempotent. The initial hash-injection fixture
  required Windows path normalization before measuring the real failure.
- `cdr-runtime --admin inspect-new-first-reply --database <db> --job-id <job>` is the
  native read-only preview. Three contracts / eight scenarios pass, including absent
  DB, duplicate identity, wrong destination and all receipt states. No message content,
  replay authority or delivery authority is exposed, and database bytes stay unchanged.
- `cdr-pro-helper collect-release-evidence --repo-root <repo> --codex-exe <exe>` replaces
  the Python release collector. Seven ported/extended contracts pass. It runs native
  checks, verifies installed inventory and a fresh resident, detects changed HEAD or
  working-tree content, and writes public-safe evidence. Zero tests or ignored required
  tests cannot count as a pass. Live browser/Pro and restart checks remain deferred.
- Former hard-coded VPS smoke scripts are replaced by native local gateway contracts,
  not silently redirected to a live server. Four tests pass: OAuth discovery/PKCE/revoke,
  unauthenticated MCP rejection, real file transport, and two real agents through the
  sequence 2 connected → 1 isolated restart → 2 restored, with B remaining usable.
  The first two-device fixture reused one credential and was correctly rejected;
  unique synthetic per-device credentials fixed the fixture before the smoke passed.
- Frozen legacy JSON/SQL/schema/token-format fixtures and read-only detection of a
  legacy writer remain. MCP may still offer `python:test` **for a user-selected external
  Python project**; that capability is not an interpreter dependency of this product.

Remaining acceptance gates are final source-frozen quality/evidence checks, independent
Pro/browser review, a verified Rust-only recovery package and controlled live deployment/
readback. The user removed the fresh Python-unavailable Windows environment requirement;
shared Python stays installed. No VM, Docker, Sandbox or WSL installation is required.
Changing PATH is still not described as proof that absolute/portable interpreters are absent.

## Final audit follow-ups

- Two additional shell installer contracts pass: injected comparison failure preserves
  the previous Pro helper, removes only staging debris, never reaches plugin commands
  and never writes a completed-install marker; successful repeat installation preserves
  the exact artifact. These were added coverage (already green), not claimed RED.
- The post-removal full run found one stale cutover **fixture** still copying a removed
  Python-only launcher. The runtime no longer uses that launcher. Removing the unused
  fixture copy preserves all assertions; all 22 cutover/recovery contracts pass afterward.
- Release evidence had a meaningful gap: a positive suite could mask another required
  suite with zero tests, or a missing suite summary. Two test functions went RED on
  these false-positive approvals. Classification now requires every requested suite to
  have a positive, unignored result; missing summaries and contradictory failure status
  cannot pass. All seven release collector/evidence tests pass after the correction.
- Latest workspace Clippy (`-D warnings`), per-package formatting and Rust documentation
  (`-D warnings`) pass. The external release rebuild including the collector correction
  finished with exit 0; native setup dry-run and release-helper cachebuster also pass.
  The candidate identities are recorded in `docs/python-free-candidate.md`, not deployed.
- Strict UTF-8 decoding passed for all 1,226 Rust files and 309 changed text files checked
  at the audit point. One pre-existing UTF-8 BOM is retained in the Korean PowerShell 5.1
  restart notification script; blindly stripping it would change Windows 5.1 decoding.
  All 85 root/operational PowerShell files parse successfully. Nineteen pre-existing
  production Rust files exceed 250 lines; none was lengthened by this migration. Their
  responsibility/exception review and the old checkpoint's stricter zero-BOM/zero-long-file
  evidence fields remain unresolved; no zero-count quality certificate was fabricated.

The native workspace evidence schema-2 producer, observed no-interpreter invocation,
independent Pro review, current-source recovery package and production cutover/readback
are not completed. A partially passing local run is not a substitute for those gates.

Earlier Chrome follow-up (superseded by the dated review above): the installed Chrome control tool initializes successfully.
The active Pro plugin is still version `0.1.0+codex.20260901081108`, with Python
evidence helpers and no installed `bin/cdr-pro-helper.exe`. No Pro prompt was sent;
native installed-helper evidence and the independent review remain unverified.
The live plugin was not replaced merely to obtain a passing result.

## Post-cache-cleanup path regressions (2026-09-10 UTC)

- `qa-smoke.ps1` completed all 420 workspace test summaries (1,881 passed, no failures,
  27 existing ignores), then failed with `GetFullPath: The path is not of a legal form`
  in the Windows setup dry-run. This was a real wrapper defect, not a unit-test failure.
- PF-08 Windows RED: `cargo test --locked --offline -j 2 -p cdr-runtime --test
  python_free_installer_contract setup_without_repo_root_uses_script_location_not_callers_directory
  -- --exact --nocapture` reproduced that error. The test invokes the real script from
  an unrelated empty directory, with no `-RepoRoot`, and checks the intended `.env`
  destination and absence of writes. Resolving the default after parameter binding
  fixes Windows PowerShell's empty parameter default without changing explicit roots.
- A subsequent wrapper check exposed Windows `CARGO_TARGET_DIR` values being prefixed
  with the repository path by both shell setup and shell installation. PF-08 shell
  RED: `cargo test --locked --offline -j 2 -p cdr-runtime --test shell_wrapper_contract
  native_absolute_cargo_target_with_spaces -- --nocapture --test-threads=2` failed both
  new tests with `Rust runtime not found`. Shell path classification now retains
  drive-rooted absolute paths; the tests use real native executables and paths with
  spaces, not a mocked successful launcher.
- GREEN: all 20 tests in `admin_cli_contract`, `admin_setup_contract`,
  `python_free_installer_contract`, `python_free_repository_contract` and
  `python_free_cutover_contract` pass. All 10 tests in `shell_wrapper_contract`,
  `plugin_helper_shell_contract` and `installer_staging_contract` pass. No assertion
  was weakened; new regressions are included in these focused totals, not added to
  the historical full-workspace count.
- `powershell.exe -NoProfile -ExecutionPolicy Bypass -File
  .\scripts\Test-NativeWorkspace.ps1 -SkipUnitTests` exited 0 with the external Cargo
  directory, two build jobs and offline Cargo enabled. It rechecked build, formatting,
  Clippy, diff whitespace, PowerShell dry-runs and Git Bash syntax/dry-runs. It explicitly
  reports `partial_checks_only`; the already completed workspace tests were not rerun.
- Changed code files decode as strict UTF-8. The two test files contain 130 and 245
  lines at verification. Cache absence was reconfirmed. Existing live PID 49728 had a
  seven-second heartbeat and no stop/restart markers; no production changes occurred.
