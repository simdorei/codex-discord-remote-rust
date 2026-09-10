from __future__ import annotations

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import codex_plugin_runtime_fingerprint as fingerprint


REMOTE_PLUGIN = "codex-discord-remote@codex-discord-remote"
BROWSER_PLUGIN = "chrome@openai-bundled"


def _inventory(remote_root: Path, browser_root: Path) -> str:
    return json.dumps(
        {
            "installed": [
                {
                    "pluginId": REMOTE_PLUGIN,
                    "version": "1.2.3",
                    "installed": True,
                    "enabled": True,
                    "source": {"path": str(remote_root)},
                },
                {
                    "pluginId": BROWSER_PLUGIN,
                    "version": "9.8.7",
                    "installed": True,
                    "enabled": True,
                    "source": {"path": str(browser_root)},
                },
            ]
        }
    )


class PluginRuntimeFingerprintTests(unittest.TestCase):
    def test_inventory_uses_configured_executable_when_codex_is_not_on_path(
        self,
    ) -> None:
        # Given
        configured = "configured-codex.exe"
        inventory = '{"installed":[]}'
        completed = subprocess.CompletedProcess(
            [configured, "plugin", "list", "--json"],
            0,
            inventory,
            "",
        )

        # When
        with (
            mock.patch.dict(os.environ, {"CODEX_EXE": configured}),
            mock.patch(
                "codex_plugin_runtime_fingerprint.shutil.which",
                return_value=None,
            ) as which,
            mock.patch(
                "codex_plugin_runtime_fingerprint.subprocess.run",
                return_value=completed,
            ) as run,
        ):
            actual = fingerprint.read_codex_plugin_inventory()

        # Then
        self.assertEqual(actual, inventory)
        self.assertEqual(run.call_args.args[0][0], configured)
        which.assert_not_called()

    def test_fingerprint_is_deterministic_for_same_plugin_trees(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            root = Path(raw_dir)
            remote = root / "remote"
            browser = root / "browser"
            remote.mkdir()
            browser.mkdir()
            _ = (remote / "hook.py").write_text("value = 1\n", encoding="utf-8")
            _ = (browser / "client.mjs").write_text("export {};\n", encoding="utf-8")
            inventory = _inventory(remote, browser)

            first = fingerprint.fingerprint_required_plugins(inventory)
            second = fingerprint.fingerprint_required_plugins(inventory)

            self.assertEqual(first, second)
            self.assertRegex(first, r"^[0-9a-f]{64}$")

    def test_same_version_content_change_changes_fingerprint(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            root = Path(raw_dir)
            remote = root / "remote"
            browser = root / "browser"
            remote.mkdir()
            browser.mkdir()
            hook = remote / "hook.py"
            _ = hook.write_text("value = 1\n", encoding="utf-8")
            inventory = _inventory(remote, browser)
            before = fingerprint.fingerprint_required_plugins(inventory)

            _ = hook.write_text("value = 2\n", encoding="utf-8")
            after = fingerprint.fingerprint_required_plugins(inventory)

            self.assertNotEqual(before, after)

    def test_runtime_cache_files_do_not_make_generation_stale(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            root = Path(raw_dir)
            remote = root / "remote"
            browser = root / "browser"
            remote.mkdir()
            browser.mkdir()
            _ = (remote / "hook.py").write_text("value = 1\n", encoding="utf-8")
            inventory = _inventory(remote, browser)
            before = fingerprint.fingerprint_required_plugins(inventory)

            cache = remote / "__pycache__"
            cache.mkdir()
            _ = (cache / "hook.pyc").write_bytes(b"runtime-cache")
            after = fingerprint.fingerprint_required_plugins(inventory)

            self.assertEqual(before, after)

    def test_missing_or_disabled_required_plugin_fails_closed(self) -> None:
        cases = (
            (json.dumps({"installed": []}), "installed exactly once"),
            (
                json.dumps(
                    {
                        "installed": [
                            {
                                "pluginId": REMOTE_PLUGIN,
                                "version": "1",
                                "installed": True,
                                "enabled": False,
                                "source": {"path": "missing"},
                            }
                        ]
                    }
                ),
                "not enabled",
            ),
        )
        for inventory, expected in cases:
            with self.subTest(expected=expected):
                with self.assertRaisesRegex(
                    fingerprint.PluginRuntimeFingerprintError, expected
                ):
                    _ = fingerprint.fingerprint_required_plugins(inventory)

    def test_missing_plugin_source_tree_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            root = Path(raw_dir)
            missing = root / "missing"
            browser = root / "browser"
            browser.mkdir()

            with self.assertRaisesRegex(
                fingerprint.PluginRuntimeFingerprintError, "source path"
            ):
                _ = fingerprint.fingerprint_required_plugins(
                    _inventory(missing, browser)
                )

    def test_directory_junction_is_rejected_before_descending(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            root = Path(raw_dir)
            remote = root / "remote"
            browser = root / "browser"
            junction = remote / "junction"
            junction.mkdir(parents=True)
            browser.mkdir()
            _ = (junction / "outside.txt").write_text("outside", encoding="utf-8")

            original = fingerprint.is_directory_junction

            def fake_isjunction(path: Path) -> bool:
                return path == junction or original(path)

            with mock.patch.object(
                fingerprint, "is_directory_junction", side_effect=fake_isjunction
            ):
                with self.assertRaisesRegex(
                    fingerprint.PluginRuntimeFingerprintError,
                    "symbolic link or junction",
                ):
                    _ = fingerprint.fingerprint_required_plugins(
                        _inventory(remote, browser)
                    )

    def test_directory_scan_error_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            root = Path(raw_dir)
            remote = root / "remote"
            browser = root / "browser"
            remote.mkdir()
            browser.mkdir()

            with mock.patch("os.scandir", side_effect=PermissionError("denied")):
                with self.assertRaisesRegex(
                    fingerprint.PluginRuntimeFingerprintError,
                    "source tree could not be hashed.*denied",
                ):
                    _ = fingerprint.fingerprint_required_plugins(
                        _inventory(remote, browser)
                    )


if __name__ == "__main__":
    _ = unittest.main()
