from __future__ import annotations

import hashlib
import secrets
from pathlib import Path

from codex_remote_mcp_windows_file_condition import (
    VerifyRegularFile,
    WindowsAtomicWriteConflictError,
    retain_expected_target,
    target_has_identity,
    validate_expected,
    verify_committed,
)
from codex_remote_mcp_windows_file_link import link_file_by_handle
from codex_remote_mcp_windows_file_native import (
    close_handle,
    rename_file_by_handle,
    set_delete,
)
from codex_remote_mcp_windows_file_staging import (
    delete_path_if_identity,
    retain_temporary,
    write_temporary,
)


def atomic_replace_file(
    target: Path,
    content: bytes,
    *,
    expected_sha256: str | None,
    verify_regular_file: VerifyRegularFile,
    verify_existing_file: VerifyRegularFile,
) -> bool:
    """Publish a complete sibling file without overwriting a concurrent writer."""
    created = validate_expected(target, expected_sha256, verify_existing_file)
    content_sha256 = hashlib.sha256(content).hexdigest()
    temporary, temporary_identity = write_temporary(
        target,
        content,
        verify_regular_file,
    )
    try:
        temporary_handle = retain_temporary(
            temporary,
            temporary_identity,
            content_sha256,
            verify_regular_file,
        )
    except OSError:
        delete_path_if_identity(
            temporary,
            temporary_identity,
            verify_regular_file,
        )
        raise
    try:
        if created:
            _publish_new_file(temporary_handle, target)
        else:
            _publish_existing_file(
                temporary_handle,
                target,
                expected_sha256,
                verify_existing_file,
            )
        verify_committed(target, temporary_identity, verify_regular_file)
        return created
    finally:
        try:
            _remove_temporary_link(temporary_handle, temporary)
        finally:
            close_handle(temporary_handle)


def _publish_new_file(temporary_handle: int, target: Path) -> None:
    try:
        link_file_by_handle(temporary_handle, target)
    except FileExistsError:
        raise WindowsAtomicWriteConflictError(
            "file appeared before it was created"
        ) from None


def _publish_existing_file(
    temporary_handle: int,
    target: Path,
    expected_sha256: str | None,
    verify_regular_file: VerifyRegularFile,
) -> None:
    retained, retained_identity = retain_expected_target(
        target,
        expected_sha256,
        verify_regular_file,
    )
    moved_aside = False
    try:
        try:
            _move_to_unique_backup(retained, target)
            moved_aside = True
        except OSError as exc:
            if not target_has_identity(
                target,
                retained_identity,
                verify_regular_file,
            ):
                raise WindowsAtomicWriteConflictError(
                    "file changed during the atomic update"
                ) from exc
            raise
        try:
            link_file_by_handle(temporary_handle, target)
        except FileExistsError:
            set_delete(retained)
            moved_aside = False
            raise WindowsAtomicWriteConflictError(
                "file changed during the atomic update"
            ) from None
        except OSError:
            try:
                _restore_expected_target(retained, target)
            finally:
                moved_aside = False
            raise
        set_delete(retained)
        moved_aside = False
    finally:
        try:
            if moved_aside:
                _restore_expected_target(retained, target)
        finally:
            close_handle(retained)


def _move_to_unique_backup(handle: int, target: Path) -> None:
    for _ in range(32):
        backup = target.with_name(f".{target.name}.{secrets.token_hex(16)}.bak")
        try:
            rename_file_by_handle(handle, backup, replace_existing=False)
        except FileExistsError:
            continue
        return
    raise OSError("could not reserve a unique backup file")


def _restore_expected_target(handle: int, target: Path) -> None:
    try:
        rename_file_by_handle(handle, target, replace_existing=False)
    except FileExistsError:
        set_delete(handle)
        raise WindowsAtomicWriteConflictError(
            "file changed during the atomic update"
        ) from None


def _remove_temporary_link(handle: int, temporary: Path) -> None:
    for _ in range(32):
        discard = temporary.with_name(
            f".{temporary.name}.{secrets.token_hex(16)}.discard"
        )
        try:
            rename_file_by_handle(handle, discard, replace_existing=False)
        except FileExistsError:
            continue
        set_delete(handle)
        return
    raise OSError("could not reserve a temporary cleanup path")
