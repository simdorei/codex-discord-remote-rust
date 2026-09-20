# Manual Reserve

Reserve is an explicit model choice. The bridge does not automatically enter Reserve after a usage-limit error, restore the previous model, or poll capacity to resume a rejected request.

```text
!settings --model
!settings --model gpt-reserve
!settings
```

The first command lists model choices, the second selects Reserve, and the last shows settings. Reserve selection retains capacity checks and verification of the applied model, reasoning effort, and speed. To leave Reserve, select another available model explicitly.

Old `--auto-reserve` and slash `auto_reserve` requests are rejected as a whole, including previously saved actions and requests mixing an old option with a valid setting. A rejection does not partially apply the other settings.

## Usage limits and restart recovery

A definite usage-limit rejection records an execution hold. Changing the model or restarting the bridge does not replay that rejected request. Choose the desired model and submit a new request when ready. `!runners` lists execution holds.

Automatic Reserve policy retirement runs once before startup queue recovery. It retains policy evidence and converts existing policy modes to manual. Pending work requires positive, canonical acceptance evidence; unknown or missing acceptance is held. Existing turn ownership, cancellation, unknown-write, and message-order checks remain in force.

Back up existing state before an upgrade. Deployments must stop the old writer before taking the authoritative SQLite backup, install the reviewed candidate, and verify its process identity, advancing heartbeat, and Discord readiness. The Windows cutover scripts preserve phase state and refuse ambiguous ownership or failed recovery. They require deployment-specific reviewed inputs; they are not an unattended upgrade command.

## Saved replies

An execution hold and a saved final reply are separate. The operator-only `authorize-saved-final` command can authorize delivery of an already saved reply after reviewing the exact job, thread, turn, destination, original request, and error receipt. It does not start a turn or mark the original request as ordinarily accepted.

The grant freezes the exact rendered chunks and verifies their delivery receipts. Existing or ambiguous completion attempts require reconciliation before any retry. Other held replies receive no authorization from this operation.

For a stuck active bot, see [emergency restart on Windows](force-restart.md). Force restart interrupts the selected bridge and its owned app-server tree; it does not override execution holds or establish that interrupted work succeeded.
