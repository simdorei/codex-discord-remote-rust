from __future__ import annotations

import argparse
import json
import os
import sys
from collections.abc import Sequence
from pathlib import Path
from typing import Protocol, cast


class InventoryVerificationError(RuntimeError):
    """Raised when Codex reports an incomplete plugin installation."""


class _ParsedArgs(Protocol):
    marketplace_inventory: Path
    plugin_inventory: Path
    plugin_manifest: Path
    expected_root: Path
    marketplace_name: str
    plugin_id: str


def _json_object(path: Path, label: str) -> dict[str, object]:
    try:
        value = cast(object, json.loads(path.read_text(encoding="utf-8-sig")))
    except (OSError, json.JSONDecodeError) as exc:
        raise InventoryVerificationError(f"{label} is not valid JSON: {exc}") from exc
    if not isinstance(value, dict):
        raise InventoryVerificationError(f"{label} must be a JSON object")
    return cast("dict[str, object]", value)


def _object_list(value: object, label: str) -> tuple[dict[str, object], ...]:
    if not isinstance(value, list):
        raise InventoryVerificationError(f"{label} must be a JSON array")
    objects: list[dict[str, object]] = []
    for item in cast("list[object]", value):
        if not isinstance(item, dict):
            raise InventoryVerificationError(f"{label} entries must be JSON objects")
        objects.append(cast("dict[str, object]", item))
    return tuple(objects)


def _required_string(record: dict[str, object], field: str, label: str) -> str:
    value = record.get(field)
    if not isinstance(value, str) or not value:
        raise InventoryVerificationError(f"{label}.{field} must be a string")
    return value


def _normalized_path(value: str) -> str:
    return os.path.normcase(os.path.realpath(value)).rstrip("\\/")


def verify_inventory(
    *,
    marketplace_inventory_path: Path,
    plugin_inventory_path: Path,
    plugin_manifest_path: Path,
    expected_root: Path,
    marketplace_name: str,
    plugin_id: str,
) -> str:
    marketplace_inventory = _json_object(
        marketplace_inventory_path, "marketplace inventory"
    )
    plugin_inventory = _json_object(plugin_inventory_path, "plugin inventory")
    plugin_manifest = _json_object(plugin_manifest_path, "plugin manifest")
    expected_version = _required_string(plugin_manifest, "version", "plugin manifest")

    marketplaces = _object_list(
        marketplace_inventory.get("marketplaces"), "marketplace inventory.marketplaces"
    )
    matching_marketplaces = tuple(
        marketplace
        for marketplace in marketplaces
        if marketplace.get("name") == marketplace_name
    )
    if len(matching_marketplaces) != 1:
        raise InventoryVerificationError(
            f"marketplace {marketplace_name!r} was not registered exactly once"
        )
    actual_root = _required_string(
        matching_marketplaces[0], "root", f"marketplace {marketplace_name!r}"
    )
    if _normalized_path(actual_root) != _normalized_path(str(expected_root)):
        raise InventoryVerificationError(
            f"marketplace {marketplace_name!r} points to the wrong repository"
        )

    installed_plugins = _object_list(
        plugin_inventory.get("installed"), "plugin inventory.installed"
    )
    matching_plugins = tuple(
        plugin for plugin in installed_plugins if plugin.get("pluginId") == plugin_id
    )
    if len(matching_plugins) != 1:
        raise InventoryVerificationError(
            f"plugin {plugin_id!r} was not installed exactly once"
        )
    plugin = matching_plugins[0]
    if plugin.get("installed") is not True:
        raise InventoryVerificationError(f"plugin {plugin_id!r} is not installed")
    if plugin.get("enabled") is not True:
        raise InventoryVerificationError(f"plugin {plugin_id!r} is not enabled")
    actual_version = _required_string(plugin, "version", f"plugin {plugin_id!r}")
    if actual_version != expected_version:
        raise InventoryVerificationError(
            f"plugin {plugin_id!r} version mismatch: expected "
            + f"{expected_version!r}, got {actual_version!r}"
        )
    return expected_version


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    _ = parser.add_argument("--marketplace-inventory", type=Path, required=True)
    _ = parser.add_argument("--plugin-inventory", type=Path, required=True)
    _ = parser.add_argument("--plugin-manifest", type=Path, required=True)
    _ = parser.add_argument("--expected-root", type=Path, required=True)
    _ = parser.add_argument("--marketplace-name", required=True)
    _ = parser.add_argument("--plugin-id", required=True)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = cast(_ParsedArgs, cast(object, _parser().parse_args(argv)))
    version = verify_inventory(
        marketplace_inventory_path=args.marketplace_inventory,
        plugin_inventory_path=args.plugin_inventory,
        plugin_manifest_path=args.plugin_manifest,
        expected_root=args.expected_root,
        marketplace_name=args.marketplace_name,
        plugin_id=args.plugin_id,
    )
    print(f"Verified Codex plugin inventory: {args.plugin_id} {version}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except InventoryVerificationError as error:
        print(f"INSTALL_INCOMPLETE: {error}", file=sys.stderr)
        raise SystemExit(1) from error
