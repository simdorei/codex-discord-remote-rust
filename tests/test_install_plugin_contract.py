from __future__ import annotations

import os
import shutil
import tempfile
import unittest
from pathlib import Path

from tests.windows_install_plugin_contract_support import (
    FailureCommand,
    run_windows_installer,
)
from tests.install_plugin_inventory_cases import (
    invalid_inventory_cases,
    valid_inventory,
)


@unittest.skipUnless(os.name == "nt", "PowerShell installer test runs on Windows")
@unittest.skipUnless(shutil.which("powershell.exe"), "powershell.exe is required")
class WindowsPluginInstallContractTests(unittest.TestCase):
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
                    completed = run_windows_installer(
                        repo_root,
                        marketplace_json,
                        plugin_json,
                        failure_command=failure_command,
                    )

                output = completed.stdout + completed.stderr
                self.assertNotEqual(completed.returncode, 0, output)
                self.assertIn("INSTALL_INCOMPLETE", output)
                self.assertIn("codex stdout diagnostic", output)
                self.assertIn("error: unrecognized subcommand", output)
                self.assertIn("Update Codex or pass -CodexExe", output)
                self.assertNotIn("Skipping explicit plugin add", output)
                self.assertNotIn("Skipping plugin inventory verification", output)
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
                    completed = run_windows_installer(
                        repo_root,
                        marketplace_json,
                        plugin_json,
                        failure_command=failure_command,
                    )

                output = completed.stdout + completed.stderr
                self.assertNotEqual(completed.returncode, 0, output)
                self.assertIn("INSTALL_INCOMPLETE", output)
                self.assertIn("codex stdout diagnostic", output)
                self.assertIn("error: unrecognized subcommand", output)
                self.assertIn("Update Codex or pass -CodexExe", output)
                self.assertNotIn("Skipping explicit plugin add", output)
                self.assertNotIn("Skipping plugin inventory verification", output)
                self.assertNotIn("Install complete.", output)

    def test_successful_inventory_keeps_stderr_out_of_json(self) -> None:
        for success_stderr_command in ("marketplace_list", "plugin_list"):
            with self.subTest(success_stderr_command=success_stderr_command):
                with tempfile.TemporaryDirectory() as temp_dir:
                    repo_root = Path(temp_dir)
                    marketplace_json, plugin_json = valid_inventory(repo_root)
                    completed = run_windows_installer(
                        repo_root,
                        marketplace_json,
                        plugin_json,
                        success_stderr_command=success_stderr_command,
                    )

                output = completed.stdout + completed.stderr
                self.assertEqual(completed.returncode, 0, output)
                self.assertIn("warning:", output)
                self.assertIn("Verified Codex plugin inventory", output)
                self.assertIn("Install complete.", output)

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
                    completed = run_windows_installer(
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
            completed = run_windows_installer(
                repo_root, marketplace_json, plugin_json
            )

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("Verified Codex plugin inventory", output)
        self.assertIn("Install complete.", output)

    def test_cmd_capture_preserves_utf8_inventory_in_space_path(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir) / "한글 project %TEMP% ! & with spaces"
            repo_root.mkdir()
            marketplace_json, plugin_json = valid_inventory(repo_root)
            completed = run_windows_installer(
                repo_root,
                marketplace_json,
                plugin_json,
                utf8_inventory_output=True,
                use_system_python=True,
            )

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("warning: UTF-8 plugin inventory", output)
        self.assertNotIn("NativeCommandError", output)
        self.assertIn("Verified Codex plugin inventory", output)
        self.assertIn("Install complete.", output)

    def test_exe_capture_preserves_utf8_and_space_argument(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir) / "한글 exe project with spaces"
            repo_root.mkdir()
            marketplace_json, plugin_json = valid_inventory(repo_root)
            completed = run_windows_installer(
                repo_root,
                marketplace_json,
                plugin_json,
                codex_launcher="exe",
            )

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("warning: UTF-8 exe inventory", output)
        self.assertNotIn("NativeCommandError", output)
        self.assertIn("Verified Codex plugin inventory", output)
        self.assertIn("Install complete.", output)

    def test_ps1_shim_uses_sibling_cmd_launcher(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir) / "한글 shim project with spaces"
            repo_root.mkdir()
            marketplace_json, plugin_json = valid_inventory(repo_root)
            completed = run_windows_installer(
                repo_root,
                marketplace_json,
                plugin_json,
                codex_launcher="ps1",
                utf8_inventory_output=True,
            )

        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("warning: UTF-8 plugin inventory", output)
        self.assertNotIn("NativeCommandError", output)
        self.assertIn("Verified Codex plugin inventory", output)
        self.assertIn("Install complete.", output)

    def test_exe_capture_drains_large_stdout_and_stderr(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir) / "large output project"
            repo_root.mkdir()
            marketplace_json, plugin_json = valid_inventory(repo_root)
            completed = run_windows_installer(
                repo_root,
                marketplace_json,
                plugin_json,
                codex_launcher="exe",
                large_failure_output=True,
            )

        output = completed.stdout + completed.stderr
        self.assertNotEqual(completed.returncode, 0, output)
        self.assertIn("INSTALL_INCOMPLETE", output)
        self.assertIn("STDOUT_TAIL", output)
        self.assertIn("STDERR_TAIL", output)
        self.assertNotIn("Install complete.", output)

    def test_dry_run_does_not_claim_verified_install(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            repo_root = Path(temp_dir)
            marketplace_json, plugin_json = valid_inventory(repo_root)
            completed = run_windows_installer(
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
