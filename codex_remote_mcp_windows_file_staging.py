from __future__ import annotations

import os
import secrets
from pathlib import Path

from codex_remote_mcp_windows_file_condition import (
    FileIdentity,
    VerifyRegularFile,
    hash_verified,
)
from codex_remote_mcp_windows_file_native import (
    CREATE_NEW,
    DELETE_ACCESS,
    FILE_READ_ATTRIBUTES,
    GENERIC_READ,
    GENERIC_WRITE,
    OPEN_EXISTING,
    binary_stream,
    close_handle,
    file_identity,
    open_handle,
    set_delete,
)


def write_temporary(
    target: Path,
    content: bytes,
    verify_regular_file: VerifyRegularFile,
) -> tuple[Path, FileIdentity]:
    temporary, handle = _create_temporary(target)
    try:
        information = verify_regular_file(handle)
    except OSError:
        try:
            set_delete(handle)
        finally:
            close_handle(handle)
        raise
    identity = file_identity(information)
    try:
        with binary_stream(handle, writable=True) as stream:
            _ = stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())
    except OSError:
        delete_path_if_identity(temporary, identity, verify_regular_file)
        raise
    return temporary, identity


def retain_temporary(
    temporary: Path,
    expected_identity: FileIdentity,
    expected_sha256: str,
    verify_regular_file: VerifyRegularFile,
) -> int:
    handle = open_handle(
        temporary,
        GENERIC_READ | DELETE_ACCESS,
        OPEN_EXISTING,
        share_write=False,
        share_delete=False,
    )
    try:
        information = verify_regular_file(handle)
        if file_identity(information) != expected_identity:
            raise OSError("temporary file identity changed before publication")
        current_sha256 = hash_verified(
            temporary,
            verify_regular_file,
            exclusive=True,
        )
        if current_sha256 != expected_sha256:
            raise OSError("temporary file content changed before publication")
    except OSError:
        close_handle(handle)
        raise
    return handle


def delete_path_if_identity(
    path: Path,
    expected_identity: FileIdentity,
    verify_regular_file: VerifyRegularFile,
) -> None:
    try:
        handle = open_handle(
            path,
            DELETE_ACCESS | FILE_READ_ATTRIBUTES,
            OPEN_EXISTING,
            share_write=False,
            share_delete=False,
        )
    except FileNotFoundError:
        return
    try:
        information = verify_regular_file(handle)
        if file_identity(information) == expected_identity:
            set_delete(handle)
    finally:
        close_handle(handle)


def _create_temporary(target: Path) -> tuple[Path, int]:
    for _ in range(32):
        temporary = target.with_name(f".{target.name}.{secrets.token_hex(16)}.tmp")
        try:
            handle = open_handle(
                temporary,
                GENERIC_READ | GENERIC_WRITE | DELETE_ACCESS,
                CREATE_NEW,
            )
        except FileExistsError:
            continue
        return temporary, handle
    raise OSError("could not reserve a unique temporary file")
