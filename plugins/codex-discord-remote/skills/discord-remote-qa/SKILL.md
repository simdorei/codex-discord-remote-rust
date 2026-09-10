---
name: discord-remote-qa
description: Run focused QA for Codex Discord Remote, especially mirror mapping, context refresh, session mirror cursor priming, steering suppression, archive lock retry, and deployment readiness checks.
---

# Codex Discord Remote QA

Use this skill when validating local changes or deciding whether the remote is ready to push or deploy.

## Standard QA

Run:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File plugins/codex-discord-remote/scripts/qa-smoke.ps1
```

The smoke script covers:

- `git diff --check`
- installer dry-run without dependency or `.env` changes
- Python compile checks for core remote modules
- the main Discord bot and mirror cleanup test suites

## Runtime QA Notes

- A passing unit suite is not enough for live Discord routing claims.
- For live QA, verify bot logs show actual gateway receipt and final send lines.
- Background session mirroring should still tail archive-recommended targets and catch up backlog in bounded batches; mapped Discord ask delegation remains separately controlled by active mirror output priming.
- Cursor priming should advance to the current session file EOF before delegated mirror output.
- Session mirror delivery should claim a mirrored event only after Discord send succeeds; send failures should leave the cursor/event retryable.
- Long Discord sends should show `[part/total]` markers and `discord_delivery_*` log lines for every chunk.
- Busy mapped-thread prompts should expose explicit `Steer now`, `Queue next`, and `Ignore` controls. `!retract`/`/retract` should remove only still-queued asks and never interrupt the active turn.
- Old ask output after a steering handoff should be suppressed when a newer steering relay already sent the final answer.
- Ask and steer delivery should use the resident `codex app-server` transport by default; IPC/UI/subprocess fallback should not be used silently.
- App-server approval and request-user-input server requests should be answered through the resident JSON-RPC request id. If that fails, surface the failure instead of substituting a legacy path.
