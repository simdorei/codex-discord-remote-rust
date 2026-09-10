from __future__ import annotations

import hashlib
import re
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Final, override

from codex_remote_mcp_file_store import (
    ProjectFileStore,
    ProjectFileStoreConflict,
    ProjectFileStoreError,
    ProjectFileStoreReadLimit,
)
from codex_remote_mcp_file_listing import (
    ProjectGlobLimitError,
    ProjectGlobPatternError,
    iter_bounded_project_glob,
)
from codex_remote_mcp_redaction import redact
from simdorei_mcp_common.messages import (
    FileEntry,
    ListFilesOutput,
    ProjectInfoOutput,
    ReadFileOutput,
    WriteFileOutput,
)

MAX_FILE_BYTES: Final = 1_048_576
MAX_LIST_RESULTS: Final = 500
LIST_SKIP_DIRECTORIES: Final = frozenset(
    {
        ".codex-remote-mcp",
        ".git",
        ".next",
        ".venv",
        "__pycache__",
        "build",
        "dist",
        "node_modules",
        "vendor",
    }
)
SENSITIVE_PARTS: Final = frozenset(
    {
        ".aws",
        ".codex-remote-mcp",
        ".env",
        ".git",
        ".npmrc",
        ".ssh",
        "cookies.txt",
        "credentials",
        "id_ed25519",
        "id_rsa",
        "secrets",
        "gcloud",
    }
)
SENSITIVE_BASENAME: Final = (
    re.compile(r"^\.env(?:\..+)?$", re.IGNORECASE),
    re.compile(r"\.(?:pem|key|p12|pfx|keystore)$", re.IGNORECASE),
    re.compile(r"^id_rsa.*$", re.IGNORECASE),
    re.compile(r"(?:token|secret|credential)", re.IGNORECASE),
)


@dataclass(slots=True)
class ProjectFileError(Exception):
    path: str
    reason: str

    @override
    def __str__(self) -> str:
        return f"{self.path}: {self.reason}"


class UnsafeProjectPathError(ProjectFileError):
    """Raised when a path escapes the project or targets secret material."""


class ProjectFileSizeError(ProjectFileError):
    """Raised when a file exceeds the bridge transfer limit."""


class ProjectFileLimitError(ProjectFileError):
    """Raised when file enumeration exceeds its bounded work limit."""


class ProjectFileEncodingError(ProjectFileError):
    """Raised when a file is not UTF-8 text."""


class FileConflictError(ProjectFileError):
    """Raised when an optimistic write could overwrite unseen changes."""


