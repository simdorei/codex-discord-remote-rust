---
name: discord-remote
description: Operate the local Codex Discord Remote runtime, including status checks, restarts, archive-lock recovery, mirror mapping checks, and log triage. Use when the user asks about this Discord bridge, bot health, missing Discord messages, archive failures, mirror routing, or local deployment.
---

# Codex Discord Remote Operations

Use this skill for this repository and its local Windows bot runtime.

## Scope

- Treat the Discord bot runtime as a local service controlled by the repository scripts.
- Keep Discord bot tokens, guild IDs, channel IDs, and user-specific paths in `.env` or local state; do not hard-code secrets into plugin files.
- Prefer repo scripts and tests over ad hoc shell commands.
- Do not open extra Discord Web tabs for QA when an existing usable Discord tab/window is available.
- The plugin also packages `deep-interview` and `discord-remote-qa` skills.
- The bot exposes `/interview` and `!interview` for the clarification-first workflow.

## First Checks

1. Inspect the working tree before changing files:
   `git status --short --branch`
2. Check bot runtime status:
   `powershell.exe -NoProfile -ExecutionPolicy Bypass -File plugins/codex-discord-remote/scripts/status.ps1`
3. For archive lock failures, confirm the lock/process state before killing processes.
4. For mirror or steering issues, inspect `codex_discord_bot.log` and the mapped thread state before changing code.

## Operating Rules

- Do not revert user changes unless explicitly requested.
- Do not delete session, mirror, or Discord attachment data unless the user explicitly asks.
- For recursive deletion, resolve the absolute target path and confirm it remains under the intended repo or named target directory.
- If the remote is running, prefer a restart marker plus watchdog over force-killing the bot process.

## Useful Scripts

- `plugins/codex-discord-remote/scripts/status.ps1`: show git state, bot PID/process status, recent bot log tail, and bridge thread list.
- `plugins/codex-discord-remote/scripts/restart.ps1`: request bot restart through `.codex_discord_bot.restart` and run the watchdog.
- `plugins/codex-discord-remote/scripts/qa-smoke.ps1`: run deploy-oriented smoke checks.

## Sending Files To Discord

When the user asks to send, upload, attach, or deliver a file to Discord, start with the repo helper instead of saying there is no file-send path:

```powershell
py -3 .\send_discord_attachment.py --thread-ref <codex-thread-ref> --content-file .\caption.txt .\artifact.zip
```

- Prefer `--thread-ref` or `--work-thread` when sending to a mirrored Codex thread; use `--channel-id` only when the user gives a concrete Discord channel/thread id.
- Use `--content-file` for Korean or multiline captions so PowerShell encoding does not corrupt text.
- If the mirror target is stale, run `!mirror check` and `!mirror sync` before retrying.
- Do not use `!delete_archive` or archive deletion commands for file delivery.

## Interview Workflow

- For vague, broad, risky, or under-specified implementation requests, use the packaged `deep-interview` skill before coding.
- In Discord, use `/interview <request>` or `!interview <request>` to wrap the request in the Gajae-style clarification prompt.
- The interview must stop at a pending-approval ticket and wait for explicit user approval before implementation.
