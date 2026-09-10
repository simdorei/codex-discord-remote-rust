from __future__ import annotations

import os
import subprocess
import sys
from collections.abc import Callable
from urllib.parse import quote

try:
    import winreg
except ImportError:
    winreg = None

StartFile = Callable[[str], object]


def open_codex_thread_deep_link(
    thread_id: str,
    *,
    start_file: StartFile | None = None,
) -> None:
    normalized_thread_id = thread_id.strip()
    if not normalized_thread_id:
        raise ValueError("Missing Codex thread id for deep-link activation.")
    url = f"codex://threads/{quote(normalized_thread_id, safe='')}"
    if start_file is not None:
        _ = start_file(url)
        return
    if os.name == "nt":
        opener = getattr(os, "startfile", None)
        if opener is None:
            raise OSError("Windows does not expose URL protocol activation.")
        _ = opener(url)
        return
    if sys.platform == "darwin":
        _ = subprocess.run(["open", url], check=True)
        return
    raise OSError("Codex deep-link activation is only supported on Windows and macOS.")


def is_protocol_registered(protocol: str) -> bool:
    if not protocol or winreg is None:
        return False

    candidates = [
        (winreg.HKEY_CLASSES_ROOT, protocol),
        (winreg.HKEY_CURRENT_USER, rf"Software\Classes\{protocol}"),
    ]
    for hive, subkey in candidates:
        try:
            with winreg.OpenKey(hive, subkey):
                return True
        except FileNotFoundError:
            continue
        except OSError:
            continue
    return False
