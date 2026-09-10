from __future__ import annotations

import os
import stat
from collections.abc import Generator
from contextlib import contextmanager
from pathlib import Path
from typing import final

from codex_remote_mcp_file_hash import sha256_stream
import codex_remote_mcp_posix_file_write as posix_file_write


class PosixFileGuardError(OSError):
    """Raised when descriptor-relative POSIX confinement cannot be proven."""


PosixFileConflictError = posix_file_write.PosixFileWriteConflictError


class PosixFileReadLimitError(PosixFileGuardError):
    """Raised before a retained file can exceed its read limit."""


_DIRECTORY_FLAGS = (
    os.O_RDONLY
    | int(getattr(os, "O_CLOEXEC", 0))
    | int(getattr(os, "O_DIRECTORY", 0))
    | int(getattr(os, "O_NOFOLLOW", 0))
)
_FILE_FLAGS = os.O_RDONLY | int(getattr(os, "O_CLOEXEC", 0)) | int(
    getattr(os, "O_NOFOLLOW", 0)
)


@final
class PosixProjectFileGuard:
    """Keep a root descriptor and perform file I/O relative to retained parents."""

    __slots__ = ("_closed", "_root_fd", "_root_identity", "root")

    def __init__(self, root: Path) -> None:
        if os.name == "nt" or not getattr(os, "O_NOFOLLOW", 0):
            raise OSError("POSIX project file guards require O_NOFOLLOW support")
        self.root = root
        self._closed = False
        self._root_fd = os.open(root, _DIRECTORY_FLAGS)
        information = os.fstat(self._root_fd)
        if not stat.S_ISDIR(information.st_mode):
            self.close()
            raise PosixFileGuardError("project root is not a directory")
        self._root_identity = _identity(information)

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        os.close(self._root_fd)

    def __del__(self) -> None:
        self.close()

    def verify_root(self) -> None:
        retained = os.fstat(self._root_fd)
        current = os.stat(self.root, follow_symlinks=False)
        if (
            not stat.S_ISDIR(current.st_mode)
            or _identity(retained) != self._root_identity
            or _identity(current) != self._root_identity
        ):
            raise PosixFileGuardError(
                "the bound project root moved or changed identity"
            )

    def ensure(self, relative: Path, *, require_file: bool) -> Path:
        self.verify_root()
        parts = _parts(relative)
        if not parts:
            if require_file:
                raise PosixFileGuardError("path is not a file")
            return self.root
        current_fd = os.dup(self._root_fd)
        try:
            for index, part in enumerate(parts):
                try:
                    next_fd = os.open(part, _FILE_FLAGS, dir_fd=current_fd)
                except FileNotFoundError:
                    if require_file:
                        raise PosixFileGuardError("path is not a file") from None
                    return self.root / relative
                os.close(current_fd)
                current_fd = next_fd
                information = os.fstat(current_fd)
                if index < len(parts) - 1:
                    if not stat.S_ISDIR(information.st_mode):
                        raise PosixFileGuardError("path parent is not a directory")
                elif require_file:
                    _verify_regular(information)
                elif stat.S_ISREG(information.st_mode):
                    _verify_regular(information)
            return self.root / relative
        finally:
            os.close(current_fd)

    def read_bytes(
        self,
        relative: Path,
        *,
        max_bytes: int | None = None,
        allow_truncated: bool = False,
    ) -> bytes:
        with self._open_regular(relative) as (file_fd, information):
            if (
                max_bytes is not None
                and information.st_size > max_bytes
                and not allow_truncated
            ):
                raise PosixFileReadLimitError("file exceeds read limit")
            with os.fdopen(file_fd, "rb", closefd=False) as stream:
                if max_bytes is None:
                    return stream.read()
                content = stream.read(max_bytes if allow_truncated else max_bytes + 1)
            if len(content) > max_bytes:
                raise PosixFileReadLimitError("file exceeds read limit")
            return content

    def file_exists(self, relative: Path) -> bool:
        try:
            with self._open_regular(relative):
                return True
        except (FileNotFoundError, NotADirectoryError):
            return False
        except PosixFileGuardError as exc:
            if str(exc) == "path is not a file":
                return False
            raise

    def file_size(self, relative: Path) -> int:
        with self._open_regular(relative) as (_, information):
            return information.st_size

    def write_bytes(
        self,
        relative: Path,
        content: bytes,
        *,
        expected_sha256: str | None,
    ) -> bool:
        with self._locked_parent(relative, create=True) as (parent_fd, name):
            return posix_file_write.atomic_write_bytes(
                parent_fd,
                name,
                content,
                expected_sha256=expected_sha256,
                verify_regular=_verify_regular,
                sha256_fd=_sha256_fd,
            )

    def delete_file(self, relative: Path, *, expected_sha256: str) -> None:
        with self._locked_parent(relative, create=False) as (parent_fd, name):
            file_fd = os.open(name, _FILE_FLAGS, dir_fd=parent_fd)
            try:
                previous = _verify_regular(os.fstat(file_fd))
                if _sha256_fd(file_fd) != expected_sha256:
                    raise PosixFileConflictError("file changed since it was read")
            finally:
                os.close(file_fd)
            posix_file_write.verify_unchanged(
                parent_fd,
                name,
                previous,
                verify_regular=_verify_regular,
            )
            os.unlink(name, dir_fd=parent_fd)
            os.fsync(parent_fd)

    @contextmanager
    def _open_regular(
        self,
        relative: Path,
    ) -> Generator[tuple[int, os.stat_result]]:
        with self._locked_parent(relative, create=False) as (parent_fd, name):
            file_fd = os.open(name, _FILE_FLAGS, dir_fd=parent_fd)
            try:
                yield file_fd, _verify_regular(os.fstat(file_fd))
            finally:
                os.close(file_fd)

    @contextmanager
    def _locked_parent(
        self,
        relative: Path,
        *,
        create: bool,
    ) -> Generator[tuple[int, str]]:
        self.verify_root()
        parts = _parts(relative)
        if not parts:
            raise PosixFileGuardError("path is not a file")
        current_fd = os.dup(self._root_fd)
        try:
            for part in parts[:-1]:
                if create:
                    try:
                        os.mkdir(part, 0o755, dir_fd=current_fd)
                    except FileExistsError:
                        pass
                next_fd = os.open(part, _DIRECTORY_FLAGS, dir_fd=current_fd)
                os.close(current_fd)
                current_fd = next_fd
            yield current_fd, parts[-1]
        finally:
            os.close(current_fd)

def _parts(relative: Path) -> tuple[str, ...]:
    if relative.is_absolute() or relative.drive:
        raise PosixFileGuardError("relative paths are required")
    parts = tuple(relative.parts)
    if any(part in {"", ".", ".."} for part in parts):
        raise PosixFileGuardError("unsafe relative path component")
    return parts


def _verify_regular(information: os.stat_result) -> os.stat_result:
    if not stat.S_ISREG(information.st_mode):
        raise PosixFileGuardError("path is not a file")
    if information.st_nlink != 1:
        raise PosixFileGuardError("hard links are not allowed")
    return information


def _identity(information: os.stat_result) -> tuple[int, int]:
    return information.st_dev, information.st_ino


def _sha256_fd(file_fd: int) -> str:
    os.lseek(file_fd, 0, os.SEEK_SET)
    with os.fdopen(file_fd, "rb", closefd=False) as stream:
        return sha256_stream(stream)
