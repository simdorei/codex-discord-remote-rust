# Installed Codex selection and tool checks

On Windows, start and restart select the highest observed installed semantic version. Modification time is not version precedence. An explicit `CODEX_EXE` remains a pin, including a valid older version. A broken pin, unknown candidate version or incomplete highest version holds startup; it never silently downloads or downgrades. A complete copy of the same highest version is allowed. Installation preserves an existing pin unless the user explicitly changes it.

The validated Windows pin must identify the native `codex.exe`. An installer pin to a `.cmd`, `.bat` or `.ps1` launcher now holds before plugin commands or configuration changes because that script does not identify the executable it will run. Pass the actual executable instead. Generic native command helpers still support script launchers; this restriction applies to validated Codex selection.

`cdr-runtime --admin discover-codex --repo-root ROOT` preserves the path-only output. Add `--format json` for source, version, candidate rejections and bundle hashes. The installer passes that selection to every plugin command and checks the retained artifacts instead of resolving PATH again.

Discovery is bounded to 15 seconds including 3 seconds cleanup; startup shares one 45-second deadline including an 8-second cleanup reserve. Version probes use at most 2 seconds and 64 KiB per output stream. Failure to prove cleanup holds the request. Ordinary MCP terminal cleanup and the separate bot/MCP lifetimes are unchanged.

## Fixed local-tools-v1 contract

This narrow contract checks local JavaScript execution and computer-use surface inventory. It does not certify native window access, browser URL policy checks, input actions, or operation while the desktop app is completely stopped. It also does not certify every installed plugin, account, model or external service. Missing requirements are not removed based on the installed inventory.

| Required tool | Owner/configuration | Read-only input | Success oracle |
| --- | --- | --- | --- |
| `node_repl.js` | `node_repl` MCP server in the selected HOME | Compute `6*7===42` and emit a fixed JSON marker | Successful MCP result containing the marker and boolean true |
| `cua_repl.js` | `unified-computer-use` plugin; exact unique configured server | `cua.getState()`, inspect its returned value, then completion summary | Both observation and marker calls succeed; state was observable and completion succeeds |

Run `cdr-runtime --admin diagnose-codex-tools --repo-root ROOT --run-probes true` explicitly. Without `--run-probes true`, inventory alone is `NOT_CHECKED`. Missing required tool/plugin is `FAILED`; incomplete authentication/observation, changed configuration or a different process is `UNVERIFIED`. Only successful actual calls for all required entries yield `VERIFIED`.

The report identifies this scope as `javascript_and_surface_inventory`. Even when that scope is `VERIFIED`, `native_desktop_state` and `desktop_app_independence` remain `NOT_CHECKED`. A successful `cua.getState()` can coexist with a later native Chrome acquisition failure. Do not use this report as an app-independent desktop readiness gate.

The Windows startup override selects standalone transport only for `node_repl`. A plugin such as `cua_repl` can specify its own desktop-owned pipe in its installed manifest, independently of the bot's inherited environment. Removing desktop variables from the bot parent does not remove those plugin settings. Operation with the desktop app fully stopped requires a separate native end-to-end test, including URL policy observation and a harmless action, with process ownership and cleanup verified. Keep policy checks enabled and distinguish a closed app window from a terminated desktop process.

The diagnostic starts one separate ephemeral app-server with the selected executable and HOME, uses read-only policy, and closes it. Its result is scoped to that exact diagnostic process; it never marks the running Discord bot ready. Normal bot startup records CLI/initialize identity and `NOT_CHECKED` tools. Tool failure never selects an older CLI. No desktop-owned pipe, global configuration, auth data or plugin cache is copied or changed.
