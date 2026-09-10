from __future__ import annotations

import json
from collections.abc import Callable
from dataclasses import dataclass, replace
from pathlib import Path
from typing import cast

import codex_app_server_transport as app_server_transport
from codex_app_server_transport_lifecycle import AppServerLifecycleSnapshot
from codex_plugin_runtime_fingerprint import (
    PluginContentFingerprintError,
    PluginInventoryFingerprintError,
    PluginRuntimeFingerprintError,
    fingerprint_required_plugins,
    read_codex_plugin_inventory,
)
from codex_pro_runtime_diagnostics import (
    ProDiagnosticCode,
    ProDiagnosticStage,
    ProRuntimePreflightError as ProRuntimePreflightError,
    preflight_error,
)
from codex_pro_runtime_inventory import PluginStatus, verify_plugin_inventory

PLUGIN_MANIFEST_PATH = (
    Path(__file__).resolve().parent
    / "plugins/codex-discord-remote/.codex-plugin/plugin.json"
)

InventoryReader = Callable[[], str]
ResidentSnapshotReader = Callable[[], AppServerLifecycleSnapshot]
RuntimeCheck = Callable[[], "ProRuntimeStatus"]
ResidentRefresh = Callable[[], bool]


@dataclass(frozen=True, slots=True)
class ProRuntimeStatus:
    remote_plugin_version: str
    browser_plugin_version: str
    resident_generation: int


def expected_remote_plugin_version(path: Path = PLUGIN_MANIFEST_PATH) -> str:
    try:
        raw = cast(object, json.loads(path.read_text(encoding="utf-8-sig")))
    except (OSError, json.JSONDecodeError) as exc:
        raise preflight_error(
            stage=ProDiagnosticStage.PLUGIN_MANIFEST,
            code=ProDiagnosticCode.REMOTE_MANIFEST_UNAVAILABLE,
            public_message="The remote plugin manifest could not be read.",
            recovery_action="Reinstall the remote plugin, then retry !pro.",
            internal_detail=f"remote plugin manifest is unavailable: {exc}",
        ) from exc
    if not isinstance(raw, dict):
        raise preflight_error(
            stage=ProDiagnosticStage.PLUGIN_MANIFEST,
            code=ProDiagnosticCode.REMOTE_MANIFEST_INVALID,
            public_message="The remote plugin manifest is invalid.",
            recovery_action="Reinstall the remote plugin, then retry !pro.",
            internal_detail="remote plugin manifest must be a JSON object",
        )
    version = cast("dict[str, object]", raw).get("version")
    if not isinstance(version, str) or not version:
        raise preflight_error(
            stage=ProDiagnosticStage.PLUGIN_MANIFEST,
            code=ProDiagnosticCode.REMOTE_MANIFEST_INVALID,
            public_message="The remote plugin manifest has no valid version.",
            recovery_action="Reinstall the remote plugin, then retry !pro.",
            internal_detail="remote plugin manifest.version must be a non-empty string",
        )
    return version


def verify_pro_runtime(
    *,
    inventory_json: str,
    expected_remote_version: str,
    resident_snapshot: AppServerLifecycleSnapshot,
    current_plugin_fingerprint: str,
) -> ProRuntimeStatus:
    plugin_status = verify_plugin_inventory(
        inventory_json,
        expected_remote_version=expected_remote_version,
    )
    return _verify_resident_runtime(
        plugin_status,
        resident_snapshot=resident_snapshot,
        current_plugin_fingerprint=current_plugin_fingerprint,
    )


def _verify_resident_runtime(
    plugin_status: PluginStatus,
    *,
    resident_snapshot: AppServerLifecycleSnapshot,
    current_plugin_fingerprint: str,
) -> ProRuntimeStatus:
    if not resident_snapshot.healthy or resident_snapshot.accepting_since is None:
        raise preflight_error(
            stage=ProDiagnosticStage.RESIDENT_APP_SERVER,
            code=ProDiagnosticCode.RESIDENT_UNHEALTHY,
            public_message="The resident Codex process is not ready to accept !pro.",
            recovery_action="Restart the remote bot, wait for it to become healthy, then retry !pro.",
            internal_detail=(
                "resident Codex app-server is not healthy "
                + f"(generation {resident_snapshot.generation})"
            ),
        )
    if resident_snapshot.plugin_runtime_error is not None:
        raise preflight_error(
            stage=ProDiagnosticStage.RESIDENT_APP_SERVER,
            code=ProDiagnosticCode.RESIDENT_SNAPSHOT_FAILED,
            public_message="The resident Codex process could not verify its plugin snapshot.",
            recovery_action="Repair the plugin installation and restart the remote bot.",
            internal_detail=(
                "resident Codex app-server plugin snapshot failed: "
                + resident_snapshot.plugin_runtime_error
            ),
        )
    resident_fingerprint = resident_snapshot.plugin_runtime_fingerprint
    if resident_fingerprint is None:
        raise preflight_error(
            stage=ProDiagnosticStage.RESIDENT_APP_SERVER,
            code=ProDiagnosticCode.RESIDENT_SNAPSHOT_MISSING,
            public_message="The resident Codex process started without a verified plugin snapshot.",
            recovery_action="Restart the remote bot, then retry !pro.",
            internal_detail=(
                "resident Codex app-server has no plugin snapshot "
                + f"(generation {resident_snapshot.generation})"
            ),
        )
    if resident_fingerprint != current_plugin_fingerprint:
        raise preflight_error(
            stage=ProDiagnosticStage.RESIDENT_APP_SERVER,
            code=ProDiagnosticCode.RESIDENT_STALE,
            public_message="The installed plugins changed after the resident Codex process started.",
            recovery_action="Restart the remote bot, then retry !pro.",
            internal_detail=(
                "resident Codex app-server plugin snapshot is stale "
                + f"(generation {resident_snapshot.generation}); restart the remote bot"
            ),
        )
    return ProRuntimeStatus(
        remote_plugin_version=plugin_status.remote_version,
        browser_plugin_version=plugin_status.browser_version,
        resident_generation=resident_snapshot.generation,
    )


