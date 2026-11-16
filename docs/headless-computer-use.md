# Official Computer Use through Rust Remote

> Candidate integration; no production cutover. An official native window-list operation succeeded in a real Codex turn. Chrome control remains blocked, and desktop-app-off operation has not been tested.

Rust Remote keeps Discord orchestration and registers the installed official `cua_repl` server with its own Codex app-server. The candidate replaces the runtime's custom `cdr_computer` registration and removes its `--computer-mcp` entrypoint. The external `cdr-remote-agent` MCP and shared remote-agent computer code remain separate and unchanged.

Native operations use the official CUA/Sky helper through an active model turn and that turn's normal permissions and approvals. The managed registration preserves the installed manifest's declared surfaces (`browser`, `computer`, or both). Enabling the browser API does not establish a working browser host or make Chrome exempt from browser URL policy.

## Target and required acceptance

The target is the computer and browser functionality that actually works in the user's ChatGPT desktop app on the same PC and browser profile, also working through the Discord bot while the GUI and its GUI-owned background processes are fully stopped. This includes reference-supported browser tab and page-element operations; native-only registration is not the acceptance target.

Record paired reference-app and bot results for the same harmless test app/page, with an observable outcome such as entered text or a changed test control. A baseline failure is a recorded limitation, not evidence that the bot passed. Use real model turns and the normal official permission flow. Do not substitute a catalog, window count, mock, or a different provider for a successful action.

| ID | Required check | Pass condition | Current evidence |
| --- | --- | --- | --- |
| A01 | Reference app on the same PC/profile | Record supported native actions, Chrome actions, and tab/DOM operations, including the app/tool versions and expected visible results. | Not recorded. |
| A02 | Bot native actions | Read the test app, click and enter text, then observe the expected state through a real bot-owned turn. | Historical window enumeration only; action acceptance not checked. |
| A03 | Bot browser parity | Complete reference-supported Chrome navigation/read/click/input and tab/DOM operations with normal URL policy and approvals. | Historical browser-host and Chrome URL-policy blockers; preserving manifest surfaces does not resolve them. |
| A04 | Fresh GUI-app-off execution | Confirm GUI and GUI-owned background/helper processes are absent, start a fresh isolated bot-owned app-server/tool chain, then repeat A02/A03 without reopening the GUI. | Not checked; a helper launched while the GUI is running is insufficient. |
| A05 | Repair semantics | Official `!repair` acknowledges only a confirmed JS-session reset and leaves actual native/browser/app-off readiness unverified. Preserve admission, timeout and generation protections. | Validate with the focused repair contracts; actual tool availability still requires A02/A03. |
| A06 | Exact candidate and regressions | Bind source/artifact identities to the tested candidate and pass applicable Windows, release/package and platform CI. Preserve Discord, external MCP and user configuration. | See the final PR's exact-SHA checks; earlier PR results do not cover new commits. |

All applicable rows are merge requirements. Missing or blocked action evidence keeps the candidate in draft. The existing explicit full-tool `!recover` operation relaunches the ChatGPT desktop; its successful fixture or live execution cannot satisfy A04. Preserve that operation's existing meaning and test any standalone recovery path separately.

An acceptance receipt must record candidate commit and dirty state, tested runtime executable SHA-256/build provenance, outer/inner CLI and official bundle identities (including manifest hash), reference app/browser versions and profile identifier, GUI/process ownership observations, start/end timestamps, requested actions, observed outcomes, and raw-log locations. Redact secrets. If a record predates the final commit, identify its tested tree/artifact and demonstrate the covered source is unchanged; do not infer identity from dates alone. The older [feasibility receipt](official-cua-validation-20261115.json) lacks that candidate binding and is not a release acceptance receipt.

## Installed runtime and configuration

`local_computer::configure_current` reads the resolved `CODEX_HOME` and selects the highest numeric installed version under:

