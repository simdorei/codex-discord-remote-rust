# Official Computer Use through Rust Remote

> Candidate integration; no production cutover. An official native window-list operation succeeded in a real Codex turn. Chrome control remains blocked, and desktop-app-off operation has not been tested.

Rust Remote keeps Discord orchestration and registers the installed official `cua_repl` server with its own Codex app-server. The candidate replaces the runtime's custom `cdr_computer` registration and removes its `--computer-mcp` entrypoint. The external `cdr-remote-agent` MCP and shared remote-agent computer code remain separate and unchanged.

Native operations use the official CUA/Sky helper through an active model turn and that turn's normal permissions and approvals. The managed registration exposes the `computer` surface only. This does not make Chrome exempt from browser URL policy.

## Installed runtime and configuration

`local_computer::configure_current` reads the resolved `CODEX_HOME` and selects the highest numeric installed version under:

```text
CODEX_HOME/plugins/cache/openai-bundled/unified-computer-use/<numeric-version>/.mcp.json
```

Selection uses the installed manifest's Node, CUA entry point, access guard, inner Codex CLI and `node_repl` paths. It validates the required files and bundle relationships before changing the launch arguments. An unreadable, incomplete or ambiguous newest installation holds startup; it does not silently select an older version. Selection performs no automatic download, installation or upgrade. The outer Codex CLI continues to use Remote's existing executable-selection mechanism.

The registration is passed as configuration arguments to this app-server. It sets `SKY_CUA_NATIVE_PIPE=0`, clears the desktop native-pipe directory, and sets `CUA_REPL_ENABLED_SURFACES=computer`. The official guard, approval metadata and trusted-service configuration are preserved. The generic app-server constructor no longer forces a transport override on `node_repl`.

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

The candidate replaces the custom provider registration with official native-only registration. The Windows build and 32 focused Rust tests passed, as did the PowerShell recovery fixture suite. Workspace Clippy with warnings denied and the repository's Windows formatting harness (11 packages, 573 target roots) also passed. The built candidate also registered the installed official server and observed its four tools, then closed its owned app-server. The [validation receipt](official-cua-validation-20261115.json) records these results separately from the real-turn feasibility observations. These checks do not satisfy the Chrome or GUI-app-off acceptance requirement.

Production cutover remains blocked by the unmet Chrome requirement. Required acceptance must cover the intended native and Chrome operations through real turns and the normal approval flow, plus the requested GUI-app-off scenario when that can be tested without interrupting existing work. A catalog-only diagnostic cannot satisfy this gate. Do not automatically start/stop the GUI, disable policy checks or switch back to the custom Rust engine to turn an incomplete result into a pass.

Any later operational replacement must use the existing managed restart and launch-compatibility procedures. Existing approval and compatibility records are not removed or rewritten by this integration.

## Historical custom provider

[The historical computer-use record](computer-use-history.md) describes the earlier Rust provider and remains unchanged apart from its superseded notice. The [previous bundled-provider acceptance record](https://github.com/simdorei/codex-discord-remote-rust/blob/4d081cb467eeb9ffd76db67e675f0e95a4cd6a39/docs/headless-computer-use.md) is also historical. Its tool catalog, screenshot/input rules and acceptance results do not verify official CUA.
