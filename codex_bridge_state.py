from __future__ import annotations

import json
import sys
import time
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Final, TypeAlias

from codex_thread_settings import (
    remember_thread_settings as remember_thread_settings_in_state,
    saved_thread_settings,
)

JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
JsonObject: TypeAlias = dict[str, JsonValue]

_decode_json_value: Callable[[str], JsonValue] = json.loads


class BridgeStateFormatError(ValueError):
    pass


@dataclass(frozen=True, slots=True)
class CodexAppPackageVersionRecord:
    current_version: str | None
    previous_version: str | None
    update_detected: bool


CODEX_APP_PACKAGE_VERSION_KEY: Final = "codex_app_package_version"


def load_json(path: Path) -> JsonObject:
    if not path.exists():
        return {}
    value = _decode_json_value(path.read_text(encoding="utf-8-sig"))
    if not isinstance(value, dict):
        raise BridgeStateFormatError(f"{path} did not contain a JSON object.")
    return value


def save_json(path: Path, data: Mapping[str, JsonValue]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    _ = path.write_text(json.dumps(dict(data), ensure_ascii=False, indent=2), encoding="utf-8")


def _corrupt_bridge_state_backup_path(state_path: Path) -> Path:
    stamp = time.strftime("%Y%m%d-%H%M%S")
    candidate = state_path.with_name(f"{state_path.name}.corrupt-{stamp}.bak")
    if not candidate.exists():
        return candidate
    for index in range(1, 1000):
        candidate = state_path.with_name(f"{state_path.name}.corrupt-{stamp}-{index}.bak")
        if not candidate.exists():
            return candidate
    raise BridgeStateFormatError(f"Could not allocate corrupt bridge state backup path for {state_path}.")


def _repair_corrupt_bridge_state(state_path: Path, error: BaseException) -> JsonObject:
    backup_path = _corrupt_bridge_state_backup_path(state_path)
    _ = state_path.replace(backup_path)
    save_json(state_path, {})
    print(
        f"bridge_state_repaired: path={state_path} backup={backup_path} "
        + f"error={error.__class__.__name__}: {error}",
        file=sys.stderr,
    )
    return {}


def load_bridge_state(state_path: Path) -> JsonObject:
    try:
        return load_json(state_path)
    except (json.JSONDecodeError, UnicodeDecodeError, BridgeStateFormatError) as error:
        if not state_path.exists():
            return {}
        return _repair_corrupt_bridge_state(state_path, error)



def save_bridge_state(state_path: Path, data: Mapping[str, JsonValue]) -> None:
    save_json(state_path, data)


def get_saved_thread_settings(state_path: Path, thread_id: str) -> dict[str, str]:
    return saved_thread_settings(load_bridge_state(state_path), thread_id)


def remember_thread_settings(
    state_path: Path,
    thread_id: str,
    *,
    model: str | None,
    reasoning: str | None,
    speed: str | None,
) -> None:
    data = load_bridge_state(state_path)
    remember_thread_settings_in_state(
        data,
        thread_id,
        model=model,
        reasoning=reasoning,
        speed=speed,
    )
    save_bridge_state(state_path, data)


def get_selected_thread_id(state_path: Path) -> str | None:
    data = load_bridge_state(state_path)
    value = data.get("selected_thread_id")
    return value if isinstance(value, str) and value.strip() else None


def set_selected_thread_id(state_path: Path, thread_id: str | None) -> None:
    data = load_bridge_state(state_path)
    if thread_id:
        data["selected_thread_id"] = thread_id
    else:
        _ = data.pop("selected_thread_id", None)
    save_bridge_state(state_path, data)


def record_codex_app_package_version(state_path: Path, current_version: str | None) -> CodexAppPackageVersionRecord:
    normalized_current = str(current_version or "").strip()
    if not normalized_current:
        return CodexAppPackageVersionRecord(
            current_version=None,
            previous_version=None,
            update_detected=False,
        )

    data = load_bridge_state(state_path)
    previous_value = data.get(CODEX_APP_PACKAGE_VERSION_KEY)
    previous_version = str(previous_value).strip() if isinstance(previous_value, str) else None
    if not previous_version:
        previous_version = None

    data[CODEX_APP_PACKAGE_VERSION_KEY] = normalized_current
    save_bridge_state(state_path, data)
    return CodexAppPackageVersionRecord(
        current_version=normalized_current,
        previous_version=previous_version,
        update_detected=previous_version is not None and previous_version != normalized_current,
    )


def _stripped(value: JsonValue | None) -> str:
    return str(value or "").strip()


def _float_or_zero(value: JsonValue | None) -> float:
    if not isinstance(value, str | int | float | bool):
        return 0.0
    try:
        return float(value)
    except (TypeError, ValueError):
        return 0.0


def cache_live_approval_request(
    state_path: Path,
    pending_request: Mapping[str, JsonValue],
    *,
    now: Callable[[], float] = time.time,
) -> None:
    thread_id = _stripped(pending_request.get("thread_id"))
    if not thread_id:
        return
    request_kind = _stripped(pending_request.get("request_kind"))
    if request_kind not in {"commandExecution", "fileChange"}:
        return
    request_id = pending_request.get("request_id")
    if request_id is None:
        return

    data = load_bridge_state(state_path)
    cached_requests_value = data.get("recent_live_approval_requests")
    cached_requests = cached_requests_value if isinstance(cached_requests_value, dict) else {}

    cached_requests[thread_id] = {
        "captured_at": now(),
        "request_id": request_id,
        "request_kind": request_kind,
        "method": _stripped(pending_request.get("method")),
        "item_id": _stripped(pending_request.get("item_id")),
        "reason": _stripped(pending_request.get("reason")),
        "owner_client_id": _stripped(pending_request.get("owner_client_id")),
    }
    data["recent_live_approval_requests"] = cached_requests
    save_bridge_state(state_path, data)


def get_cached_live_approval_request(
    state_path: Path,
    thread_id: str,
    *,
    max_age_sec: float = 120.0,
    now: Callable[[], float] = time.time,
) -> JsonObject | None:
    data = load_bridge_state(state_path)
    cached_requests = data.get("recent_live_approval_requests")
    if not isinstance(cached_requests, dict):
        return None

    cached = cached_requests.get(thread_id)
    if not isinstance(cached, dict):
        return None

    captured_at_value = _float_or_zero(cached.get("captured_at"))
    if captured_at_value <= 0.0 or (now() - captured_at_value) > max_age_sec:
        return None

    request_id = cached.get("request_id")
    if request_id is None:
        return None

    request_kind = _stripped(cached.get("request_kind"))
    if request_kind not in {"commandExecution", "fileChange"}:
        return None

    return {
        "thread_id": thread_id,
        "request_id": request_id,
        "request_kind": request_kind,
        "method": _stripped(cached.get("method")),
        "item_id": _stripped(cached.get("item_id")),
        "reason": _stripped(cached.get("reason")),
        "owner_client_id": _stripped(cached.get("owner_client_id")),
    }


def clear_cached_live_approval_request(state_path: Path, thread_id: str) -> None:
    data = load_bridge_state(state_path)
    cached_requests = data.get("recent_live_approval_requests")
    if not isinstance(cached_requests, dict):
        return
    if thread_id not in cached_requests:
        return
    _ = cached_requests.pop(thread_id, None)
    if cached_requests:
        data["recent_live_approval_requests"] = cached_requests
    else:
        _ = data.pop("recent_live_approval_requests", None)
    save_bridge_state(state_path, data)
