from __future__ import annotations

import hashlib
import os
from pathlib import Path

import pytest

import codex_remote_mcp_posix_file_guard as posix_guard
import codex_remote_mcp_posix_file_write as posix_write


pytestmark = pytest.mark.skipif(
    os.name == "nt",
    reason="POSIX descriptor-relative file semantics",
)


def _swap_parent(root: Path, outside: Path) -> Path:
    parent = root / "mutable"
    retained = root / "retained"
    parent.rename(retained)
    parent.symlink_to(outside, target_is_directory=True)
    return retained


def test_read_retains_verified_parent_during_symlink_swap(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    root = tmp_path / "project"
    outside = tmp_path / "outside"
    (root / "mutable").mkdir(parents=True)
    outside.mkdir()
    (root / "mutable" / "item.txt").write_bytes(b"inside")
    (outside / "item.txt").write_bytes(b"outside")
    guard = posix_guard.PosixProjectFileGuard(root)
    real_open = posix_guard.os.open
    swapped = False

    def swap_before_leaf_open(
        path: str | bytes | os.PathLike[str] | os.PathLike[bytes],
        flags: int,
        mode: int = 0o777,
        *,
        dir_fd: int | None = None,
    ) -> int:
        nonlocal swapped
        if os.fspath(path) == "item.txt" and dir_fd is not None and not swapped:
            _ = _swap_parent(root, outside)
            swapped = True
        return real_open(path, flags, mode, dir_fd=dir_fd)

    monkeypatch.setattr(posix_guard.os, "open", swap_before_leaf_open)

    assert guard.read_bytes(Path("mutable/item.txt")) == b"inside"
    assert (outside / "item.txt").read_bytes() == b"outside"
    assert swapped is True


def test_write_replaces_inside_retained_parent_during_symlink_swap(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    root = tmp_path / "project"
    outside = tmp_path / "outside"
    (root / "mutable").mkdir(parents=True)
    outside.mkdir()
    before = b"before"
    (root / "mutable" / "item.txt").write_bytes(before)
    (outside / "item.txt").write_bytes(b"outside")
    guard = posix_guard.PosixProjectFileGuard(root)
    real_replace = posix_write.os.replace
    retained: Path | None = None

    def swap_before_replace(
        source: str | bytes,
        destination: str | bytes,
        *,
        src_dir_fd: int | None = None,
        dst_dir_fd: int | None = None,
    ) -> None:
        nonlocal retained
        retained = _swap_parent(root, outside)
        real_replace(
            source,
            destination,
            src_dir_fd=src_dir_fd,
            dst_dir_fd=dst_dir_fd,
        )

    monkeypatch.setattr(posix_write.os, "replace", swap_before_replace)

    created = guard.write_bytes(
        Path("mutable/item.txt"),
        b"after",
        expected_sha256=hashlib.sha256(before).hexdigest(),
    )

    assert created is False
    assert retained is not None
    assert (retained / "item.txt").read_bytes() == b"after"
    assert (outside / "item.txt").read_bytes() == b"outside"


def test_delete_unlinks_inside_retained_parent_during_symlink_swap(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    root = tmp_path / "project"
    outside = tmp_path / "outside"
    (root / "mutable").mkdir(parents=True)
    outside.mkdir()
    content = b"inside"
    (root / "mutable" / "item.txt").write_bytes(content)
    (outside / "item.txt").write_bytes(b"outside")
    guard = posix_guard.PosixProjectFileGuard(root)
    real_unlink = posix_guard.os.unlink
    retained: Path | None = None

    def swap_before_unlink(
        path: str | bytes,
        *,
        dir_fd: int | None = None,
    ) -> None:
        nonlocal retained
        if os.fspath(path) == "item.txt" and retained is None:
            retained = _swap_parent(root, outside)
        real_unlink(path, dir_fd=dir_fd)

    monkeypatch.setattr(posix_guard.os, "unlink", swap_before_unlink)

    guard.delete_file(
        Path("mutable/item.txt"),
        expected_sha256=hashlib.sha256(content).hexdigest(),
    )

    assert retained is not None
    assert not (retained / "item.txt").exists()
    assert (outside / "item.txt").read_bytes() == b"outside"


def test_read_rejects_hard_linked_file(tmp_path: Path) -> None:
    root = tmp_path / "project"
    root.mkdir()
    outside = tmp_path / "outside.txt"
    outside.write_bytes(b"outside")
    os.link(outside, root / "linked.txt")
    guard = posix_guard.PosixProjectFileGuard(root)

    with pytest.raises(posix_guard.PosixFileGuardError, match="hard links"):
        _ = guard.read_bytes(Path("linked.txt"))
