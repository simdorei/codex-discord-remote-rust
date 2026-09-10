from __future__ import annotations

import json
from dataclasses import dataclass
from typing import cast

from codex_plugin_runtime_fingerprint import CHROME_PLUGIN_ID, REMOTE_PLUGIN_ID
from codex_pro_runtime_diagnostics import (
    ProDiagnosticCode,
    ProDiagnosticStage,
    preflight_error,
)


@dataclass(frozen=True, slots=True)
class PluginStatus:
    remote_version: str
    browser_version: str


def verify_plugin_inventory(
    inventory_json: str,
    *,
    expected_remote_version: str,
) -> PluginStatus:
    inventory = _inventory_object(inventory_json)
    plugins = _plugin_records(inventory.get("installed"))
    remote = _required_plugin(plugins, REMOTE_PLUGIN_ID, browser=False)
    browser = _required_plugin(plugins, CHROME_PLUGIN_ID, browser=True)
    remote_version = _enabled_version(remote, REMOTE_PLUGIN_ID, browser=False)
    browser_version = _enabled_version(browser, CHROME_PLUGIN_ID, browser=True)
    if remote_version != expected_remote_version:
        raise preflight_error(
            stage=ProDiagnosticStage.PLUGIN_INVENTORY,
            code=ProDiagnosticCode.REMOTE_PLUGIN_VERSION_MISMATCH,
            public_message="The installed remote plugin version does not match this bot.",
            recovery_action="Reinstall the remote plugin and restart the remote bot.",
            internal_detail=(
                f"plugin {REMOTE_PLUGIN_ID!r} version mismatch: expected "
                + f"{expected_remote_version!r}, got {remote_version!r}"
            ),
        )
    return PluginStatus(remote_version, browser_version)


def _inventory_object(inventory_json: str) -> dict[str, object]:
    try:
        raw = cast(object, json.loads(inventory_json))
    except json.JSONDecodeError as exc:
        raise _invalid_inventory(
            f"Codex plugin inventory is not valid JSON: {exc}"
        ) from exc
    if not isinstance(raw, dict):
        raise _invalid_inventory("Codex plugin inventory must be a JSON object")
    return cast("dict[str, object]", raw)


def _plugin_records(value: object) -> tuple[dict[str, object], ...]:
    if not isinstance(value, list):
        raise _invalid_inventory(
            "Codex plugin inventory.installed must be a JSON array"
        )
    records: list[dict[str, object]] = []
    for item in cast("list[object]", value):
        if not isinstance(item, dict):
            raise _invalid_inventory(
                "Codex plugin inventory entries must be JSON objects"
            )
        records.append(cast("dict[str, object]", item))
    return tuple(records)


def _invalid_inventory(internal_detail: str) -> Exception:
    return preflight_error(
        stage=ProDiagnosticStage.PLUGIN_INVENTORY,
        code=ProDiagnosticCode.PLUGIN_INVENTORY_INVALID,
        public_message="Codex returned an invalid installed plugin inventory.",
        recovery_action=(
            "Run `codex plugin list --json`, fix the reported error, then retry !pro."
        ),
        internal_detail=internal_detail,
    )


def _required_plugin(
    records: tuple[dict[str, object], ...],
    plugin_id: str,
    *,
    browser: bool,
) -> dict[str, object]:
    matches = tuple(record for record in records if record.get("pluginId") == plugin_id)
    if len(matches) != 1:
        raise preflight_error(
            stage=ProDiagnosticStage.PLUGIN_INVENTORY,
            code=(
                ProDiagnosticCode.BROWSER_PLUGIN_MISSING
                if browser
                else ProDiagnosticCode.REMOTE_PLUGIN_MISSING
            ),
            public_message=(
                "The Chrome plugin entry is missing or duplicated; "
                + "Chrome availability was not tested."
                if browser
                else "The remote plugin entry is missing or duplicated."
            ),
            recovery_action=(
                "Reinstall and enable the Chrome plugin, then retry !pro."
                if browser
                else "Reinstall and enable the remote plugin, then retry !pro."
            ),
            internal_detail=f"plugin {plugin_id!r} was not installed exactly once",
        )
    return matches[0]


def _enabled_version(
    record: dict[str, object],
    plugin_id: str,
    *,
    browser: bool,
) -> str:
    if record.get("installed") is not True:
        raise preflight_error(
            stage=ProDiagnosticStage.PLUGIN_INVENTORY,
            code=_plugin_code(
                browser,
                ProDiagnosticCode.BROWSER_PLUGIN_NOT_INSTALLED,
                ProDiagnosticCode.REMOTE_PLUGIN_NOT_INSTALLED,
            ),
            public_message=_plugin_message(
                browser,
                "The Chrome plugin is listed but not installed; Chrome availability was not tested.",
                "The remote plugin is listed but not installed.",
            ),
            recovery_action=_plugin_message(
                browser,
                "Install and enable the Chrome plugin, then retry !pro.",
                "Install and enable the remote plugin, then retry !pro.",
            ),
            internal_detail=f"plugin {plugin_id!r} is not installed",
        )
    if record.get("enabled") is not True:
        raise preflight_error(
            stage=ProDiagnosticStage.PLUGIN_INVENTORY,
            code=_plugin_code(
                browser,
                ProDiagnosticCode.BROWSER_PLUGIN_DISABLED,
                ProDiagnosticCode.REMOTE_PLUGIN_DISABLED,
            ),
            public_message=_plugin_message(
                browser,
                "The Chrome plugin is installed but disabled; Chrome availability was not tested.",
                "The remote plugin is installed but disabled.",
            ),
            recovery_action=_plugin_message(
                browser,
                "Enable the Chrome plugin, then retry !pro.",
                "Enable the remote plugin, then retry !pro.",
            ),
            internal_detail=f"plugin {plugin_id!r} is not enabled",
        )
    version = record.get("version")
    if not isinstance(version, str) or not version:
        raise preflight_error(
            stage=ProDiagnosticStage.PLUGIN_INVENTORY,
            code=_plugin_code(
                browser,
                ProDiagnosticCode.BROWSER_PLUGIN_VERSION_INVALID,
                ProDiagnosticCode.REMOTE_PLUGIN_VERSION_INVALID,
            ),
            public_message=_plugin_message(
                browser,
                "The Chrome plugin has no valid version; Chrome availability was not tested.",
                "The remote plugin has no valid version.",
            ),
            recovery_action=_plugin_message(
                browser,
                "Reinstall the Chrome plugin, then retry !pro.",
                "Reinstall the remote plugin, then retry !pro.",
            ),
            internal_detail=f"plugin {plugin_id!r} version must be a non-empty string",
        )
    return version


def _plugin_code(
    browser: bool,
    browser_code: ProDiagnosticCode,
    remote_code: ProDiagnosticCode,
) -> ProDiagnosticCode:
    return browser_code if browser else remote_code


def _plugin_message(browser: bool, browser_message: str, remote_message: str) -> str:
    return browser_message if browser else remote_message
