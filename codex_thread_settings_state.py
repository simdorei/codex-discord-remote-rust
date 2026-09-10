from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Final, TypeAlias

JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
JsonObject: TypeAlias = dict[str, JsonValue]
SavedThreadSettings: TypeAlias = dict[str, str]

THREAD_SETTINGS_STATE_KEY: Final = "thread_settings"

__all__ = [
    "JsonObject",
    "JsonScalar",
    "JsonValue",
    "SavedThreadSettings",
    "THREAD_SETTINGS_STATE_KEY",
    "collaboration_mode_from_events",
    "format_thread_model_display",
    "remember_thread_settings",
    "saved_thread_settings",
    "service_tier_from_events",
    "speed_from_service_tier",
]


def saved_thread_settings(state: Mapping[str, JsonValue], thread_id: str) -> SavedThreadSettings:
    all_settings = state.get(THREAD_SETTINGS_STATE_KEY)
    if not isinstance(all_settings, dict):
        return {}
    settings = all_settings.get(thread_id)
    if not isinstance(settings, dict):
        return {}
    result: SavedThreadSettings = {}
    for key in ("model", "reasoning", "speed"):
        value = settings.get(key)
        cleaned = _clean_string(value)
        if cleaned:
            result[key] = cleaned
    return result


def remember_thread_settings(
    state: JsonObject,
    thread_id: str,
    *,
    model: str | None,
    reasoning: str | None,
    speed: str | None,
) -> None:
    if model is None and reasoning is None and speed is None:
        return

    raw_all_settings = state.get(THREAD_SETTINGS_STATE_KEY)
    all_settings: JsonObject = dict(raw_all_settings) if isinstance(raw_all_settings, dict) else {}
    raw_settings = all_settings.get(thread_id)
    stored_settings: JsonObject = dict(raw_settings) if isinstance(raw_settings, dict) else {}

    if model is not None:
        stored_settings["model"] = model
    if reasoning is not None:
        stored_settings["reasoning"] = reasoning
    if speed is not None:
        stored_settings["speed"] = speed

    all_settings[thread_id] = stored_settings
    state[THREAD_SETTINGS_STATE_KEY] = all_settings


def collaboration_mode_from_events(events: Iterable[Mapping[str, JsonValue]]) -> str:
    mode = ""
    for event in events:
        payload = event.get("payload")
        if not isinstance(payload, dict):
            continue

        event_type = event.get("type")
        if event_type == "turn_context":
            collaboration_mode = payload.get("collaboration_mode")
            if isinstance(collaboration_mode, dict):
                raw_mode = _clean_string(collaboration_mode.get("mode"))
                if raw_mode:
                    mode = raw_mode
            continue
        if event_type == "event_msg" and payload.get("type") == "task_started":
            raw_mode = _clean_string(payload.get("collaboration_mode_kind"))
            if raw_mode:
                mode = raw_mode

    return mode


def speed_from_service_tier(value: JsonValue | str | None) -> str:
    if value is None:
        return "standard"
    normalized = _clean_string(value).lower()
    if normalized in {"priority", "fast"}:
        return "fast"
    if normalized in {"default", "standard", "none", "null"}:
        return "standard"
    return ""


def service_tier_from_events(events: Iterable[Mapping[str, JsonValue]]) -> str:
    speed = ""
    for event in events:
        payload = event.get("payload")
        if not isinstance(payload, dict):
            continue
        for key in ("service_tier", "serviceTier"):
            if key in payload:
                speed = speed_from_service_tier(payload.get(key))
    return speed


def format_thread_model_display(
    *,
    model: str,
    reasoning: str,
    mode: str,
    speed: str,
    saved_settings: Mapping[str, str],
) -> str:
    display_model = saved_settings.get("model", model).strip() or "-"
    display_reasoning = saved_settings.get("reasoning", reasoning).strip() or "-"
    display_mode = mode.strip() or "-"
    display_speed = speed.strip() or saved_settings.get("speed", "").strip() or "-"
    return _collapse_list_text(
        f"{display_model}/{display_reasoning}/{display_mode}/{display_speed}",
        limit=36,
    )


def _clean_string(value: JsonValue | str | None) -> str:
    if isinstance(value, str):
        return value.strip()
    return ""


def _collapse_list_text(value: str, limit: int) -> str:
    collapsed = " ".join((value or "").replace("\r", " ").replace("\n", " ").split())
    if len(collapsed) <= limit:
        return collapsed
    return collapsed[: max(0, limit - 3)].rstrip() + "..."
