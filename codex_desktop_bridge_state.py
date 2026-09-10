from __future__ import annotations

import os
from pathlib import Path

from codex_bridge_state import (
    CodexAppPackageVersionRecord,
    JsonObject,
    cache_live_approval_request as cache_live_approval_request_in_state,
    clear_cached_live_approval_request as clear_cached_live_approval_request_in_state,
    get_cached_live_approval_request as get_cached_live_approval_request_from_state,
    get_saved_thread_settings as get_saved_thread_settings_from_state,
    get_selected_thread_id as get_selected_thread_id_from_state,
    load_bridge_state as load_bridge_state_from_path,
    load_json as load_json_from_path,
    record_codex_app_package_version as record_codex_app_package_version_in_state,
    remember_thread_settings as remember_thread_settings_in_state,
    save_bridge_state as save_bridge_state_to_path,
    save_json as save_json_to_path,
    set_selected_thread_id as set_selected_thread_id_in_state,
)


LIVE_APPROVAL_CACHE_MAX_AGE_SEC = 120.0


def get_env_path(name: str, default: Path) -> Path:
    value = os.environ.get(name, "").strip()
    if value:
        return Path(value).expanduser()
    return default


def resolve_state_db_path(codex_home: Path) -> Path:
    explicit = os.environ.get("CODEX_STATE_DB", "").strip()
    if explicit:
        return Path(explicit).expanduser()

    candidates: list[tuple[float, str, Path]] = []
    for path in codex_home.glob("state_*.sqlite"):
        if path.name.endswith((".sqlite-shm", ".sqlite-wal")):
            continue
        try:
            mtime = path.stat().st_mtime
        except OSError:
            mtime = 0.0
        candidates.append((mtime, path.name, path))

    if candidates:
        candidates.sort(reverse=True)
        return candidates[0][2]

    return codex_home / "state_5.sqlite"


def get_optional_env_file_path(name: str) -> Path | None:
    value = os.environ.get(name, "").strip()
    if not value:
        return None
    path = Path(value).expanduser()
    if path.exists() and path.is_file():
        return path
    return None


def get_float_env(name: str, default: float, *, minimum: float, maximum: float) -> float:
    raw = os.environ.get(name, "").strip()
    if not raw:
        return default
    try:
        value = float(raw)
    except ValueError:
        return default
    return max(minimum, min(maximum, value))


SCRIPT_DIR = Path(__file__).resolve().parent
BRIDGE_ENV_PATH = SCRIPT_DIR / ".env"
CODEX_HOME = get_env_path("CODEX_HOME", Path.home() / ".codex")
GLOBAL_STATE_PATH = get_env_path("CODEX_GLOBAL_STATE", CODEX_HOME / ".codex-global-state.json")
STATE_DB_PATH = resolve_state_db_path(CODEX_HOME)
SESSION_INDEX_PATH = get_env_path("CODEX_SESSION_INDEX", CODEX_HOME / "session_index.jsonl")
BRIDGE_STATE_PATH = get_env_path("CODEX_BRIDGE_STATE", CODEX_HOME / "codex_desktop_bridge_state.json")
LOG_DB_PATH = get_env_path("CODEX_LOG_DB", CODEX_HOME / "logs_2.sqlite")
ARCHIVED_SESSIONS_DIR = get_env_path("CODEX_ARCHIVED_SESSIONS_DIR", CODEX_HOME / "archived_sessions")
MAINTENANCE_BACKUP_ROOT = get_env_path("CODEX_MAINTENANCE_BACKUP_ROOT", CODEX_HOME / "maintenance_backups")


def load_json(path: Path) -> JsonObject:
    return load_json_from_path(path)


def save_json(path: Path, data: JsonObject) -> None:
    save_json_to_path(path, data)


def load_bridge_state() -> JsonObject:
    return load_bridge_state_from_path(BRIDGE_STATE_PATH)


def save_bridge_state(data: JsonObject) -> None:
    save_bridge_state_to_path(BRIDGE_STATE_PATH, data)


def get_saved_thread_settings(thread_id: str) -> dict[str, str]:
    return get_saved_thread_settings_from_state(BRIDGE_STATE_PATH, thread_id)


def remember_thread_settings(
    thread_id: str,
    *,
    model: str | None,
    reasoning: str | None,
    speed: str | None,
) -> None:
    remember_thread_settings_in_state(
        BRIDGE_STATE_PATH,
        thread_id,
        model=model,
        reasoning=reasoning,
        speed=speed,
    )


def get_selected_thread_id() -> str | None:
    return get_selected_thread_id_from_state(BRIDGE_STATE_PATH)


def set_selected_thread_id(thread_id: str | None) -> None:
    set_selected_thread_id_in_state(BRIDGE_STATE_PATH, thread_id)


def record_codex_app_package_version(current_version: str | None) -> CodexAppPackageVersionRecord:
    return record_codex_app_package_version_in_state(BRIDGE_STATE_PATH, current_version)


def cache_live_approval_request(pending_request: JsonObject) -> None:
    cache_live_approval_request_in_state(BRIDGE_STATE_PATH, pending_request)


def get_cached_live_approval_request(
    thread_id: str,
    *,
    max_age_sec: float = LIVE_APPROVAL_CACHE_MAX_AGE_SEC,
) -> JsonObject | None:
    return get_cached_live_approval_request_from_state(
        BRIDGE_STATE_PATH,
        thread_id,
        max_age_sec=max_age_sec,
    )


def clear_cached_live_approval_request(thread_id: str) -> None:
    clear_cached_live_approval_request_in_state(BRIDGE_STATE_PATH, thread_id)