```text
CODEX_HOME/plugins/cache/openai-bundled/unified-computer-use/<numeric-version>/.mcp.json
```

Selection uses the installed manifest's Node, CUA entry point, access guard, inner Codex CLI and `node_repl` paths. It validates required files, Node/NodeREPL and launcher/guard relationships, and the declared surfaces before changing launch arguments. The inner CLI is the readable absolute file declared by that manifest; this is not an independent proof of outer/inner CLI version compatibility. An absent, unreadable, incomplete or ambiguous newest installation rejects CUA registration; it does not silently select an older version. Selection performs no automatic download, installation or upgrade. The outer Codex CLI continues to use Remote's existing executable-selection mechanism.

Normal startup and diagnostics use `configure_available`: if requested managed CUA registration fails, they log `official_cua_unavailable`, mask `cua_repl` and the legacy computer provider with complete disabled transport definitions for this app-server, and continue ordinary bot/`node_repl` startup. The diagnostic records `computer_provider.mode=unavailable`, the registration error, and an overall `FAILED` tool result; successful ordinary JS work never claims CUA recovery. The masks do not rewrite user settings or modify other MCP providers. A subsequent runtime start re-evaluates the installation. Configuration errors unrelated to optional CUA registration still fail normally.

The registration is passed as configuration arguments to this app-server. It sets `SKY_CUA_NATIVE_PIPE=0`, clears the desktop native-pipe directory, and pins `OAI_SKY_WINDOWS_BIN` to the selected bundle's `@oai/sky/bin/windows/swift/<x64-or-arm64>/codex-computer-use-swift.exe`. This is the Swift helper observed under the reference desktop app on 5060; without the explicit path, Sky's direct transport defaults to a different Rust executable in the same bundle. Missing Swift artifacts reject registration instead of silently selecting that other implementation. Case-insensitive inherited helper overrides cannot replace the selected path. Diagnostics include `computer_provider.source.native_helper`; matching a package version alone does not prove matching native implementations.

The manifest's `CUA_REPL_ENABLED_SURFACES` value is retained rather than replaced with `computer`. Missing, empty or unsupported surface declarations fail before partial registration. The official guard, approval metadata and trusted-service configuration are preserved. The generic app-server constructor does not force a transport override on `node_repl`. Selecting the reference helper is a configuration correction; actual window read/input and GUI-off acceptance still require A01-A04.

Native transport independence does not create an independent browser access host. The browser service still needs a valid official host and its executor/pipe/verifying-identity context. This integration only forwards genuine host context; it does not create identities, disable the guard, or manufacture an approval. The installed package declares `browser,computer`, but browser-host availability and GUI-off browser operation remain separate unresolved acceptance requirements.

The existing `DISCORD_HEADLESS_COMPUTER_USE` parsing remains:

| Value | Effect |
| --- | --- |
| Absent or blank | Enable managed official registration on Windows; disabled on other platforms. |
| `1` or `true` | Enable on Windows; reject on other platforms. |
| `0` or `false` | Skip managed official registration and mask the legacy `cdr_computer` provider for this app-server. Independently configured official plugins remain configured as before. |
| Any other value | Hold startup with a configuration error. |

The enabled path also masks legacy `cdr_computer` registration for this app-server. Older CLI versions validate a transport even when it is disabled, so the mask supplies a complete disabled definition using the selected CLI with `--version`. This avoids the `invalid transport` startup failure reproduced with an empty profile and an `enabled=false` flag alone. It does not delete that registration from the user's global profile. Normal startup and the diagnostic use the same selection/registration function; resident app-server restarts retain the resulting configuration. User configuration and other MCP registrations are not rewritten.

The removed `cdr-runtime.exe --computer-mcp` invocation is rejected as an unknown argument with exit code 2, before bot configuration or Discord startup. The historical standalone helper is not an automatic fallback.

## Inventory diagnostic

Run the candidate executable from the intended runtime installation:

