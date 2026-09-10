from __future__ import annotations

import os
import secrets
from collections.abc import Callable


class PosixFileWriteConflictError(OSError):
    """Raised when an optimistic POSIX write condition no longer holds."""


VerifyRegular = Callable[[os.stat_result], os.stat_result]
HashFileDescriptor = Callable[[int], str]


def atomic_write_bytes(
    parent_fd: int,
    name: str,
    content: bytes,
    *,
    expected_sha256: str | None,
    verify_regular: VerifyRegular,
    sha256_fd: HashFileDescriptor,
) -> bool:
    created, previous = _check_write_condition(
        parent_fd,
        name,
        expected_sha256,
        verify_regular=verify_regular,
        sha256_fd=sha256_fd,
    )
    temporary = _write_temporary(parent_fd, name, content)
    try:
        if created:
            try:
                os.link(
                    temporary,
                    name,
                    src_dir_fd=parent_fd,
                    dst_dir_fd=parent_fd,
                    follow_symlinks=False,
                )
            except FileExistsError as exc:
                raise PosixFileWriteConflictError(
                    "file appeared before it could be created"
                ) from exc
            os.unlink(temporary, dir_fd=parent_fd)
        else:
            _verify_unchanged(
                parent_fd,
                name,
                previous,
                verify_regular=verify_regular,
            )
            os.replace(
                temporary,
                name,
                src_dir_fd=parent_fd,
                dst_dir_fd=parent_fd,
            )
        temporary = ""
        os.fsync(parent_fd)
        return created
    finally:
        if temporary:
            try:
                os.unlink(temporary, dir_fd=parent_fd)
            except FileNotFoundError:
                pass


def verify_unchanged(
    parent_fd: int,
    name: str,
    previous: os.stat_result,
    *,
    verify_regular: VerifyRegular,
) -> None:
    _verify_unchanged(
        parent_fd,
        name,
        previous,
        verify_regular=verify_regular,
    )


def _check_write_condition(
    parent_fd: int,
    name: str,
    expected_sha256: str | None,
    *,
    verify_regular: VerifyRegular,
    sha256_fd: HashFileDescriptor,
) -> tuple[bool, os.stat_result | None]:
    flags = os.O_RDONLY | int(getattr(os, "O_CLOEXEC", 0)) | int(
        getattr(os, "O_NOFOLLOW", 0)
    )
    try:
        file_fd = os.open(name, flags, dir_fd=parent_fd)
    except FileNotFoundError:
        if expected_sha256 is not None:
            raise PosixFileWriteConflictError(
                "new files require expected_sha256=null"
            ) from None
        return True, None
    try:
        information = verify_regular(os.fstat(file_fd))
        if expected_sha256 is None:
            raise PosixFileWriteConflictError(
                "existing files require expected_sha256"
            )
        if sha256_fd(file_fd) != expected_sha256:
            raise PosixFileWriteConflictError("file changed since it was read")
        return False, information
    finally:
        os.close(file_fd)


def _write_temporary(parent_fd: int, name: str, content: bytes) -> str:
    temporary = f".{name}.{secrets.token_hex(8)}.tmp"
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | int(
        getattr(os, "O_CLOEXEC", 0)
    )
    file_fd = os.open(temporary, flags, 0o600, dir_fd=parent_fd)
    with os.fdopen(file_fd, "wb") as stream:
        _ = stream.write(content)
        stream.flush()
        os.fsync(stream.fileno())
    return temporary


def _verify_unchanged(
    parent_fd: int,
    name: str,
    previous: os.stat_result | None,
    *,
    verify_regular: VerifyRegular,
) -> None:
    if previous is None:
        return
    current = verify_regular(
        os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    )
    if (
        _identity(current) != _identity(previous)
        or current.st_size != previous.st_size
        or current.st_mtime_ns != previous.st_mtime_ns
    ):
        raise PosixFileWriteConflictError("file changed since it was read")


def _identity(information: os.stat_result) -> tuple[int, int]:
    return information.st_dev, information.st_ino
