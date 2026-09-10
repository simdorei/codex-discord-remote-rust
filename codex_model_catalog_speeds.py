from __future__ import annotations

from collections.abc import Mapping
from typing import TypeAlias

JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]


def _clean(value: JsonValue | None) -> str:
    return value.strip() if isinstance(value, str) else ""


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
    return [
        row
        for row in rows
        if model in {_clean(row.get("id")), _clean(row.get("model"))}
    ]


def _service_tier_maps(row: Mapping[str, JsonValue]) -> tuple[dict[str, str], set[str]]:
    by_name: dict[str, str] = {}
    ids: set[str] = set()
    tiers = row.get("serviceTiers")
    for item in _mapping_list(tiers):
        tier_id = _clean(item.get("id"))
        tier_name = _clean(item.get("name")).lower()
        if tier_id:
            ids.add(tier_id)
        if tier_name and tier_id:
            by_name[tier_name] = tier_id
    return by_name, ids


def speed_service_tiers(
    catalog: Mapping[str, JsonValue],
    *,
    model: str | None = None,
) -> dict[str, str | None]:
    tiers_by_speed: dict[str, str | None] = {"standard": None}
    for row in _matching_model_rows(catalog, model):
        tier_by_name, tier_ids = _service_tier_maps(row)
        service_tiers = row.get("serviceTiers")
        for item in _mapping_list(service_tiers):
            speed_name = _clean(item.get("name")).lower()
            tier_id = _clean(item.get("id"))
            if speed_name and tier_id:
                _ = tiers_by_speed.setdefault(speed_name, tier_id)

        additional = row.get("additionalSpeedTiers")
        if not isinstance(additional, list):
            continue
        for item in additional:
            speed = _clean(item).lower()
            if not speed:
                continue
            service_tier = tier_by_name.get(speed)
            if not service_tier and speed == "fast" and "priority" in tier_ids:
                service_tier = "priority"
            _ = tiers_by_speed.setdefault(speed, service_tier or speed)
    return tiers_by_speed


def available_speeds(
    catalog: Mapping[str, JsonValue],
    *,
    model: str | None = None,
) -> tuple[str, ...]:
    return tuple(speed_service_tiers(catalog, model=model).keys())