```powershell
.\cdr-runtime.exe --admin diagnose-official-computer --repo-root 'C:/path/to/runtime'
```

Optional `--codex-exe` and `--codex-home` select the installed CLI and profile explicitly. `diagnose-bundled-computer` remains a deprecated alias for this same inventory diagnostic; it no longer invokes the old Rust window-list tool. Neither command accepts `--run-probes`.

The diagnostic starts an isolated app-server, creates an ephemeral thread with `approvalPolicy=never` and `sandbox=read-only`, and inspects `mcpServerStatus/list`. It does not call a CUA tool, request a model turn, send a Discord message, capture the desktop or send input. A successful inventory report contains:

| Report field | Value after successful inventory inspection |
| --- | --- |
| `contract_id` | `official-computer-inventory-v1` |
| `inventory_state` | `VERIFIED` |
| `verification_scope` | `official_mcp_inventory_only` |
| `state` | `NOT_CHECKED` |
| `native_desktop_state` | `NOT_CHECKED` |
| `browser_state` | `NOT_CHECKED` |
| `desktop_app_independence` | `NOT_CHECKED` |

Tool discovery proves that the configured server exposes its tool catalog. It does not establish helper execution, a usable desktop, Chrome control or independence from a running GUI app. Errors retain their diagnostic stage; an outer MCP `isError=false` must not hide errors embedded in a CUA result.

### Browser-host observations

The inventory report additionally includes `browser_host_context`. On Windows its
`observations.launch` records the inherited, explicit-override and final child
environment field states from the actual app-server launch. Other platforms leave
that observation absent. The existing whole-set forwarding and partial/ambiguous
context guard remain unchanged.

`written_thread_requests` records the last 16 successfully written `thread/start`,
`thread/resume` or `thread/fork` requests. It reports only host-field states in their
`node_repl`/`cua_repl` environment overrides, timestamps and local request IDs. A
successful write is not a successful thread operation; a missing override does not
prove that the worker lacks inherited context. Raw host values and thread/turn IDs
are not retained in this diagnostic.

| State | Meaning at the named observation point |
| --- | --- |
| `UNKNOWN` | Not observed or not interpretable. |
| `ABSENT` | The inspected input contains no such field. |
| `EMPTY` | The inspected string is empty or whitespace. |
| `NONEMPTY` | A value exists; validity and authorization remain unverified. |
| `AMBIGUOUS` | Multiple aliases address the field; precedence is not inferred. |

During normal operation, recognized failed `node_repl`/`cua_repl` JS tool error
metadata adds a bounded `reported_tool_failures` observation and a redacted
`browser_host_observation` runtime log entry. It distinguishes missing host/turn
context, response-authentication failure, access denial and unavailable Chrome.
It does not scan successful results/page text or infer success from silence.
Unrecognized/wrapped errors and errors carried only in result content are not
classified. Recent requests are context, not proof that a failure belongs to one
of them; controlled live-turn evidence is still required for that correlation.

`worker_environment` stays `UNKNOWN`; `access_check` and `browser_discovery` stay
`NOT_CHECKED` in the inventory report. No extra host RPC or model turn is added.
This diagnostic patch does not supply a browser host, register a bot executor or
establish GUI-independent browser access. A supported provisioning/registration
contract has not yet been established; A03/A04 remain unresolved.

## Official `!repair` outcome

For `cua_repl`, `!repair` checks the existing idle/admission conditions and sends only `js_reset`. A confirmed reset is recorded as `reset_confirmed` with `verification_scope=js_session_reset_only`. Native desktop, browser and desktop-app independence remain `NOT_CHECKED`, and the response asks for an actual task in the next conversation turn. It does not call `cua.getState()` or send a completion-summary cell outside a model turn. Reset errors, malformed acknowledgements and uncertain timeouts retain the existing failure/hold behavior.

The legacy `node_repl` path retains its existing initialization/inventory checks. `!repair` does not switch providers to disguise an official CUA failure. The explicit desktop-wide `!recover` operation retains its separate restart semantics.

