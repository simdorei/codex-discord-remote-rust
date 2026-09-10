from __future__ import annotations

import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class WindowsContractWorkflowTests(unittest.TestCase):
    def test_windows_rust_gate_preserves_every_native_failure(self) -> None:
        workflow = (ROOT / ".github/workflows/windows-contract.yml").read_text(
            encoding="utf-8"
        )
        for command in (
            "rustup show active-toolchain",
            "cargo build --workspace --locked",
            "cargo test --workspace --locked",
            "cargo clippy --workspace --all-targets --locked -- -D warnings",
        ):
            self.assertIn(
                command + "\n          if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }",
                workflow,
            )

    def test_windows_formatting_avoids_workspace_wide_argument_overflow(self) -> None:
        workflow = (ROOT / ".github/workflows/windows-contract.yml").read_text(
            encoding="utf-8"
        )
        script = (ROOT / "scripts/Test-RustFormatting.ps1").read_text(
            encoding="utf-8"
        )
        self.assertIn("./scripts/Test-RustFormatting.ps1", workflow)
        self.assertNotIn("cargo fmt --all", workflow)
        self.assertIn("cargo fmt --package $package.name -- --check", script)
        self.assertIn("$LASTEXITCODE -ne 0", script)

    def test_windows_workflow_covers_install_bridge_and_store_contracts(self) -> None:
        text = (ROOT / ".github" / "workflows" / "windows-contract.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("runs-on: windows-latest", text)
        self.assertIn("--require-hashes", text)
        self.assertIn("tests.test_install_scripts", text)
        self.assertIn("tests.test_install_plugin_contract", text)
        self.assertIn("tests.test_codex_desktop_bridge_type_exports", text)
        self.assertIn("tests.test_codex_discord_store_schema", text)
        self.assertIn("install.ps1 -DryRun", text)

    def test_connector_contracts_are_required_on_windows_and_macos(self) -> None:
        workflow_root = ROOT / ".github" / "workflows"
        windows = (workflow_root / "windows-contract.yml").read_text(encoding="utf-8")
        macos = (workflow_root / "macos-smoke.yml").read_text(encoding="utf-8")

        command = "python -m tests.pro_plugin_contract_suite"
        for workflow in (windows, macos):
            self.assertIn(command, workflow)
            self.assertIn("fetch-depth: 0", workflow)
            self.assertIn("PLUGIN_VERSION_BASE_REF", workflow)
            self.assertIn("github.event.pull_request.base.sha", workflow)
            self.assertIn("github.event.before", workflow)
            self.assertIn("python scripts/verify_plugin_cachebuster.py", workflow)

    def test_codex_exe_persistence_regression_runs_on_both_platforms(self) -> None:
        workflow_root = ROOT / ".github" / "workflows"
        test_module = "tests.test_install_codex_exe_persistence"
        for workflow_name in ("windows-contract.yml", "macos-smoke.yml"):
            workflow = (workflow_root / workflow_name).read_text(encoding="utf-8")
            self.assertIn(test_module, workflow)

    def test_rust_workspace_is_built_tested_and_linted_on_both_platforms(self) -> None:
        workflow_root = ROOT / ".github" / "workflows"
        required_commands = (
            "rustup show active-toolchain",
            "cargo build --workspace --locked",
            "cargo test --workspace --locked",
            "cargo clippy --workspace --all-targets --locked -- -D warnings",
        )
        for workflow_name in ("windows-contract.yml", "macos-smoke.yml"):
            workflow = (workflow_root / workflow_name).read_text(encoding="utf-8")
            for command in required_commands:
                self.assertIn(command, workflow)

    def test_connector_contract_suite_covers_release_critical_modules(self) -> None:
        suite = (ROOT / "tests" / "pro_plugin_contract_suite.py").read_text(
            encoding="utf-8"
        )

        required_modules = (
            "tests.test_pro_connector_control",
            "tests.test_pro_connector_evidence_hook",
            "tests.test_pro_connector_evidence_retry",
            "tests.test_pro_connector_receipt_ordering",
            "tests.test_codex_pro_connector_evidence",
            "tests.test_codex_pro_release_evidence",
            "tests.test_codex_discord_plugin_packaging",
            "tests.test_pro_conversation_map",
            "tests.test_verify_plugin_cachebuster",
            "tests.test_windows_contract_workflow",
        )
        for module in required_modules:
            self.assertIn(module, suite)


if __name__ == "__main__":
    _ = unittest.main()
