# pyright: reportAny=false
from __future__ import annotations

import os
from collections.abc import Generator
from contextlib import contextmanager
from pathlib import Path
from typing import final

from codex_remote_mcp_file_hash import sha256_stream
from codex_remote_mcp_windows_file_condition import WindowsAtomicWriteConflictError
from codex_remote_mcp_windows_file_native import (
    DELETE_ACCESS,
    FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_READ_ATTRIBUTES,
    GENERIC_READ,
    OPEN_EXISTING,
    ByHandleFileInformation,
    binary_stream,
    close_handle,
    confined_information,
    file_identity,
    file_information,
    final_path,
    open_handle,
    os_handle,
    set_delete,
)
from codex_remote_mcp_windows_file_write import atomic_replace_file


class WindowsFileGuardError(OSError):
    """Raised when handle-based Windows path confinement cannot be proven."""


class WindowsFileConflictError(WindowsFileGuardError):
    """Raised when a retained file identity no longer meets a write condition."""


class WindowsFileReadLimitError(WindowsFileGuardError):
    """Raised before a retained file can exceed a bounded read."""


@final
class WindowsProjectFileGuard:
    """Hold the project root and use verified handles for every file mutation."""

    __slots__ = (
        "_closed",
        "_root_final",
        "_root_handle",
        "_root_identity",
        "root",
    )

    def __init__(self, root: Path) -> None:
        if os.name != "nt":
            raise OSError("Windows project file guards require Windows.")
        self.root: Path = root
        self._closed: bool = False
        self._root_handle: int = open_handle(
            root, FILE_READ_ATTRIBUTES, OPEN_EXISTING, directory=True
        )
        information = file_information(self._root_handle)
        if not information.file_attributes & FILE_ATTRIBUTE_DIRECTORY:
            self.close()
            raise WindowsFileGuardError("project root is not a directory")
        if information.file_attributes & FILE_ATTRIBUTE_REPARSE_POINT:
            self.close()
            raise WindowsFileGuardError("project root cannot be a reparse point")
        self._root_final: str = final_path(self._root_handle)
        self._root_identity: tuple[int, int, int] = file_identity(information)

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        close_handle(self._root_handle)

    def __del__(self) -> None:
        self.close()

    def ensure(self, relative: Path, *, require_file: bool) -> Path:
        self.verify_root()
        if relative == Path("."):
            if require_file:
                raise WindowsFileGuardError("path is not a file")
            return self.root
        try:
            with self._locked_parent(relative, create=False) as target:
                try:
                    handle = open_handle(
                        target,
                        FILE_READ_ATTRIBUTES,
                        OPEN_EXISTING,
                        directory=True,
                    )
                except FileNotFoundError:
                    if require_file:
                        raise WindowsFileGuardError("path is not a file") from None
                    return target
                try:
                    information = self._verify(handle)
                    if information.file_attributes & FILE_ATTRIBUTE_DIRECTORY:
                        if require_file:
                            raise WindowsFileGuardError("path is not a file")
                    else:
                        _ = self._verify_single_link_file(handle)
                finally:
                    close_handle(handle)
                return target
        except FileNotFoundError:
            if require_file:
                raise WindowsFileGuardError("path is not a file") from None
            return self.root / relative

    def verify_root(self) -> None:
        information = file_information(self._root_handle)
        if file_identity(information) != self._root_identity or os.path.normcase(
            final_path(self._root_handle)
        ) != os.path.normcase(self._root_final):
            raise WindowsFileGuardError(
                "the bound project root moved or changed identity"
            )

    def read_bytes(
        self,
        relative: Path,
        *,
        max_bytes: int | None = None,
        allow_truncated: bool = False,
    ) -> bytes:
        with self._locked_parent(relative, create=False) as target:
            handle = open_handle(target, GENERIC_READ, OPEN_EXISTING)
            try:
                information = self._verify_single_link_file(handle)
                size = (
                    (information.file_size_high << 32)
                    | information.file_size_low
                )
                if (
                    max_bytes is not None
                    and size > max_bytes
                    and not allow_truncated
                ):
                    raise WindowsFileReadLimitError("file exceeds read limit")
            except OSError:
                close_handle(handle)
                raise
            with binary_stream(handle, writable=False) as stream:
                if max_bytes is None:
                    return stream.read()
                content = stream.read(
                    max_bytes if allow_truncated else max_bytes + 1
                )
                if len(content) > max_bytes:
                    raise WindowsFileReadLimitError("file exceeds read limit")
                return content

    def file_size(self, relative: Path) -> int:
        with self._locked_parent(relative, create=False) as target:
            handle = open_handle(target, FILE_READ_ATTRIBUTES, OPEN_EXISTING)
            try:
                information = self._verify_single_link_file(handle)
                return (information.file_size_high << 32) | information.file_size_low
            finally:
                close_handle(handle)

    def file_exists(self, relative: Path) -> bool:
        try:
            _ = self.ensure(relative, require_file=True)
        except FileNotFoundError:
            return False
        except WindowsFileGuardError as exc:
            if str(exc) == "path is not a file":
                return False
            raise
        return True

    def write_bytes(
        self,
        relative: Path,
        content: bytes,
        *,
        expected_sha256: str | None,
    ) -> bool:
        with self._locked_parent(relative, create=True) as target:
            try:
                return atomic_replace_file(
                    target,
                    content,
                    expected_sha256=expected_sha256,
                    verify_regular_file=self._verify_regular_file,
                    verify_existing_file=self._verify_single_link_file,
                )
            except WindowsAtomicWriteConflictError as exc:
                raise WindowsFileConflictError(str(exc)) from exc

    def delete_file(self, relative: Path, *, expected_sha256: str) -> None:
        with self._locked_parent(relative, create=False) as target:
            handle = open_handle(target, GENERIC_READ | DELETE_ACCESS, OPEN_EXISTING)
            try:
                _ = self._verify_single_link_file(handle)
            except OSError:
                close_handle(handle)
                raise
            with binary_stream(handle, writable=False) as stream:
                if sha256_stream(stream) != expected_sha256:
                    raise WindowsFileConflictError("file changed since it was read")
                set_delete(os_handle(stream))

    @contextmanager
    def _locked_parent(self, relative: Path, *, create: bool) -> Generator[Path]:
        self.verify_root()
        handles: list[int] = []
        current = self.root
        try:
            for part in relative.parent.parts:
                current /= part
                if create:
                    try:
                        os.mkdir(current)
                    except FileExistsError:
                        if not current.is_dir():
                            raise WindowsFileGuardError(
                                "path parent is not a directory"
                            ) from None
                handle = open_handle(
                    current,
                    FILE_READ_ATTRIBUTES,
                    OPEN_EXISTING,
                    directory=True,
                )
                try:
                    information = self._verify(handle)
                except OSError:
                    close_handle(handle)
                    raise
                handles.append(handle)
                if not information.file_attributes & FILE_ATTRIBUTE_DIRECTORY:
                    raise WindowsFileGuardError("path parent is not a directory")
            yield current / relative.name
        finally:
            for handle in reversed(handles):
                close_handle(handle)

    def _verify_regular_file(self, handle: int) -> ByHandleFileInformation:
        information = self._verify(handle)
        if information.file_attributes & FILE_ATTRIBUTE_DIRECTORY:
            raise WindowsFileGuardError("path is not a file")
        return information

    def _verify_single_link_file(self, handle: int) -> ByHandleFileInformation:
        information = self._verify_regular_file(handle)
        if information.number_of_links != 1:
            raise WindowsFileGuardError("hard links are not allowed")
        return information

    def _verify(self, handle: int) -> ByHandleFileInformation:
        return confined_information(handle, self._root_final)