@dataclass(frozen=True, slots=True)
class ProjectFileAccess:
    """Confined UTF-8 file access for one bound project root."""

    root: Path
    _store: ProjectFileStore = field(
        init=False,
        repr=False,
        compare=False,
    )

    def __post_init__(self) -> None:
        resolved = self.root.expanduser().resolve()
        if not resolved.is_dir():
            raise UnsafeProjectPathError(
                str(resolved), "project root is not a directory"
            )
        object.__setattr__(self, "root", resolved)
        try:
            store = ProjectFileStore(resolved)
        except ProjectFileStoreError as exc:
            raise UnsafeProjectPathError(
                str(resolved),
                f"project root cannot be retained safely: {exc}",
            ) from exc
        object.__setattr__(self, "_store", store)

    def project_info(self, thread_id: str) -> ProjectInfoOutput:
        return ProjectInfoOutput(root=str(self.root), thread_id=thread_id)

    def verify_root(self) -> None:
        try:
            self._store.verify_root()
        except ProjectFileStoreError as exc:
            raise UnsafeProjectPathError(str(self.root), str(exc)) from exc

    def resolve_path(self, value: str, *, require_file: bool = True) -> Path:
        """Resolve one non-sensitive path while keeping it inside the project."""
        return self._resolve(value, require_file=require_file)

    def list_files(
        self,
        pattern: str = "**/*",
        limit: int = 200,
        *,
        ensure_active: Callable[[], None] | None = None,
    ) -> ListFilesOutput:
        bounded_limit = min(max(limit, 1), MAX_LIST_RESULTS)
        entries: list[FileEntry] = []
        try:
            matched = iter_bounded_project_glob(
                self.root,
                pattern,
                excluded_directory_names=LIST_SKIP_DIRECTORIES,
                ensure_active=ensure_active,
            )
            for candidate in matched:
                if ensure_active is not None:
                    ensure_active()
                if len(entries) > bounded_limit:
                    break
                try:
                    _ = candidate.resolve().relative_to(self.root)
                except (OSError, RuntimeError, ValueError):
                    continue
                if self._is_sensitive(candidate):
                    continue
                relative = candidate.relative_to(self.root).as_posix()
                try:
                    size = self.file_size(relative)
                except ProjectFileError:
                    continue
                entries.append(
                    FileEntry(
                        path=relative,
                        size_bytes=size,
                    )
                )
        except ProjectGlobPatternError as exc:
            raise UnsafeProjectPathError(pattern, str(exc)) from exc
        except ProjectGlobLimitError as exc:
            raise ProjectFileLimitError(pattern, str(exc)) from exc
        return ListFilesOutput(
            files=tuple(entries[:bounded_limit]),
            truncated=len(entries) > bounded_limit,
        )

    def read_file(
        self, path: str, *, start_line: int, max_lines: int
    ) -> ReadFileOutput:
        target = self._resolve(path)
        raw = self.read_bytes(path, max_bytes=MAX_FILE_BYTES)
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError as exc:
            raise ProjectFileEncodingError(path, "file is not UTF-8 text") from exc
        lines = text.splitlines()
        bounded_start = max(start_line, 1)
        bounded_count = min(max(max_lines, 1), 500)
        start_index = bounded_start - 1
        selected = lines[start_index : start_index + bounded_count]
        end_line = start_index + len(selected)
        selected_content = "\n".join(selected)
        safe_content = redact(selected_content)
        return ReadFileOutput(
            path=target.relative_to(self.root).as_posix(),
            content=safe_content,
            sha256=hashlib.sha256(raw).hexdigest(),
            start_line=bounded_start,
            end_line=end_line,
            total_lines=len(lines),
            truncated=end_line < len(lines),
            redacted=safe_content != selected_content,
        )

    def write_file(
        self,
        path: str,
        content: str,
        *,
        expected_sha256: str | None,
    ) -> WriteFileOutput:
        target = self._resolve(path, require_file=False)
        raw = content.encode("utf-8")
        if len(raw) > MAX_FILE_BYTES:
            raise ProjectFileSizeError(path, f"content exceeds {MAX_FILE_BYTES} bytes")
        created = self.write_bytes(
            path,
            raw,
            expected_sha256=expected_sha256,
        )
        return WriteFileOutput(
            path=target.relative_to(self.root).as_posix(),
            sha256=hashlib.sha256(raw).hexdigest(),
            bytes_written=len(raw),
            created=created,
        )

    def _resolve(self, value: str, *, require_file: bool = True) -> Path:
        relative = self._relative(value)
        try:
            return self._store.ensure(relative, require_file=require_file)
        except ProjectFileStoreError as exc:
            raise UnsafeProjectPathError(value, str(exc)) from exc

    def read_bytes(
        self,
        value: str,
        *,
        max_bytes: int | None = None,
        allow_truncated: bool = False,
    ) -> bytes:
        if allow_truncated and max_bytes is None:
            raise ValueError("allow_truncated requires max_bytes")
        relative = self._relative(value)
        try:
            return self._store.read_bytes(
                relative,
                max_bytes=max_bytes,
                allow_truncated=allow_truncated,
            )
        except ProjectFileStoreReadLimit as exc:
            raise ProjectFileSizeError(
                value,
                f"file exceeds {max_bytes} bytes",
            ) from exc
        except ProjectFileStoreError as exc:
            raise UnsafeProjectPathError(value, str(exc)) from exc

    def file_exists(self, value: str) -> bool:
        relative = self._relative(value)
        try:
            return self._store.file_exists(relative)
        except ProjectFileStoreError as exc:
            raise UnsafeProjectPathError(value, str(exc)) from exc

    def file_size(self, value: str) -> int:
        relative = self._relative(value)
        try:
            return self._store.file_size(relative)
        except ProjectFileStoreError as exc:
            raise UnsafeProjectPathError(value, str(exc)) from exc

    def write_bytes(
        self,
        value: str,
        content: bytes,
        *,
        expected_sha256: str | None,
    ) -> bool:
        relative = self._relative(value)
        try:
            return self._store.write_bytes(
                relative,
                content,
                expected_sha256=expected_sha256,
            )
        except ProjectFileStoreConflict as exc:
            raise FileConflictError(value, str(exc)) from exc
        except ProjectFileStoreError as exc:
            raise UnsafeProjectPathError(value, str(exc)) from exc

    def delete_file(self, value: str, *, expected_sha256: str) -> None:
        relative = self._relative(value)
        try:
            self._store.delete_file(relative, expected_sha256=expected_sha256)
        except ProjectFileStoreConflict as exc:
            raise FileConflictError(value, str(exc)) from exc
        except ProjectFileStoreError as exc:
            raise UnsafeProjectPathError(value, str(exc)) from exc

    def _relative(self, value: str) -> Path:
        relative = Path(value)
        if relative.is_absolute() or relative.drive or ".." in relative.parts:
            raise UnsafeProjectPathError(
                value, "relative paths without '..' are required"
            )
        target = self.root / relative
        if self._is_sensitive(target):
            raise UnsafeProjectPathError(value, "sensitive paths are not exposed")
        return relative

    def _is_sensitive(self, path: Path) -> bool:
        relative = path.relative_to(self.root)
        parts = tuple(part.casefold() for part in relative.parts)
        basename = relative.name
        return any(part in SENSITIVE_PARTS for part in parts) or any(
            pattern.search(basename) is not None for pattern in SENSITIVE_BASENAME
        )
