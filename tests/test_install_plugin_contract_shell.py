from __future__ import annotations

import shutil
import tempfile
import unittest
from pathlib import Path

from tests.install_plugin_contract_support import (
    FailureCommand,
    run_shell_installer,
)
from tests.install_plugin_inventory_cases import (
    invalid_inventory_cases,
    valid_inventory,
)


@unittest.skipIf(shutil.which("sh") is None, "sh is required")
class ShellPluginInstallContractTests(unittest.TestCase):
    def test_install_fails_when_required_codex_plugin_step_fails(self) -> None:
        failure_commands: tuple[FailureCommand, ...] = (
            "marketplace_add",
            "plugin_add",
        )
        for failure_command in failure_commands:
            with self.subTest(failure_command=failure_command):
                with tempfile.TemporaryDirectory() as temp_dir:
                    repo_root = Path(temp_dir)
                    marketplace_json, plugin_json = valid_inventory(repo_root)
                    completed = run_shell_installer(
                        repo_root,
                        marketplace_json,
                        plugin_json,
                        failure_command=failure_command,
                    )

                output = completed.stdout + completed.stderr
                self.assertNotEqual(completed.returncode, 0, output)
                self.assertIn("INSTALL_INCOMPLETE", output)
                self.assertNotIn("Install complete.", output)

    def test_install_fails_when_inventory_query_fails(self) -> None:
        failure_commands: tuple[FailureCommand, ...] = (
            "marketplace_list",
            "plugin_list",
        )
        for failure_command in failure_commands:
            with self.subTest(failure_command=failure_command):
                with tempfile.TemporaryDirectory() as temp_dir:
                    repo_root = Path(temp_dir)
                    marketplace_json, plugin_json = valid_inventory(repo_root)
                    completed = run_shell_installer(
                        repo_root,
                        marketplace_json,
                        plugin_json,
                        failure_command=failure_command,
                    )

                output = completed.stdout + completed.stderr
                self.assertNotEqual(completed.returncode, 0, output)
                self.assertIn("INSTALL_INCOMPLETE", output)
                self.assertNotIn("Install complete.", output)

    def test_install_rejects_unverified_plugin_inventory(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            cases = invalid_inventory_cases(Path(temp_dir))

        for case in cases:
            with self.subTest(case_name=case.name):
                with tempfile.TemporaryDirectory() as temp_dir:
                    repo_root = Path(temp_dir)
                    matching_case = next(
                        item
                        for item in invalid_inventory_cases(repo_root)
                        if item.name == case.name
                    )
                    completed = run_shell_installer(
                        repo_root,
                        matching_case.marketplace_json,
                        matching_case.plugin_json,
                    )

                output = completed.stdout + completed.stderr
                self.assertNotEqual(completed.returncode, 0, output)
                self.assertIn("INSTALL_INCOMPLETE", output)
                self.assertNotIn("Install complete.", output)

    def test_install_accepts_verified_plugin_inventory(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            marketplace_json, plugin_json = valid_inventory(repo_root)
            completed = run_shell_installer(repo_root, marketplace_json, plugin_json)

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("Verified Codex plugin inventory", output)
        self.assertIn("Install complete.", output)

    def test_dry_run_does_not_claim_verified_install(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            marketplace_json, plugin_json = valid_inventory(repo_root)
            completed = run_shell_installer(
                repo_root,
                marketplace_json,
                plugin_json,
                dry_run=True,
            )

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("Dry run complete.", output)
        self.assertIn("Plugin inventory was not verified.", output)
        self.assertNotIn("Install complete.", output)


if __name__ == "__main__":
    _ = unittest.main()
