from __future__ import annotations

import os
from pathlib import Path
import subprocess

import codex_desktop_bridge_session_index as session_index
import codex_desktop_bridge_sidecar as sidecar_transport
import codex_desktop_bridge_thread_store as thread_store


class NewThreadCwdDoesNotExistError(RuntimeError):
    def __init__(self, path: Path) -> None:
        self.path: Path = path
        super().__init__(f"New-thread cwd does not exist: {path}")


class NewThreadCwdNotDirectoryError(RuntimeError):
    def __init__(self, path: Path) -> None:
        self.path: Path = path
        super().__init__(f"New-thread cwd is not a directory: {path}")


def resolve_new_thread_cwd(cwd: str | None) -> str:
    target_source = str(cwd or "").strip()
    if not target_source:
        try:
            target_source = session_index.strip_windows_extended_prefix(thread_store.choose_thread(None, None).cwd)
        except (FileNotFoundError, OSError, RuntimeError):
            target_source = ""
    if not target_source:
        target_source = os.getcwd()

    target = Path(target_source).expanduser()
    if not target.is_absolute():
        target = target.resolve()
    if not target.exists():
        raise NewThreadCwdDoesNotExistError(target)
    if not target.is_dir():
        raise NewThreadCwdNotDirectoryError(target)
    return str(target)


def spawn_background_new_thread_runner(prompt: str, cwd: str) -> subprocess.Popen[bytes]:
    creationflags = (
        getattr(subprocess, "CREATE_NO_WINDOW", 0)
        | getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
        | getattr(subprocess, "DETACHED_PROCESS", 0)
    )
    return subprocess.Popen(
        [
            sidecar_transport.resolve_codex_app_server_executable(),
            "debug",
            "app-server",
            "send-message-v2",
            prompt,
        ],
        cwd=cwd,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        creationflags=creationflags,
    )
