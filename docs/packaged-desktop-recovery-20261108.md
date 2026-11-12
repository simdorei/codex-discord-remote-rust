# Preserve package identity during desktop recovery

The recovery paths launched `app/ChatGPT.exe` directly. On the affected Windows installation, the resulting desktop log reported `failureReason=no-package-identity`, and a read-only `GetPackageFullName` probe confirmed that the running desktop process had no package identity. Registration on disk alone did not make that process packaged.

Both desktop-only and desktop-plus-tools recovery now resolve the registered desktop app from its package manifest and activate its app user model ID through Windows. The activation manager is created out of process so its lifetime is independent of the recovery worker. Before stopping the previous desktop, recovery checks activation availability. It verifies the desktop process's executable path and package full name before restarting the bot or reporting host recovery.

The package identity and app ID are part of the prepared recovery plan and are rechecked before activation. The activation process handle stays pinned while waiting for an app-server backend. If Electron replaces that process during startup, only a direct child created after the pinned activation process can qualify; its path, creation time and package identity must agree. Pre-existing unrelated desktops, reused process IDs, renderers, unpackaged processes and ambiguous hosts are rejected. There is no raw-executable fallback. The new activation helper is hashed into the one-use recovery request; a changed helper cannot dispatch a restart.

The first live activation was reported as failed by the initial process verifier. Subsequent read-only inspection found a replacement desktop with the correct package identity and backend, while the activation process had already exited. That failure receipt is preserved. The follow-up resolver handles a verified direct relaunch and waits for its backend; it does not turn a matching but unrelated running desktop into evidence of successful activation.

No firewall permission, native URL policy, browser access verifier, or cross-session identity rule is changed. A Windows Security firewall dialog was observed while Chrome's URL detection failed. Whether correct package activation resolves that dialog or the URL failure requires separate live verification; neither is established by these tests.

Validation on Windows:

- The original launcher failed the new regression fixture because it attempted a raw desktop launch.
- 27 focused Rust integration tests passed across `recover_host_identity_contract`, `recover_bot_receipt_contract`, `writer_recovery_contract`, and `recover_controller_lifecycle_contract`.
- The package helper fixture compiles the native interop and reads the test process identity, while faking app activation and process mutation. It checks registration changes, direct relaunch, delayed backend startup, wrong or absent package identities, stale/reused IDs, unrelated hosts, renderer exclusion and handle disposal on failure.
- Actual read-only registration and activation-manager preflight passed. The existing desktop process's package identity was confirmed absent.
- Changed-file formatting and `git diff --check` passed. No full-workspace CI result is claimed.

API references: [Windows application activation](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-iapplicationactivationmanager-activateapplication), [activation manager lifetime](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-iapplicationactivationmanager), [process package identity](https://learn.microsoft.com/en-us/windows/win32/api/appmodel/nf-appmodel-getpackagefullname).
