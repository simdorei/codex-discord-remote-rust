# Installed Codex selection and tool checks

On Windows, start and restart select the highest observed installed semantic version. Modification time is not version precedence. An explicit `CODEX_EXE` remains a pin, including a valid older version. A broken pin, unknown candidate version or incomplete highest version holds startup; it never silently downloads or downgrades. A complete copy of the same highest version is allowed. Installation preserves an existing pin unless the user explicitly changes it.

The validated Windows pin must identify the native `codex.exe`. An installer pin to a `.cmd`, `.bat` or `.ps1` launcher now holds before plugin commands or configuration changes because that script does not identify the executable it will run. Pass the actual executable instead. Generic native command helpers still support script launchers; this restriction applies to validated Codex selection.

`cdr-runtime --admin discover-codex --repo-root ROOT` preserves the path-only output. Add `--format json` for source, version, candidate rejections and bundle hashes. The installer passes that selection to every plugin command and checks the retained artifacts instead of resolving PATH again.

Discovery is bounded to 15 seconds including 3 seconds cleanup; startup shares one 45-second deadline including an 8-second cleanup reserve. Version probes use at most 2 seconds and 64 KiB per output stream. Failure to prove cleanup holds the request. Ordinary MCP terminal cleanup and the separate bot/MCP lifetimes are unchanged.

After initialize, selection receipt filesystem work runs in an owned worker within the remaining startup deadline. The parent compares the already resolved OS image path without another filesystem lookup. Receipt timeout or publication failure holds startup and reaps the initialized app-server using the reserved cleanup time; unconfirmed cleanup is reported explicitly.

## Fixed local-tools-v1 contract

This narrow contract checks local JavaScript execution and MCP tool inventory. It does not invoke official CUA outside an active model turn or certify native window access, browser URL policy checks, input actions, or operation while the desktop app is completely stopped. It also does not certify every installed plugin, account, model or external service. Missing requirements are not removed based on the installed inventory.

| Required tool | Owner/configuration | Read-only input | Success oracle |
| --- | --- | --- | --- |
| `node_repl.js` | `node_repl` MCP server in the selected HOME | Compute `6*7===42` and emit a fixed JSON marker | Successful MCP result containing the marker and boolean true |
| `cua_repl.js` | Official plugin identity, or the unique canonical server after verified managed registration | Inspect `mcpServerStatus/list`; no CUA call | Tool discovered, but invocation remains `NOT_CHECKED` with `requires_active_model_turn` |

Run `cdr-runtime --admin diagnose-codex-tools --repo-root ROOT --run-probes true` explicitly to exercise plain `node_repl` JavaScript only. Without `--run-probes true`, inventory alone is `NOT_CHECKED`. Missing required tool/plugin is `FAILED`; incomplete authentication/observation, changed configuration or a different process is `UNVERIFIED`. Successful JavaScript does not promote the overall result to `VERIFIED` while the CUA entry remains `NOT_CHECKED`.

The official-only `diagnose-official-computer` command reports `inventory_state=VERIFIED` after an unambiguous valid catalog. Its native/browser/app-off states remain `NOT_CHECKED`. This command and the deprecated `diagnose-bundled-computer` alias do not accept `--run-probes`.

The general report identifies its scope as `javascript_and_mcp_inventory`. `native_desktop_state`, `browser_state` and `desktop_app_independence` remain `NOT_CHECKED`. Do not use either diagnostic as an app-independent desktop readiness gate.

Managed Windows registration selects standalone native transport for official `cua_repl` and preserves its installed surface declaration. It does not override generic `node_repl` transport or create a browser access host. Separately configured plugins may still use desktop-owned pipes. Operation with the desktop app fully stopped requires the real-turn acceptance cases in [Official Computer Use](headless-computer-use.md#target-and-required-acceptance), including reference-supported browser operations, URL policy observation, harmless actions, process ownership and cleanup. Keep policy checks enabled and distinguish a closed app window from terminated GUI-owned processes.

The diagnostic starts one separate ephemeral app-server with the selected executable and HOME, uses read-only policy, and closes it. Its result is scoped to that exact diagnostic process; it never marks the running Discord bot ready. Normal bot startup records CLI/initialize identity and `NOT_CHECKED` tools. Tool failure never selects an older CLI. No desktop-owned pipe, global configuration, auth data or plugin cache is copied or changed.
