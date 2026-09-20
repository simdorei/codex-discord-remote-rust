# Emergency restart on Windows

In an authorized Discord channel, send **`!restart_codex force`** (also `!restart_codex --force` or `!force_restart`). The force command has its own bounded gateway receiver, bypasses normal and sealed drain admission, and does not call the app-server before launching the independent Windows controller. Existing channel/user access rules and message deduplication still apply. Bot-authored force requests are rejected; historical messages never replay a force restart.

Run from the bridge folder:

```powershell
.\codex-discord-rust-restart.ps1 -Force
```

Or run `codex-discord-force-restart.cmd`, or choose **Force restart bot + app-server (interrupt work)** in the tray menu.

This immediately terminates the verified Rust bridge and its app-server process tree, even when a turn or approval is active. It cancels an ordinary watchdog that is holding the control lock while waiting for restart readiness. It does not wait for quiet time, a drain acknowledgement, an app-server response, or active work to finish. The brief post-termination wait verifies OS process exit and the new runtime heartbeat.

The normal `Restart bot` menu and bare `!restart_codex` retain their graceful behavior. `-Immediate` alone is also graceful; use **`-Force`** to interrupt work. The local command and tray work without Discord or app-server responsiveness. A disconnected Discord gateway cannot receive chat commands, so use the local command/tray if the bot itself cannot receive messages. A restart may interrupt the acknowledgement message; the completion receipt establishes the outcome.

If the command itself runs inside the bot's tool process tree, it hands off to a hidden, independent Windows worker before termination. `force_restart_handed_off` acknowledges that handoff; the completion receipt, rather than the requesting tool surviving its own termination, proves the restart finished.

Only the selected repository's verified process identities and descendants are terminated. Other Codex desktop instances and other bots are outside that tree. A stale expected identity fails without killing a replacement. Active binary deployment remains protected; it is not an active conversation.

Queue databases and conversations are preserved. Interrupted work is **not** reported as successfully completed or blindly replayed; normal queue recovery reconciles persisted turns on startup. A force-restart receipt records the interrupted and replacement identities in `.codex_discord_rust.force.completed`. Superseded restart/drain markers are retained in `maintenance_backups/force-restart/<operation>/`. An interrupted force launch is resumed by the scheduled watchdog using `.codex_discord_rust.force.launch` and the same durable child tracking as normal restart.

`-Force -DryRun` verifies the target without terminating or launching it. `discord_launcher.log` contains `force_cancel_restart_wait`, `force_stop`, and `force_restart_completed` events.
