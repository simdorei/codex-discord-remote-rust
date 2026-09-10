from __future__ import annotations

import re
from datetime import datetime
from pathlib import Path
from typing import TypeVar

from codex_discord_project_types import BridgeProjectModule, ProjectThread

ThreadT = TypeVar("ThreadT", bound=ProjectThread)

_CODEX_PROJECTLESS_CHAT_CWD_PREFIXES = (
    "new-chat",
    "chatgpt-conversation-",
)


def _thread_cwd(thread: ProjectThread, *, bridge_module: BridgeProjectModule) -> str:
    cwd = str(thread.cwd).strip() if thread.cwd is not None else ""
    return bridge_module.strip_windows_extended_prefix(cwd)


def is_codex_projectless_chat_cwd(cwd: str, *, bridge_module: BridgeProjectModule) -> bool:
    normalized = bridge_module.normalize_workspace_path(cwd)
    parts = re.split(r"[\\/]+", normalized)
    if len(parts) < 4:
        return False
    leaf_name = parts[-1].lower()
    return (
        len(parts) >= 5
        and any(leaf_name.startswith(prefix) for prefix in _CODEX_PROJECTLESS_CHAT_CWD_PREFIXES)
        and re.match(r"^\d{4}-\d{2}-\d{2}$", parts[-2] or "") is not None
        and parts[-3].lower() == "codex"
        and parts[-4].lower() == "documents"
    )


def get_project_key(thread: ProjectThread, *, bridge_module: BridgeProjectModule, projectless_chat_key: str) -> str:
    cwd = _thread_cwd(thread, bridge_module=bridge_module)
    if cwd:
        if is_codex_projectless_chat_cwd(cwd, bridge_module=bridge_module):
            return projectless_chat_key
        return normalize_project_key(
            cwd,
            bridge_module=bridge_module,
            projectless_chat_key=projectless_chat_key,
        )
    return f"projectless:{bridge_module.get_thread_workspace_name(thread)}"


def normalize_project_key(
    project_key: str | None,
    *,
    bridge_module: BridgeProjectModule,
    projectless_chat_key: str,
) -> str:
    value = str(project_key or "").strip()
    if not value:
        return ""
    if value == projectless_chat_key or value.startswith("projectless:"):
        return value
    return bridge_module.normalize_workspace_path(value)


def get_project_name(thread: ProjectThread, *, bridge_module: BridgeProjectModule) -> str:
    cwd = _thread_cwd(thread, bridge_module=bridge_module)
    if cwd and is_codex_projectless_chat_cwd(cwd, bridge_module=bridge_module):
        return "\ucc44\ud305"
    name = bridge_module.get_thread_workspace_name(thread)
    return name if name and name != "-" else "projectless"


def get_saved_workspace_project_keys(*, bridge_module: BridgeProjectModule) -> set[str]:
    data = bridge_module.load_json(bridge_module.GLOBAL_STATE_PATH)
    saved: set[str] = set()
    for key in ("project-order", "electron-saved-workspace-roots"):
        roots_value = data.get(key)
        if not isinstance(roots_value, list):
            continue
        for root in roots_value:
            value = str(root).strip() if root is not None else ""
            if value:
                saved.add(bridge_module.normalize_workspace_path(value))
    return saved


def is_thread_mirrorable(
    thread: ProjectThread,
    saved_project_keys: set[str] | None = None,
    *,
    bridge_module: BridgeProjectModule,
    projectless_chat_key: str,
) -> bool:
    project_key = get_project_key(
        thread,
        bridge_module=bridge_module,
        projectless_chat_key=projectless_chat_key,
    )
    if project_key == projectless_chat_key or project_key.startswith("projectless:"):
        return True
    saved_keys = (
        saved_project_keys
        if saved_project_keys is not None
        else get_saved_workspace_project_keys(bridge_module=bridge_module)
    )
    if not saved_keys:
        return True
    return project_key in saved_keys


def filter_mirrorable_threads(
    threads: list[ThreadT],
    *,
    bridge_module: BridgeProjectModule,
    projectless_chat_key: str,
) -> list[ThreadT]:
    saved_project_keys = get_saved_workspace_project_keys(bridge_module=bridge_module)
    return [
        thread
        for thread in threads
        if is_thread_mirrorable(
            thread,
            saved_project_keys,
            bridge_module=bridge_module,
            projectless_chat_key=projectless_chat_key,
        )
    ]


def get_thread_cwd(thread_id: str | None, *, bridge_module: BridgeProjectModule) -> str | None:
    if not thread_id:
        return None
    try:
        thread = bridge_module.choose_thread(thread_id, None)
    except RuntimeError:
        return None
    cwd = _thread_cwd(thread, bridge_module=bridge_module)
    return cwd or None


def find_projectless_new_chat_cwd(*, home_path: Path | None = None) -> str | None:
    codex_docs = (home_path or Path.home()) / "Documents" / "Codex"
    if not codex_docs.exists():
        return None
    today_new_chat = codex_docs / datetime.now().strftime("%Y-%m-%d") / "new-chat"
    if today_new_chat.is_dir():
        return str(today_new_chat)
    candidates = [path for path in codex_docs.glob("????-??-??/new-chat") if path.is_dir()]
    if not candidates:
        return None
    candidates.sort(key=lambda path: path.stat().st_mtime, reverse=True)
    return str(candidates[0])


def project_keys_match(
    left: str | None,
    right: str | None,
    *,
    bridge_module: BridgeProjectModule,
    projectless_chat_key: str,
) -> bool:
    left_value = str(left or "").strip()
    right_value = str(right or "").strip()
    if not left_value or not right_value:
        return False
    if left_value == right_value:
        return True
    if left_value.startswith("projectless:") or right_value.startswith("projectless:"):
        return False
    if left_value == projectless_chat_key or right_value == projectless_chat_key:
        return False
    return normalize_project_key(
        left_value,
        bridge_module=bridge_module,
        projectless_chat_key=projectless_chat_key,
    ) == normalize_project_key(
        right_value,
        bridge_module=bridge_module,
        projectless_chat_key=projectless_chat_key,
    )
