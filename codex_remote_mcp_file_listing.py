from __future__ import annotations

import fnmatch
import os
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from pathlib import Path
from typing import Final, cast

from codex_remote_mcp_windows_file_native import (
    FILE_ATTRIBUTE_DIRECTORY,
    FILE_READ_ATTRIBUTES,
    OPEN_EXISTING,
    close_handle,
    confined_information,
    open_handle,
)

MAX_LIST_SCAN_CANDIDATES: Final = 10_000
NO_EXCLUDED_DIRECTORIES: Final[frozenset[str]] = frozenset()


class ProjectGlobPatternError(ValueError):
    """Raised before an unsafe project glob reaches the filesystem."""


class ProjectGlobLimitError(ValueError):
    """Raised before a broad project glob can consume unbounded memory."""


def bounded_project_glob(
    root: Path,
    pattern: str,
    *,
    excluded_directory_names: frozenset[str] = NO_EXCLUDED_DIRECTORIES,
    ensure_active: Callable[[], None] | None = None,
) -> tuple[Path, ...]:
    return tuple(
        iter_bounded_project_glob(
            root,
            pattern,
            excluded_directory_names=excluded_directory_names,
            ensure_active=ensure_active,
        )
    )


def iter_bounded_project_glob(
    root: Path,
    pattern: str,
    *,
    excluded_directory_names: frozenset[str] = NO_EXCLUDED_DIRECTORIES,
    ensure_active: Callable[[], None] | None = None,
) -> Iterator[Path]:
    _validate_pattern(pattern)
    budget = _ScanBudget(ensure_active=ensure_active)
    excluded = frozenset(name.casefold() for name in excluded_directory_names)
    entry_cache: dict[Path, tuple[Path, ...]] = {}
    seen: set[Path] = set()
    for candidate in _iter_project_glob(
        root,
        tuple(pattern.split("/")),
        index=0,
        root=root,
        budget=budget,
        entry_cache=entry_cache,
        excluded_directory_names=excluded,
    ):
        if candidate not in seen:
            seen.add(candidate)
            yield candidate


@dataclass(slots=True)
class _ScanBudget:
    observed: int = 0
    ensure_active: Callable[[], None] | None = None

    def check(self) -> None:
        if self.ensure_active is not None:
            self.ensure_active()

    def consume(self) -> None:
        self.check()
        self.observed += 1
        if self.observed > MAX_LIST_SCAN_CANDIDATES:
            raise ProjectGlobLimitError(
                f"glob scanned too many candidates; limit is {MAX_LIST_SCAN_CANDIDATES}"
            )


def _iter_project_glob(
    current: Path,
    parts: tuple[str, ...],
    *,
    index: int,
    root: Path,
    budget: _ScanBudget,
    entry_cache: dict[Path, tuple[Path, ...]],
    excluded_directory_names: frozenset[str],
) -> Iterator[Path]:
    budget.check()
    if index >= len(parts):
        yield current
        return

    component = parts[index]
    if component == "**":
        yield from _iter_project_glob(
            current,
            parts,
            index=index + 1,
            root=root,
            budget=budget,
            entry_cache=entry_cache,
            excluded_directory_names=excluded_directory_names,
        )
        for child in _bounded_directory_entries(
            current,
            root,
            budget,
            entry_cache,
        ):
            if _can_descend(child, root, excluded_directory_names):
                yield from _iter_project_glob(
                    child,
                    parts,
                    index=index,
                    root=root,
                    budget=budget,
                    entry_cache=entry_cache,
                    excluded_directory_names=excluded_directory_names,
                )
        return

    is_terminal = index == len(parts) - 1
    for child in _bounded_directory_entries(current, root, budget, entry_cache):
        if not fnmatch.fnmatch(child.name, component):
            continue
        if is_terminal:
            yield child
        elif _can_descend(child, root, excluded_directory_names):
            yield from _iter_project_glob(
                child,
                parts,
                index=index + 1,
                root=root,
                budget=budget,
                entry_cache=entry_cache,
                excluded_directory_names=excluded_directory_names,
            )


def _bounded_directory_entries(
    path: Path,
    root: Path,
    budget: _ScanBudget,
    entry_cache: dict[Path, tuple[Path, ...]],
) -> tuple[Path, ...]:
    cached = entry_cache.get(path)
    if cached is not None:
        return cached
    entries: list[Path] = []
    for child in _iter_directory_entries(path, root):
        budget.consume()
        entries.append(child)
    result = tuple(entries)
    result = tuple(
        sorted(result, key=lambda candidate: (candidate.name.casefold(), candidate.name))
    )
    entry_cache[path] = result
    return result


def _iter_directory_entries(path: Path, root: Path) -> Iterator[Path]:
    if os.name == "nt":
        yield from _iter_locked_windows_directory(path, root)
        return
    try:
        yield from path.iterdir()
    except OSError:
        return


def _iter_locked_windows_directory(path: Path, root: Path) -> Iterator[Path]:
    try:
        handle = open_handle(
            path,
            FILE_READ_ATTRIBUTES,
            OPEN_EXISTING,
            directory=True,
        )
    except OSError:
        return
    try:
        information = confined_information(handle, str(root))
        if not cast(int, information.file_attributes) & FILE_ATTRIBUTE_DIRECTORY:
            return
        yield from path.iterdir()
    except OSError:
        return
    finally:
        close_handle(handle)


def _can_descend(
    path: Path,
    root: Path,
    excluded_directory_names: frozenset[str],
) -> bool:
    try:
        if (
            path.name.casefold() in excluded_directory_names
            or path.is_symlink()
            or os.path.isjunction(path)
        ):
            return False
        _ = path.resolve().relative_to(root)
        return path.is_dir()
    except (OSError, RuntimeError, ValueError):
        return False


def _validate_pattern(pattern: str) -> None:
    if "\x00" in pattern:
        raise ProjectGlobPatternError("glob pattern contains a null byte")
    if "\\" in pattern:
        raise ProjectGlobPatternError("relative glob patterns must use forward slashes")

    relative = Path(pattern)
    parts = pattern.split("/")
    if parts.count("**") > 1:
        raise ProjectGlobPatternError("at most one '**' segment is allowed")
    if (
        relative.is_absolute()
        or bool(relative.drive)
        or pattern.startswith("/")
        or ".." in parts
    ):
        raise ProjectGlobPatternError(
            "relative glob patterns without '..' segments are required"
        )
