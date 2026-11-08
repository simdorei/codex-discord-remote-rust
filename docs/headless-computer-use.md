# Computer Use bundled with Rust Remote

Computer Use is built into `cdr-runtime.exe`. On Windows, starting Rust Remote automatically registers its `cdr_computer` provider with the app-server owned by that runtime. The MCP child runs the **same executable** with `--computer-mcp`; distributing the Rust executable includes the computer engine.

There is no separate helper installation, `codex mcp add` step, or enable environment variable for normal use. The built-in registration is passed to this runtime's app-server as command-line configuration, so it does not rewrite the user's global Codex settings. App-server restarts retain that registration.

The Codex desktop application does not need to be running. The installed Codex CLI and matching Code Mode host are still used for model work. Windows must remain logged in, unlocked, and interactive.

## Start and configure

Start Rust Remote using its normal supported launcher. Computer Use is enabled by default on Windows. `DISCORD_HEADLESS_COMPUTER_USE=0` or `false` explicitly disables this provider in the Remote's app-server, including an older global registration under the same name. An absent or blank value uses the platform default; `1` and `true` enable it on Windows. Invalid values hold startup. Other platforms default to disabled and reject explicit enablement.

The normal bot startup and the bundled diagnostic both call `local_computer::configure_current`, which selects `std::env::current_exe()`. The stdio entrypoint runs before bot environment loading, Discord connection, database access, or the runtime instance lock. The computer child therefore does not start a second bot.

MCP is the internal connection to Codex's app-server; it is part of this Rust distribution. The separate process uses the same executable and is cleaned up with its MCP connection.

## Read-only diagnostic

Run the bundled diagnostic from the intended installed or candidate Rust executable:

```powershell
.\cdr-runtime.exe --admin diagnose-bundled-computer --repo-root 'C:/path/to/runtime'
```

Optional `--codex-exe` and `--codex-home` arguments select the installed CLI and profile explicitly. This starts an isolated app-server using the same built-in provider configuration as normal bot startup, creates an ephemeral read-only thread, verifies the exact tool catalog, and calls the native window-list operation. It reports a window count, without window titles or IDs. It does not request a model turn, send Discord messages, launch applications, or type/click. A successful diagnostic is narrower than a complete browser workflow.

## Tool workflow

For an authorized desktop task, the model uses the built-in `cdr_computer` tools:

1. List windows and select the intended window.
2. Activate it, then capture a fresh screenshot.
3. Inspect the image and use its observation ID for one input operation. Coordinates are relative to that screenshot.
4. Capture again before the next input. Observations expire after 30 seconds and are consumed once. Native checks bind input to the window, process, title, geometry and focus.
5. Stop the control session when finished. A stopped MCP connection requires a new connection before further control.

`computer_launch_app` opens temporary owned Chrome guest or Notepad windows. Stopping that MCP connection closes only its owned applications. Use list/activate for an existing signed-in window that should remain open.

The engine exposes 11 typed desktop tools, without an arbitrary shell or filesystem endpoint. Screenshot bytes are returned in a native image block. Coordination between these Rust MCP processes invalidates old observations when another participating controller changes the desktop.

Bundled `cdr_computer` is the provider for this integration. Native `cua_repl.js` and its browser-host permissions remain separate capabilities; this implementation does not grant or bypass another provider's denied access.

## Earlier standalone experiment

The first acceptance run used a separately copied helper. Its results are preserved in [the historical acceptance record](computer-use-history.md). That installation method is superseded by this same-executable integration.

When an existing deployment has that earlier registration, the bundled runtime overrides it with its own executable for its own app-server. Remove the old global registration only after the intended bundled runtime is verified in service and the exact registration is confirmed to be the old helper. Existing artifact-pinning and managed restart requirements still apply to an operational binary replacement.
