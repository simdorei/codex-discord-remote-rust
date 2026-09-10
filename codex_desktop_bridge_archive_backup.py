from __future__ import annotations

import os
import shutil
import sqlite3
import time
import uuid
from pathlib import Path

import codex_desktop_bridge_session_index as session_index
import codex_desktop_bridge_state as bridge_state


def is_path_within_directory(path: Path, directory: Path) -> bool:
    candidate = Path(session_index.strip_windows_extended_prefix(str(path))).expanduser().resolve(strict=False)
    root = Path(session_index.strip_windows_extended_prefix(str(directory))).expanduser().resolve(strict=False)
    try:
        return os.path.commonpath([str(candidate), str(root)]) == str(root)
    except ValueError:
        return False


def sqlite_backup_to_path(source: Path, destination: Path) -> bool:
    if not source.exists():
        return False
    destination.parent.mkdir(parents=True, exist_ok=True)
    with sqlite3.connect(source) as src, sqlite3.connect(destination) as dst:
        src.backup(dst)
    return True


def copy_file_to_backup(source: Path, destination: Path) -> bool:
    if not source.exists():
        return False
    destination.parent.mkdir(parents=True, exist_ok=True)
    _ = shutil.copy2(source, destination)
    return True


def create_archive_delete_backup_dir(thread_id: str) -> Path:
    stamp = time.strftime("%Y%m%d-%H%M%S", time.localtime())
    backup_dir = bridge_state.MAINTENANCE_BACKUP_ROOT / f"delete-archive-{stamp}-{thread_id[:8]}"
    if backup_dir.exists():
        backup_dir = backup_dir.with_name(f"{backup_dir.name}-{uuid.uuid4().hex[:6]}")
    backup_dir.mkdir(parents=True, exist_ok=False)
    return backup_dir


def backup_archive_delete_inputs(backup_dir: Path) -> list[Path]:
    copied: list[Path] = []
    if sqlite_backup_to_path(bridge_state.STATE_DB_PATH, backup_dir / bridge_state.STATE_DB_PATH.name):
        copied.append(backup_dir / bridge_state.STATE_DB_PATH.name)
    if sqlite_backup_to_path(bridge_state.LOG_DB_PATH, backup_dir / bridge_state.LOG_DB_PATH.name):
        copied.append(backup_dir / bridge_state.LOG_DB_PATH.name)
    for source in (bridge_state.GLOBAL_STATE_PATH, bridge_state.BRIDGE_STATE_PATH, bridge_state.SESSION_INDEX_PATH):
        destination = backup_dir / source.name
        if copy_file_to_backup(source, destination):
            copied.append(destination)
    return copied
