from __future__ import annotations

import json
from pathlib import Path
from typing import NamedTuple, cast

ROOT = Path(__file__).resolve().parents[1]
PLUGIN_ID = "codex-discord-remote@codex-discord-remote"
MARKETPLACE_NAME = "codex-discord-remote"


class InventoryCase(NamedTuple):
    name: str
    marketplace_json: str
    plugin_json: str


def _expected_version() -> str:
    raw = cast(
        object,
        json.loads(
            (
                ROOT / "plugins/codex-discord-remote/.codex-plugin/plugin.json"
            ).read_text(encoding="utf-8")
        ),
    )
    if not isinstance(raw, dict):
        raise TypeError("plugin manifest must be a JSON object")
    version = cast("dict[str, object]", raw).get("version")
    if not isinstance(version, str):
        raise TypeError("plugin manifest version must be a string")
    return version


EXPECTED_VERSION = _expected_version()


def valid_inventory(repo_root: Path) -> tuple[str, str]:
    marketplace = {
        "marketplaces": [{"name": MARKETPLACE_NAME, "root": str(repo_root)}]
    }
    plugin = {
        "pluginId": PLUGIN_ID,
        "version": EXPECTED_VERSION,
        "installed": True,
        "enabled": True,
    }
    return json.dumps(marketplace), json.dumps({"installed": [plugin]})


def invalid_inventory_cases(repo_root: Path) -> tuple[InventoryCase, ...]:
    marketplace_json, plugin_json = valid_inventory(repo_root)
    marketplace = {
        "name": MARKETPLACE_NAME,
        "root": str(repo_root),
    }
    plugin = {
        "pluginId": PLUGIN_ID,
        "version": EXPECTED_VERSION,
        "installed": True,
        "enabled": True,
    }
    return (
        InventoryCase("marketplace_missing", '{"marketplaces": []}', plugin_json),
        InventoryCase("empty_object", "{}", "{}"),
        InventoryCase("malformed_json", "{not-json", plugin_json),
        InventoryCase(
            "wrong_root",
            json.dumps(
                {
                    "marketplaces": [
                        {**marketplace, "root": str(repo_root / "wrong")}
                    ]
                }
            ),
            plugin_json,
        ),
        InventoryCase(
            "duplicate_marketplace",
            json.dumps({"marketplaces": [marketplace, marketplace]}),
            plugin_json,
        ),
        InventoryCase("plugin_list_empty", marketplace_json, '{"installed": []}'),
        InventoryCase(
            "duplicate_plugin",
            marketplace_json,
            json.dumps({"installed": [plugin, plugin]}),
        ),
        InventoryCase(
            "plugin_not_installed",
            marketplace_json,
            json.dumps({"installed": [{**plugin, "installed": False}]}),
        ),
        InventoryCase(
            "plugin_disabled",
            marketplace_json,
            json.dumps({"installed": [{**plugin, "enabled": False}]}),
        ),
        InventoryCase(
            "version_mismatch",
            marketplace_json,
            json.dumps({"installed": [{**plugin, "version": "0.0.0-stale"}]}),
        ),
    )
