---
name: archive-used
description: Archive Codex Discord Remote threads whose `used` value in `!list` or `/list` output is at or above a user-provided number-plus-M threshold, using the UUID printed in that same list row as the archive target. Use when the user asks to bulk archive high-used threads, archive everything over `used xxxM`, or clean up listed Codex threads based on the `used` column.
---

# Archive Used

Archive active Codex threads by comparing the `used` column shown in the bridge list output against a threshold.

## Workflow

1. Get the threshold. If the user did not provide one, ask for the `used` cutoff in `<number>M` form. Accept lowercase `m` as uppercase `M`.
2. Use the freshest list output available. If Codex has shell access in this repo, run `py -3 .\codex_desktop_bridge.py list --db-root`; otherwise use the user's current `!list` or `/list` output, or ask them to run `!list`.
3. Parse only thread rows shaped like:
   `  3 | workspace | idle | ctx ... | used 12.3M | rec archive | model ... | uuid-from-list | timestamp | title`
4. Compare against the `used` column only. Do not use `ctx`, `rec archive`, RAM, disk, or process memory as substitutes.
5. Treat `m` or `M` as millions of tokens. Decimal `M` values are allowed; `k` and plain numbers are below any `M` threshold.
6. Select rows whose `used` value is greater than or equal to the threshold.
7. Skip non-idle rows (`busy`, `waiting-*`, etc.) and the selected row marked with `*` unless the user explicitly says to include them.
8. Archive by the UUID printed in the same list row. Copy it exactly; do not substitute a workspace name, selected thread, title, or guessed id. Prefer the local bridge command when shell access is available:
   `py -3 .\codex_desktop_bridge.py archive --thread-id <uuid-from-list>`
   If operating only through Discord, use:
   `!archive <uuid-from-list>`
9. If the UUID column is missing or ambiguous, refresh the list and stop if it is still not visible.
10. After archiving, run `py -3 .\codex_desktop_bridge.py list --db-root` or `!list` again and report which selected UUIDs disappeared from the active list.

## Safety Rules

- `archive` is not permanent deletion. Never run `!delete_archive` or `!confirm_delete_archive` for this skill.
- If the user clearly asked to execute now, archive idle candidates without asking for a second confirmation.
- If the candidate set includes only skipped rows, explain what was skipped and why.
- If any archive command fails, stop and surface the exact failure instead of continuing blindly.
- If the list output is truncated or a candidate row cannot be parsed confidently, refresh the list and continue from the complete output.

## Output

Before running archive commands, show a compact candidate summary:

```text
threshold: used >= <threshold>
archive: uuid-from-list-1 used 123.4M idle title...
skip: uuid-from-list-2 used 130.0M busy
```

After verification, report the archived count and any skipped or failed rows.
