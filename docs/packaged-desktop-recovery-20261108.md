# Preserve package identity during desktop recovery

The recovery paths launched `app/ChatGPT.exe` directly. On the affected Windows installation, the resulting desktop log reported `failureReason=no-package-identity`, and a read-only `GetPackageFullName` probe confirmed that the running desktop process had no package identity. Registration on disk alone did not make that process packaged.

Both desktop-only and desktop-plus-tools recovery now resolve the registered desktop app from its package manifest and activate its app user model ID through Windows. The activation manager is created out of process so its lifetime is independent of the recovery worker. Before stopping the previous desktop, recovery checks activation availability. It verifies the returned process's executable path and package full name before restarting the bot or reporting host recovery.

The package identity and app ID are part of the prepared recovery plan and are rechecked before activation. A changed registration, wrong executable, unpackaged replacement, or exited process is rejected. There is no raw-executable fallback. The new activation helper is hashed into the one-use recovery request; a changed helper cannot dispatch a restart.

No firewall permission, native URL policy, browser access verifier, or cross-session identity rule is changed. A Windows Security firewall dialog was observed while Chrome's URL detection failed. Whether correct package activation resolves that dialog or the URL failure requires separate live verification; neither is established by these tests.

Validation on Windows:

- The original launcher failed the new regression fixture because it attempted a raw desktop launch.
- 27 focused Rust integration tests passed across `recover_host_identity_contract`, `recover_bot_receipt_contract`, `writer_recovery_contract`, and `recover_controller_lifecycle_contract`.
- The package helper fixture compiles the native interop and reads the test process identity, while faking app activation and process mutation. It checks registration changes and wrong, absent, or exited replacement identities.
- Actual read-only registration and activation-manager preflight passed. The existing desktop process's package identity was confirmed absent.
- Changed-file formatting and `git diff --check` passed. No full-workspace CI result is claimed.

API references: [Windows application activation](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-iapplicationactivationmanager-activateapplication), [activation manager lifetime](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-iapplicationactivationmanager), [process package identity](https://learn.microsoft.com/en-us/windows/win32/api/appmodel/nf-appmodel-getpackagefullname).