def run_pro_runtime_preflight(
    *,
    inventory_reader: InventoryReader = read_codex_plugin_inventory,
    resident_snapshot_reader: ResidentSnapshotReader = (
        app_server_transport.DEFAULT_CLIENT.lifecycle_snapshot
    ),
    manifest_path: Path = PLUGIN_MANIFEST_PATH,
) -> ProRuntimeStatus:
    try:
        inventory_json = inventory_reader()
    except PluginRuntimeFingerprintError as exc:
        raise preflight_error(
            stage=ProDiagnosticStage.PLUGIN_INVENTORY,
            code=ProDiagnosticCode.PLUGIN_INVENTORY_QUERY_FAILED,
            public_message="Codex could not read the installed plugin inventory.",
            recovery_action="Run `codex plugin list --json`, fix the reported error, then retry !pro.",
            internal_detail=str(exc),
        ) from exc
    expected_version = expected_remote_plugin_version(manifest_path)
    plugin_status = verify_plugin_inventory(
        inventory_json,
        expected_remote_version=expected_version,
    )
    try:
        current_fingerprint = fingerprint_required_plugins(inventory_json)
    except PluginInventoryFingerprintError as exc:
        raise preflight_error(
            stage=ProDiagnosticStage.PLUGIN_INVENTORY,
            code=ProDiagnosticCode.PLUGIN_INVENTORY_INVALID,
            public_message="Codex returned invalid plugin source metadata.",
            recovery_action="Repair or reinstall the required plugins, then retry !pro.",
            internal_detail=f"plugin fingerprint inventory validation failed: {exc}",
        ) from exc
    except PluginContentFingerprintError as exc:
        raise preflight_error(
            stage=ProDiagnosticStage.PLUGIN_CONTENT,
            code=ProDiagnosticCode.PLUGIN_CONTENT_UNVERIFIED,
            public_message="The installed plugin files could not be verified; Chrome availability was not tested.",
            recovery_action="Repair or reinstall the required plugins, restart the remote bot, then retry !pro.",
            internal_detail=f"current Codex plugin fingerprint failed: {exc}",
        ) from exc
    except PluginRuntimeFingerprintError as exc:
        raise preflight_error(
            stage=ProDiagnosticStage.PLUGIN_INVENTORY,
            code=ProDiagnosticCode.PLUGIN_INVENTORY_INVALID,
            public_message="Codex could not classify the plugin verification result.",
            recovery_action="Repair or reinstall the required plugins, then retry !pro.",
            internal_detail=f"unclassified plugin fingerprint failure: {exc}",
        ) from exc
    return _verify_resident_runtime(
        plugin_status,
        resident_snapshot=resident_snapshot_reader(),
        current_plugin_fingerprint=current_fingerprint,
    )


def recover_stale_pro_runtime(
    check: RuntimeCheck,
    refresh: ResidentRefresh,
) -> ProRuntimeStatus:
    try:
        return check()
    except ProRuntimePreflightError as exc:
        if exc.diagnostic.code is not ProDiagnosticCode.RESIDENT_STALE:
            raise
        try:
            refreshed = refresh()
        except Exception as refresh_exc:  # noqa: BLE001  # noqa: BROAD_EXCEPT_OK - recovery boundary normalizes resident startup failures.
            raise ProRuntimePreflightError(
                replace(
                    exc.diagnostic,
                    internal_detail=(
                        exc.diagnostic.internal_detail
                        + "; automatic resident refresh failed "
                        + f"error_type={type(refresh_exc).__name__} "
                        + f"error={str(refresh_exc)[:300]}"
                    ),
                )
            ) from None
        if not refreshed:
            raise
    return check()


def run_pro_runtime_preflight_with_recovery() -> ProRuntimeStatus:
    return recover_stale_pro_runtime(
        run_pro_runtime_preflight,
        app_server_transport.DEFAULT_CLIENT.try_restart_if_quiescent,
    )
