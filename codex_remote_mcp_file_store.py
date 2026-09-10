from __future__ import annotations

import os
from pathlib import Path
from typing import final

from codex_remote_mcp_posix_file_guard import (
    PosixFileConflictError,
    PosixFileReadLimitError,
    PosixProjectFileGuard,
)
from codex_remote_mcp_windows_file_guard import (
    WindowsFileConflictError,
    WindowsFileReadLimitError,
    WindowsProjectFileGuard,
)


class ProjectFileStoreError(OSError):
    """Raised when the backend cannot prove project-path confinement."""


class ProjectFileStoreConflict(ProjectFileStoreError):
    """Raised when a retained file no longer meets its write condition."""


class ProjectFileStoreReadLimit(ProjectFileStoreError):
    """Raised before a backend read can exceed its byte limit."""


@final
class ProjectFileStore:
    """Use retained platform handles for confined project file access."""

    __slots__ = ("_posix", "_windows", "root")

    def __init__(self, root: Path) -> None:
        self.root: Path = root
        try:
            if os.name == "nt":
                self._windows: WindowsProjectFileGuard | None = (
                    WindowsProjectFileGuard(root)
                )
                self._posix: PosixProjectFileGuard | None = None
            else:
                self._windows = None
                self._posix = PosixProjectFileGuard(root)
        except OSError as exc:
            raise ProjectFileStoreError(str(exc)) from exc

    def ensure(self, relative: Path, *, require_file: bool) -> Path:
        if self._windows is not None:
            try:
                return self._windows.ensure(relative, require_file=require_file)
            except OSError as exc:
                raise ProjectFileStoreError(str(exc)) from exc
        assert self._posix is not None
        try:
            return self._posix.ensure(relative, require_file=require_file)
        except OSError as exc:
            raise ProjectFileStoreError(str(exc)) from exc

    def verify_root(self) -> None:
        guard = self._windows if self._windows is not None else self._posix
        assert guard is not None
        try:
            guard.verify_root()
        except OSError as exc:
            raise ProjectFileStoreError(str(exc)) from exc

    def read_bytes(
        self,
        relative: Path,
        *,
        max_bytes: int | None = None,
        allow_truncated: bool = False,
    ) -> bytes:
        if self._windows is not None:
            try:
                return self._windows.read_bytes(
                    relative,
                    max_bytes=max_bytes,
                    allow_truncated=allow_truncated,
                )
            except WindowsFileReadLimitError as exc:
                raise ProjectFileStoreReadLimit(str(exc)) from exc
            except OSError as exc:
                raise ProjectFileStoreError(str(exc)) from exc
        assert self._posix is not None
        try:
            return self._posix.read_bytes(
                relative,
                max_bytes=max_bytes,
                allow_truncated=allow_truncated,
            )
        except PosixFileReadLimitError as exc:
            raise ProjectFileStoreReadLimit(str(exc)) from exc
        except OSError as exc:
            raise ProjectFileStoreError(str(exc)) from exc

    def file_exists(self, relative: Path) -> bool:
        if self._windows is not None:
            try:
                return self._windows.file_exists(relative)
            except OSError as exc:
                raise ProjectFileStoreError(str(exc)) from exc
        assert self._posix is not None
        try:
            return self._posix.file_exists(relative)
        except OSError as exc:
            raise ProjectFileStoreError(str(exc)) from exc

    def file_size(self, relative: Path) -> int:
        if self._windows is not None:
            try:
                return self._windows.file_size(relative)
            except OSError as exc:
                raise ProjectFileStoreError(str(exc)) from exc
        assert self._posix is not None
        try:
            return self._posix.file_size(relative)
        except OSError as exc:
            raise ProjectFileStoreError(str(exc)) from exc

    def write_bytes(
        self,
        relative: Path,
        content: bytes,
        *,
        expected_sha256: str | None,
    ) -> bool:
        if self._windows is not None:
            try:
                return self._windows.write_bytes(
                    relative,
                    content,
                    expected_sha256=expected_sha256,
                )
            except WindowsFileConflictError as exc:
                raise ProjectFileStoreConflict(str(exc)) from exc
            except OSError as exc:
                raise ProjectFileStoreError(str(exc)) from exc
        assert self._posix is not None
        try:
            return self._posix.write_bytes(
                relative,
                content,
                expected_sha256=expected_sha256,
            )
        except PosixFileConflictError as exc:
            raise ProjectFileStoreConflict(str(exc)) from exc
        except OSError as exc:
            raise ProjectFileStoreError(str(exc)) from exc

    def delete_file(self, relative: Path, *, expected_sha256: str) -> None:
        if self._windows is not None:
            try:
                self._windows.delete_file(
                    relative,
                    expected_sha256=expected_sha256,
                )
                return
            except WindowsFileConflictError as exc:
                raise ProjectFileStoreConflict(str(exc)) from exc
            except OSError as exc:
                raise ProjectFileStoreError(str(exc)) from exc
        assert self._posix is not None
        try:
            self._posix.delete_file(
                relative,
                expected_sha256=expected_sha256,
            )
        except PosixFileConflictError as exc:
            raise ProjectFileStoreConflict(str(exc)) from exc
        except OSError as exc:
            raise ProjectFileStoreError(str(exc)) from exc
