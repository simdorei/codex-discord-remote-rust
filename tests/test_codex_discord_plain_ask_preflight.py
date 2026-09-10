from __future__ import annotations

import json
import tempfile
import unittest
from collections.abc import Callable
from pathlib import Path
from unittest import mock

import codex_discord_prompt_rewrite as prompt_rewrite
import codex_discord_prompt_mapped_delivery as mapped_delivery
import codex_pro_runtime_preflight as pro_preflight
from codex_pro_runtime_diagnostics import (
    ProDiagnosticCode,
    ProDiagnosticStage,
    preflight_error,
)
from codex_app_server_transport_lifecycle import AppServerLifecycleSnapshot
from codex_plugin_runtime_fingerprint import (
    PluginInventoryFingerprintError,
    PluginRuntimeFingerprintError,
)
from codex_remote_mcp_bridge_config import DeviceTicket
from codex_remote_mcp_bridge_connection import RemoteMcpBridgeError


REMOTE_PLUGIN = "codex-discord-remote@codex-discord-remote"
BROWSER_PLUGIN = "chrome@openai-bundled"
EXPECTED_VERSION = "1.2.3"


def _inventory(
    *,
    remote_installed: bool = True,
    remote_enabled: bool = True,
    remote_version: str = EXPECTED_VERSION,
    browser_installed: bool = True,
    browser_enabled: bool = True,
    remote_source: Path | None = None,
    browser_source: Path | None = None,
) -> str:
    remote: dict[str, object] = {
        "pluginId": REMOTE_PLUGIN,
        "version": remote_version,
        "installed": remote_installed,
        "enabled": remote_enabled,
    }
    browser: dict[str, object] = {
        "pluginId": BROWSER_PLUGIN,
        "version": "9.8.7",
        "installed": browser_installed,
        "enabled": browser_enabled,
    }
    if remote_source is not None:
        remote["source"] = {"path": str(remote_source)}
    if browser_source is not None:
        browser["source"] = {"path": str(browser_source)}
    return json.dumps(
        {
            "installed": [remote, browser]
        }
    )


def _healthy_resident() -> AppServerLifecycleSnapshot:
    return AppServerLifecycleSnapshot(
        generation=7,
        healthy=True,
        accepting_since=1.0,
        plugin_runtime_fingerprint="runtime-fingerprint",
    )


def _missing_bridge(
    root: Path,
    log: prompt_rewrite.LogFunc,
) -> None:
    _ = root, log


def _connected_device(
    root: Path,
    log: prompt_rewrite.LogFunc,
) -> DeviceTicket:
    _ = log
    return DeviceTicket(
        device_id="device-1",
        working_directory=root.resolve(),
    )


def _stale_bridge(
    root: Path,
    log: prompt_rewrite.LogFunc,
) -> DeviceTicket:
    _ = root, log
    raise RemoteMcpBridgeError(
        "The remote MCP device did not connect in time."
    )