## Observed scope on 5060 — 2026-11-15 KST

The observations below used installed Codex CLI `0.167.0-alpha.4.1`, official plugin `26.1106.51428`, `@oai/cua-repl` `0.1.3`, and `@oai/sky` `0.7.18`. They are feasibility observations, not acceptance of a deployed replacement binary.

| Check | Observation and limit |
| --- | --- |
| Native window listing in a real turn | On 2026-11-14 **16:50:38–16:51:04 UTC** (2026-11-15 **01:50:38–01:51:04 KST**), `cua.getState()`, `cua.listWindows()` and completion returned successfully, with **6 windows**. |
| Official helper ownership | Helper PID `17196` was a child of the test trusted worker `18332`, in the test tree containing `node_repl` PID `23428` and owned app-server PID `2232`. It was not the existing GUI-owned helper. |
| Turn permissions | The successful test used the user's existing `dangerFullAccess` / `on-request` turn settings. This record does not establish that those settings are universally required, and the integration does not lower approval requirements to obtain a pass. |
| Earlier direct MCP invocation | A `mcpServer/tool/call` without an active model turn returned `spawn EPERM` from the restricted worker. It did not reproduce the successful real turn's execution context and is not the native acceptance method. |
| GUI app fully stopped | **NOT CHECKED.** The existing GUI app remained running. No test terminated it; helper ownership alone does not prove operation after GUI exit or after a fresh boot. |
| Official browser SDK | Returned `The browser access host is unavailable`. Independent DOM/tab control was not established. |
| Native Chrome attempt | On 2026-11-14 **17:01:49–17:02:15 UTC** (2026-11-15 **02:01:49–02:02:15 KST**), the normal high-risk `chrome.exe` MCP elicitation was accepted once. The operation then stopped at the Windows current-browser-URL confidence policy check. |
| Accessibility tree, screenshot and input | No successful AX/accessibility-tree retrieval, screenshot or input completion was verified by these tests. |
| Existing services | The live Discord bot, external MCP and browser were not stopped. Test-owned processes were closed separately. |

Changing the managed surface to `computer` does not solve the browser-host or Chrome policy failure. Official app approvals, URL checks and Escape handling remain unchanged. Do not treat registration success or the native window count as Chrome acceptance.

## Deployment gate

The previous native-only candidate passed a Windows build, 32 focused Rust tests, the PowerShell recovery fixture suite, workspace Clippy with warnings denied, and the repository's Windows formatting harness (11 packages, 573 target roots). That candidate's inventory diagnostic observed the four official tools and closed its owned app-server. The [historical validation receipt](official-cua-validation-20261115.json) keeps these results separate from the real-turn feasibility observations. These results do not verify the later surface-preservation/repair changes or satisfy the Chrome or GUI-app-off acceptance requirement. Use the final PR's candidate-bound checks for those revisions.

Merge and production cutover remain blocked until A01-A06 are satisfied. In particular, the reference-supported browser operations and fresh GUI-app-off scenario are mandatory. If an acceptance run cannot be performed without interrupting existing work, retain `NOT_CHECKED` and the draft state. A catalog-only diagnostic cannot satisfy this gate. Do not automatically start/stop the GUI, disable policy checks or switch back to the custom Rust engine to turn an incomplete result into a pass.

Any later operational replacement must use the existing managed restart and launch-compatibility procedures. Existing approval and compatibility records are not removed or rewritten by this integration.

## Historical custom provider

[The historical computer-use record](computer-use-history.md) describes the earlier Rust provider and remains unchanged apart from its superseded notice. The [previous bundled-provider acceptance record](https://github.com/simdorei/codex-discord-remote-rust/blob/4d081cb467eeb9ffd76db67e675f0e95a4cd6a39/docs/headless-computer-use.md) is also historical. Its tool catalog, screenshot/input rules and acceptance results do not verify official CUA.
