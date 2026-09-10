from __future__ import annotations

from collections.abc import Mapping
from typing import Final, TypeAlias

from codex_model_catalog import (
    available_model_ids,
    available_reasoning_efforts,
    speed_service_tiers,
)
from codex_thread_settings_state import (
    JsonObject,
    JsonScalar,
    JsonValue,
    SavedThreadSettings,
    THREAD_SETTINGS_STATE_KEY,
    collaboration_mode_from_events,
    format_thread_model_display,
    remember_thread_settings,
    saved_thread_settings,
    service_tier_from_events,
    speed_from_service_tier,
)

ThreadSettingsUpdate: TypeAlias = dict[str, str | None]

THREAD_SETTINGS_MODELS: Final = ("gpt-5.4", "gpt-5.5")
THREAD_SETTINGS_REASONING_EFFORTS: Final = ("high", "xhigh")
THREAD_SETTINGS_SPEEDS: Final = ("fast", "standard")

__all__ = [
    "JsonObject",
    "JsonScalar",
    "JsonValue",
    "MissingThreadSettingError",
    "SavedThreadSettings",
    "THREAD_SETTINGS_MODELS",
    "THREAD_SETTINGS_REASONING_EFFORTS",
    "THREAD_SETTINGS_SPEEDS",
    "THREAD_SETTINGS_STATE_KEY",
    "ThreadSettingsError",
    "ThreadSettingsUpdate",
    "UnsupportedThreadSettingError",
    "build_thread_settings_update",
    "collaboration_mode_from_events",
    "format_thread_model_display",
    "remember_thread_settings",
    "saved_thread_settings",
    "service_tier_for_speed",
    "service_tier_from_events",
    "speed_from_service_tier",
]


class ThreadSettingsError(RuntimeError):
    pass


class UnsupportedThreadSettingError(ThreadSettingsError):
    setting: str
    value: str

    def __init__(self, setting: str, value: str) -> None:
        self.setting = setting
        self.value = value
        super().__init__(f"Unsupported {setting}: {value}")


class MissingThreadSettingError(ThreadSettingsError):
    def __init__(self) -> None:
        super().__init__("Specify at least one of --model, --reasoning, or --speed.")


def service_tier_for_speed(speed: str) -> str | None:
    if speed == "fast":
        return "priority"
    if speed == "standard":
        return None
    raise UnsupportedThreadSettingError("speed", speed)


def build_thread_settings_update(
    model: str | None,
    reasoning: str | None,
    speed: str | None,
    *,
    model_catalog: Mapping[str, JsonValue] | None = None,
    current_model: str | None = None,
) -> ThreadSettingsUpdate:
    settings: ThreadSettingsUpdate = {}
    allowed_models = THREAD_SETTINGS_MODELS
    allowed_reasoning = THREAD_SETTINGS_REASONING_EFFORTS
    allowed_speed_tiers: Mapping[str, str | None] = {
        speed_name: service_tier_for_speed(speed_name)
        for speed_name in THREAD_SETTINGS_SPEEDS
    }
    if model_catalog is not None:
        allowed_models = available_model_ids(model_catalog)
        if not allowed_models:
            raise ThreadSettingsError("No available models returned by app model/list.")
        if model is not None and model not in allowed_models:
            raise UnsupportedThreadSettingError("model", model)
        selected_model = model or (current_model if current_model in allowed_models else None)
        allowed_reasoning = available_reasoning_efforts(model_catalog, model=selected_model)
        allowed_speed_tiers = speed_service_tiers(model_catalog, model=selected_model)

    if model is not None:
        if model not in allowed_models:
            raise UnsupportedThreadSettingError("model", model)
        settings["model"] = model
    if reasoning is not None:
        if reasoning not in allowed_reasoning:
            raise UnsupportedThreadSettingError("reasoning", reasoning)
        settings["effort"] = reasoning
    if speed is not None:
        if speed not in allowed_speed_tiers:
            raise UnsupportedThreadSettingError("speed", speed)
        settings["serviceTier"] = allowed_speed_tiers[speed]
    if not settings:
        raise MissingThreadSettingError()
    return settings
