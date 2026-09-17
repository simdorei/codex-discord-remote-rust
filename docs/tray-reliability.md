# Windows tray reliability

The tray is an optional user-session UI. Observing status or recovering the UI
never authorizes a bot start, stop, request replay, or maintenance retry.

## Status and UI errors

The tray distinguishes a confirmed stopped runtime from an unavailable status.
Malformed or duplicate PID fields, failed lock reads, and incomplete process
observations produce `unknown`. An unknown observation clears the previously
cached PID and restart grace period. Diagnostic logging is bounded and cannot
terminate the UI. Recovery is logged only after both a fresh observation and UI
publication succeed.

## Automatic tray launch

The watchdog captures the exact newly started runtime identity without invoking
the tray helper inside the maintenance launch budget. Only an ordinary successful
watchdog invocation can attempt UI launch after releasing the control lock.
Dedicated maintenance, recovery, preparation, and completion paths do not launch
the UI. Exact completion receipts may preserve explicit UI launch intent for a
later ordinary watchdog invocation.

A create-only file consumes at most one automatic UI attempt per exact runtime
identity. Empty or partial attempt records and uncertain process-creation results
are not automatically retried. A valid child PID means that spawning was requested;
it does not prove that Explorer is displaying an icon. User-session checks and a
per-root named mutex prevent inappropriate or duplicate UI ownership.

## Offline verification

The revision 5 verification ran 847 tests successfully, with no failures and eight
pre-existing ignored fixtures. This includes 29 tray tests and six UTF-8/formatting
runner tests. Unchanged restart/recovery coverage is separately supported by the
revision 4 run of 63 passing tests and one pre-existing ignored fixture; these are
not represented as newly executed revision 5 tests.

Workspace/all-target Clippy with `-D warnings`, bounded whole-workspace formatting,
PowerShell parsing, installer preparation, and selected maintenance regressions
passed in the reviewed source snapshot. Local review manifests and raw receipts
remain operator evidence, not portable deployment tickets.

These offline tests use copied product functions and fake process/UI providers.
They do not establish actual Explorer visibility, post-logon behavior, or continuous
UI residence. A deployment must distinguish source publication, tray process
startup, and actual icon observation. Main-branch integration verification is a
separate step from the original revision 5 snapshot.
