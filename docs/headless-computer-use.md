# Headless Windows Computer Use

The Rust Remote can expose its existing Windows computer engine as the explicit `cdr_computer` stdio MCP server. It does not require the Codex desktop application process, a browser access host, OAuth relay, or network listener. The installed Codex CLI and its matching Code Mode host are still used by Remote for normal model work.

## Enable for the Remote process

Set `DISCORD_HEADLESS_COMPUTER_USE=1` in the environment used to start the Rust Remote, then perform a normal managed restart. The default is disabled. `0`, `false`, or an absent setting preserve the existing configuration; invalid values hold startup. Windows is required.

When enabled, Remote registers `mcp_servers.cdr_computer` with the absolute current runtime executable and the sole argument `--computer-mcp`. The endpoint is handled before bot configuration, Discord connection, database access, or the runtime instance lock. Normal app-server restarts retain this explicit configuration. No user Codex configuration file is rewritten.

This is a separately named, operator-selected desktop provider. It does not implement `cua_repl.js`, forge browser host credentials, change bundled plugin settings, grant native app approvals, or serve as an automatic fallback after another provider denies an action. Existing authorization and policy refusals must be respected. For an explicitly authorized desktop task on this PC, select the `cdr_computer` tools.

## Install as a standalone helper

An existing Remote binary can use the new provider without replacing the bot executable. Build this revision, copy the verified executable to a stable versioned helper path, and register it through the installed Codex CLI:

```powershell
codex mcp add cdr_computer -- 'C:\absolute\versioned\cdr-computer-mcp.exe' --computer-mcp
```

Inspect an existing registration before changing it. This registration is an explicit operator choice and is available to new Codex app-server sessions for the same Windows account. Refresh MCP configuration or start a new session after registration. Do not enable the Remote environment switch against an older bot binary that does not contain this entrypoint. To remove only this registration, use `codex mcp remove cdr_computer`.

The Windows user desktop must be logged in, unlocked, and interactive. Application-process independence does not imply Windows-service/session-0 or locked-desktop support.

## Tool workflow

1. Call `computer_list_windows` and choose the intended window from the returned identities.
2. Activate that window, then call `computer_screenshot`.
3. Inspect the returned PNG and use its `observation_id` with one matching input operation. Coordinates are relative to that window's screenshot.
4. Take a new screenshot before each additional input. The engine consumes observations once, expires them after 30 seconds, and verifies window, process, title and geometry before acting.
5. Call `computer_stop` when finished. It permanently stops that MCP connection; a new connection is required to start another control session.

`computer_launch_app` opens an owned temporary Chrome guest window or Notepad instance. Stop and connection shutdown clean up applications launched by that MCP process. Use the list/activate workflow for existing signed-in windows. Never type secrets or OTPs, or use desktop control to bypass an access restriction.

Screenshots return a native MCP image block plus metadata. Image bytes are not duplicated into JSON text. The endpoint accepts only its bounded, typed computer requests; it offers no shell or arbitrary filesystem tool.

## Verification scope

Focused protocol, configuration, cross-process observation, and native desktop checks are recorded separately after execution. Catalog discovery alone does not establish that screenshot capture or input works. Native `cua_repl` and its browser URL policy remain independent capabilities and are not certified by these checks.