class ProRuntimePreflightTests(unittest.TestCase):
    def test_accepts_enabled_plugins_and_healthy_resident(self) -> None:
        status = pro_preflight.verify_pro_runtime(
            inventory_json=_inventory(),
            expected_remote_version=EXPECTED_VERSION,
            resident_snapshot=_healthy_resident(),
            current_plugin_fingerprint="runtime-fingerprint",
        )

        self.assertEqual(status.remote_plugin_version, EXPECTED_VERSION)
        self.assertEqual(status.browser_plugin_version, "9.8.7")
        self.assertEqual(status.resident_generation, 7)

    def test_rejects_missing_disabled_or_stale_plugins(self) -> None:
        cases: tuple[tuple[str, str, str, ProDiagnosticCode], ...] = (
            (
                "remote_missing",
                json.dumps({"installed": []}),
                "codex-discord-remote@codex-discord-remote.*exactly once",
                ProDiagnosticCode.REMOTE_PLUGIN_MISSING,
            ),
            (
                "remote_disabled",
                _inventory(remote_enabled=False),
                "codex-discord-remote@codex-discord-remote.*not enabled",
                ProDiagnosticCode.REMOTE_PLUGIN_DISABLED,
            ),
            (
                "remote_stale",
                _inventory(remote_version="0.0.0-stale"),
                "version mismatch",
                ProDiagnosticCode.REMOTE_PLUGIN_VERSION_MISMATCH,
            ),
            (
                "browser_missing",
                _inventory(browser_installed=False),
                "chrome@openai-bundled.*not installed",
                ProDiagnosticCode.BROWSER_PLUGIN_NOT_INSTALLED,
            ),
            (
                "browser_disabled",
                _inventory(browser_enabled=False),
                "chrome@openai-bundled.*not enabled",
                ProDiagnosticCode.BROWSER_PLUGIN_DISABLED,
            ),
        )
        for name, inventory_json, expected, expected_code in cases:
            with self.subTest(name=name):
                with self.assertRaisesRegex(
                    pro_preflight.ProRuntimePreflightError,
                    expected,
                ) as raised:
                    _ = pro_preflight.verify_pro_runtime(
                        inventory_json=inventory_json,
                        expected_remote_version=EXPECTED_VERSION,
                        resident_snapshot=_healthy_resident(),
                        current_plugin_fingerprint="runtime-fingerprint",
                    )
                self.assertEqual(raised.exception.diagnostic.code, expected_code)
                self.assertEqual(
                    raised.exception.diagnostic.stage,
                    ProDiagnosticStage.PLUGIN_INVENTORY,
                )

    def test_rejects_malformed_inventory_and_unhealthy_resident(self) -> None:
        with self.assertRaisesRegex(
            pro_preflight.ProRuntimePreflightError,
            "not valid JSON",
        ) as malformed:
            _ = pro_preflight.verify_pro_runtime(
                inventory_json="{bad-json",
                expected_remote_version=EXPECTED_VERSION,
                resident_snapshot=_healthy_resident(),
                current_plugin_fingerprint="runtime-fingerprint",
            )
        self.assertEqual(
            malformed.exception.diagnostic.code,
            ProDiagnosticCode.PLUGIN_INVENTORY_INVALID,
        )

        with self.assertRaisesRegex(
            pro_preflight.ProRuntimePreflightError,
            "resident Codex app-server is not healthy",
        ) as unhealthy:
            _ = pro_preflight.verify_pro_runtime(
                inventory_json=_inventory(),
                expected_remote_version=EXPECTED_VERSION,
                resident_snapshot=AppServerLifecycleSnapshot(
                    generation=7,
                    healthy=False,
                    accepting_since=None,
                    restart_pending=True,
                ),
                current_plugin_fingerprint="runtime-fingerprint",
            )
        self.assertEqual(
            unhealthy.exception.diagnostic.code,
            ProDiagnosticCode.RESIDENT_UNHEALTHY,
        )

    def test_rejects_stale_or_missing_resident_plugin_fingerprint(self) -> None:
        cases = (
            (
                _healthy_resident(),
                "changed-fingerprint",
                "resident Codex app-server plugin snapshot is stale",
                ProDiagnosticCode.RESIDENT_STALE,
            ),
            (
                AppServerLifecycleSnapshot(
                    generation=7,
                    healthy=True,
                    accepting_since=1.0,
                    plugin_runtime_error="inventory query failed",
                ),
                "runtime-fingerprint",
                "inventory query failed",
                ProDiagnosticCode.RESIDENT_SNAPSHOT_FAILED,
            ),
            (
                AppServerLifecycleSnapshot(
                    generation=7,
                    healthy=True,
                    accepting_since=1.0,
                ),
                "runtime-fingerprint",
                "has no plugin snapshot",
                ProDiagnosticCode.RESIDENT_SNAPSHOT_MISSING,
            ),
        )
        for resident, current, expected, expected_code in cases:
            with self.subTest(expected=expected):
                with self.assertRaisesRegex(
                    pro_preflight.ProRuntimePreflightError, expected
                ) as raised:
                    _ = pro_preflight.verify_pro_runtime(
                        inventory_json=_inventory(),
                        expected_remote_version=EXPECTED_VERSION,
                        resident_snapshot=resident,
                        current_plugin_fingerprint=current,
                    )
                self.assertEqual(raised.exception.diagnostic.code, expected_code)

    def test_run_preflight_classifies_inventory_query_and_content_failures(
        self,
    ) -> None:
        def fail_inventory() -> str:
            raise PluginRuntimeFingerprintError("private inventory command failed")

        with self.assertRaises(pro_preflight.ProRuntimePreflightError) as query:
            _ = pro_preflight.run_pro_runtime_preflight(
                inventory_reader=fail_inventory,
                resident_snapshot_reader=_healthy_resident,
            )
        self.assertEqual(
            query.exception.diagnostic.code,
            ProDiagnosticCode.PLUGIN_INVENTORY_QUERY_FAILED,
        )

        with tempfile.TemporaryDirectory() as raw_dir:
            manifest = Path(raw_dir) / "plugin.json"
            _ = manifest.write_text(
                json.dumps({"version": EXPECTED_VERSION}),
                encoding="utf-8",
            )
            inventory = _inventory(
                remote_source=Path(raw_dir) / "missing-remote",
                browser_source=Path(raw_dir) / "missing-browser",
            )
            with self.assertRaises(pro_preflight.ProRuntimePreflightError) as content:
                _ = pro_preflight.run_pro_runtime_preflight(
                    inventory_reader=lambda: inventory,
                    resident_snapshot_reader=_healthy_resident,
                    manifest_path=manifest,
                )
        self.assertEqual(
            content.exception.diagnostic.code,
            ProDiagnosticCode.PLUGIN_CONTENT_UNVERIFIED,
        )
        self.assertIn(
            "Chrome availability was not tested",
            content.exception.diagnostic.public_message,
        )

    def test_fingerprint_inventory_failure_is_not_classified_as_content(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            manifest = Path(raw_dir) / "plugin.json"
            _ = manifest.write_text(
                json.dumps({"version": EXPECTED_VERSION}),
                encoding="utf-8",
            )
            with (
                mock.patch.object(
                    pro_preflight,
                    "fingerprint_required_plugins",
                    side_effect=PluginInventoryFingerprintError(
                        "source.path must be a non-empty string"
                    ),
                ),
                self.assertRaises(
                    pro_preflight.ProRuntimePreflightError
                ) as raised,
            ):
                _ = pro_preflight.run_pro_runtime_preflight(
                    inventory_reader=_inventory,
                    resident_snapshot_reader=_healthy_resident,
                    manifest_path=manifest,
                )

        self.assertEqual(
            raised.exception.diagnostic.code,
            ProDiagnosticCode.PLUGIN_INVENTORY_INVALID,
        )
        self.assertNotEqual(
            raised.exception.diagnostic.code,
            ProDiagnosticCode.PLUGIN_CONTENT_UNVERIFIED,
        )

    def test_manifest_failure_has_public_safe_diagnostic(self) -> None:
        missing_manifest = Path("definitely-missing-pro-plugin-manifest.json")

        with self.assertRaises(pro_preflight.ProRuntimePreflightError) as raised:
            _ = pro_preflight.run_pro_runtime_preflight(
                inventory_reader=_inventory,
                resident_snapshot_reader=_healthy_resident,
                manifest_path=missing_manifest,
            )

        self.assertEqual(
            raised.exception.diagnostic.code,
            ProDiagnosticCode.REMOTE_MANIFEST_UNAVAILABLE,
        )
        self.assertNotIn(
            str(missing_manifest),
            raised.exception.diagnostic.public_message,
        )


class ProPromptPreflightTests(unittest.TestCase):
    def _rewrite(
        self,
        *,
        checker: Callable[[], pro_preflight.ProRuntimeStatus],
        connector: prompt_rewrite.DeviceConnector,
    ) -> mapped_delivery.PromptPreprocessResult:
        return prompt_rewrite.rewrite_prompt(
            "!pro inspect",
            target_thread_id="thread-1",
            cwd=Path.cwd(),
            log=lambda _: None,
            runtime_preflight=checker,
            device_connector=connector,
        )

    def test_preflight_failure_exposes_only_public_diagnostic(self) -> None:
        def fail() -> pro_preflight.ProRuntimeStatus:
            raise preflight_error(
                stage=ProDiagnosticStage.PLUGIN_INVENTORY,
                code=ProDiagnosticCode.BROWSER_PLUGIN_DISABLED,
                public_message=(
                    "The Chrome plugin is installed but disabled; "
                    + "Chrome availability was not tested."
                ),
                recovery_action="Enable the Chrome plugin, then retry !pro.",
                internal_detail=(
                    "plugin path C:/private/browser project_scope=secret is not enabled"
                ),
            )

        result = self._rewrite(checker=fail, connector=_missing_bridge)

        self.assertFalse(result.should_deliver)
        self.assertIn("installed but disabled", result.visible_line)
        self.assertIn("Chrome availability was not tested", result.visible_line)
        self.assertIn("Enable the Chrome plugin", result.visible_line)
        self.assertIn("browser_plugin_disabled", result.visible_line)
        self.assertNotIn("C:/private", result.visible_line)
        self.assertNotIn("project_scope", result.visible_line)
        self.assertIn("C:/private", result.error_message)
        self.assertEqual(result.diagnostic_stage, "plugin_inventory")
        self.assertEqual(result.diagnostic_code, "browser_plugin_disabled")

    def test_missing_bridge_configuration_blocks_instead_of_falling_back(self) -> None:
        result = self._rewrite(
            checker=lambda: pro_preflight.ProRuntimeStatus(
                remote_plugin_version=EXPECTED_VERSION,
                browser_plugin_version="9.8.7",
                resident_generation=7,
            ),
            connector=_missing_bridge,
        )

        self.assertFalse(result.should_deliver)
        self.assertIn("remote MCP is not configured", result.error_message)
        self.assertIn("local PC connection is not configured", result.visible_line)
        self.assertEqual(result.diagnostic_code, "remote_mcp_not_configured")

    def test_connected_device_allows_transport_without_project_ticket(self) -> None:
        result = self._rewrite(
            checker=lambda: pro_preflight.ProRuntimeStatus(
                remote_plugin_version=EXPECTED_VERSION,
                browser_plugin_version="9.8.7",
                resident_generation=7,
            ),
            connector=_connected_device,
        )

        self.assertTrue(result.should_deliver)
        self.assertIn("<local-device-mcp", result.prompt)
        self.assertNotIn("project_scope", result.prompt)

    def test_stale_bridge_blocks_transport_with_bridge_reason(self) -> None:
        result = self._rewrite(
            checker=lambda: pro_preflight.ProRuntimeStatus(
                remote_plugin_version=EXPECTED_VERSION,
                browser_plugin_version="9.8.7",
                resident_generation=7,
            ),
            connector=_stale_bridge,
        )

        self.assertFalse(result.should_deliver)
        self.assertIn("did not connect", result.error_message)
        self.assertNotIn("did not connect", result.visible_line)
        self.assertEqual(result.diagnostic_code, "remote_mcp_connection_failed")


if __name__ == "__main__":
    _ = unittest.main()
