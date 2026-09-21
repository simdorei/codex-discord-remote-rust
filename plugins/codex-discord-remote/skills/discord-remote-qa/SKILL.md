---
name: discord-remote-qa
description: Run focused QA for Codex Discord Remote, especially mirror mapping, context refresh, session mirror cursor priming, steering suppression, archive lock retry, and deployment readiness checks.
---

# Codex Discord Remote QA

Use this skill when validating local changes or deciding whether the remote is ready to push or deploy.

## Standard QA

Use the [evidence reuse and retry rules](../intent-driven-qa/SKILL.md#reuse-evidence-and-retry-only-affected-work) for existing checks as well as new tests. Share existing execution results with other QA/review skills. Select checks from the changed behavior and required release scope; do not rerun the full smoke command when valid evidence already covers it, or merely to repair a reviewer's evidence-access problem.

When full workspace QA is needed and matching evidence is missing or invalidated, run:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File plugins/codex-discord-remote/scripts/qa-smoke.ps1
```

The wrapper calls `scripts/Test-NativeWorkspace.ps1`, which covers:

- `git diff --check`
- Rust formatting, locked workspace build, workspace tests, and Clippy
- PowerShell installer and Discord setup dry-runs without installation or `.env` changes
- Git Bash syntax checks and installer/setup dry-runs

`-SkipUnitTests` produces partial checks only. Even a complete workspace PASS does not establish live Discord behavior or prove that the running deployment matches the tested source. Report newly run and reused results separately, and verify only the runtime observations relevant to the claim below.

Pro review is not a mandatory QA stage. When requested or when its automatic-consultation criteria apply, use `ask-chatgpt-pro` and its existing Chrome/connector recovery procedure. A connection or evidence-access failure does not invalidate unrelated test results; resume that review stage after recovery.

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
