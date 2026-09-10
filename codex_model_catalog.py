from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import TypeAlias

from codex_model_catalog_speeds import (
    available_speeds as available_speeds,
    speed_service_tiers as speed_service_tiers,
)

JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
JsonObject: TypeAlias = dict[str, JsonValue]


def _clean(value: JsonValue | None) -> str:
    return value.strip() if isinstance(value, str) else ""


def _dedupe(values: Iterable[str]) -> tuple[str, ...]:
    result: list[str] = []
    seen: set[str] = set()
    for value in values:
        cleaned = value.strip()
        if not cleaned or cleaned in seen:
            continue
        seen.add(cleaned)
        result.append(cleaned)
    return tuple(result)


def _mapping_list(value: JsonValue | None) -> list[Mapping[str, JsonValue]]:
    if not isinstance(value, list):
        return []
    result: list[Mapping[str, JsonValue]] = []
    for item in value:
        if isinstance(item, dict):
            result.append(item)
    return result


def _visible_model_rows(catalog: Mapping[str, JsonValue]) -> list[Mapping[str, JsonValue]]:
    rows: list[Mapping[str, JsonValue]] = []
    for item in _mapping_list(catalog.get("data")):
        if item.get("hidden") is not True:
            rows.append(item)
    return rows


def _matching_model_rows(
    catalog: Mapping[str, JsonValue],
    model: str | None,
) -> list[Mapping[str, JsonValue]]:
    rows = _visible_model_rows(catalog)
    if not model:
        return rows
    matched = [
        row
        for row in rows
        if model in {_clean(row.get("id")), _clean(row.get("model"))}
    ]
    return matched


def available_model_ids(catalog: Mapping[str, JsonValue]) -> tuple[str, ...]:
    return _dedupe(
        _clean(row.get("model")) or _clean(row.get("id"))
        for row in _visible_model_rows(catalog)
    )


def available_reasoning_efforts(
    catalog: Mapping[str, JsonValue],
    *,
    model: str | None = None,
) -> tuple[str, ...]:
    efforts: list[str] = []
    for row in _matching_model_rows(catalog, model):
        supported = row.get("supportedReasoningEfforts")
        for item in _mapping_list(supported):
            efforts.append(_clean(item.get("reasoningEffort")))
    return _dedupe(efforts)


def format_settings_options(catalog: Mapping[str, JsonValue], field: str) -> str:
    normalized = field.strip().lower() or "all"
    lines: list[str] = []
    if normalized in {"all", "model"}:
        lines.append("models: " + (", ".join(available_model_ids(catalog)) or "-"))
    if normalized in {"all", "effort", "reasoning"}:
        lines.append("efforts: " + (", ".join(available_reasoning_efforts(catalog)) or "-"))
    if normalized in {"all", "speed"}:
        lines.append("speeds: " + (", ".join(available_speeds(catalog)) or "-"))
        lines.append("Use --speed standard when not using fast.")
    lines.append("source: app model/list")
    return "\n".join(lines)
