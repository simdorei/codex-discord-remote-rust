from __future__ import annotations

from collections.abc import Callable
from pathlib import Path
from typing import cast

from codex_remote_mcp_file_hash import sha256_stream
from codex_remote_mcp_windows_file_native import (
    DELETE_ACCESS,
    FILE_READ_ATTRIBUTES,
    GENERIC_READ,
    OPEN_EXISTING,
    ByHandleFileInformation,
    binary_stream,
    close_handle,
    file_identity,
    open_handle,
)

VerifyRegularFile = Callable[[int], ByHandleFileInformation]
FileIdentity = tuple[int, int, int]


class WindowsAtomicWriteConflictError(OSError):
    """Raised when the destination no longer meets its write condition."""


def validate_expected(
    target: Path,
    expected_sha256: str | None,
    verify_regular_file: VerifyRegularFile,
) -> bool:
    try:
        current_sha256 = hash_verified(target, verify_regular_file)
    except FileNotFoundError:
        if expected_sha256 is not None:
            raise WindowsAtomicWriteConflictError(
                "file changed since it was read"
            ) from None
        return True
    if expected_sha256 is None:
        raise WindowsAtomicWriteConflictError("existing files require expected_sha256")
    if current_sha256 != expected_sha256:
        raise WindowsAtomicWriteConflictError("file changed since it was read")
    return False


def retain_expected_target(
    target: Path,
    expected_sha256: str | None,
    verify_regular_file: VerifyRegularFile,
) -> tuple[int, FileIdentity]:
    if expected_sha256 is None:
        raise WindowsAtomicWriteConflictError("existing files require expected_sha256")
    try:
        retained = open_handle(
            target,
            GENERIC_READ | DELETE_ACCESS,
            OPEN_EXISTING,
            share_write=False,
            share_delete=True,
        )
    except FileNotFoundError:
        raise WindowsAtomicWriteConflictError(
            "file changed since it was read"
        ) from None
    except OSError as exc:
        raise WindowsAtomicWriteConflictError(
            "file could not be locked for an atomic update"
        ) from exc
    validation_completed = False
    try:
        information = verify_regular_file(retained)
        current_sha256 = hash_verified(
            target,
            verify_regular_file,
            exclusive=True,
        )
        if current_sha256 != expected_sha256:
            raise WindowsAtomicWriteConflictError("file changed since it was read")
        validation_completed = True
        return retained, file_identity(information)
    finally:
        if not validation_completed:
            close_handle(retained)


def target_has_identity(
    target: Path,
    expected_identity: FileIdentity,
    verify_regular_file: VerifyRegularFile,
) -> bool:
    try:
        handle = open_handle(target, FILE_READ_ATTRIBUTES, OPEN_EXISTING)
    except FileNotFoundError:
        return False
    try:
        return file_identity(verify_regular_file(handle)) == expected_identity
    finally:
        close_handle(handle)


def verify_committed(
    target: Path,
    expected_identity: FileIdentity,
    verify_regular_file: VerifyRegularFile,
) -> None:
    handle = open_handle(target, FILE_READ_ATTRIBUTES, OPEN_EXISTING)
    try:
        information = verify_regular_file(handle)
        if file_identity(information) != expected_identity:
            raise WindowsAtomicWriteConflictError(
                "file changed during the atomic update"
            )
        if cast(int, information.number_of_links) != 2:
            raise WindowsAtomicWriteConflictError(
                "published file has an unexpected hard-link count"
            )
    finally:
        close_handle(handle)


def hash_verified(
    target: Path,
    verify_regular_file: VerifyRegularFile,
    *,
    exclusive: bool = False,
) -> str:
    handle = open_handle(
        target,
        GENERIC_READ,
        OPEN_EXISTING,
        share_write=not exclusive,
        share_delete=exclusive,
    )
    try:
        _ = verify_regular_file(handle)
    except OSError:
        close_handle(handle)
        raise
    with binary_stream(handle, writable=False) as stream:
        return sha256_stream(stream)
