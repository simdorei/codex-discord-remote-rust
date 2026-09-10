from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[1]
WATCHDOG = ROOT / "codex-discord-rust-watchdog.ps1"
RESTART = ROOT / "codex-discord-rust-restart.ps1"
DRAIN = ROOT / "codex-discord-rust-drain.ps1"


@unittest.skipUnless(os.name == "nt", "Rust watchdog contracts run on Windows")
@unittest.skipUnless(shutil.which("powershell.exe"), "powershell.exe is required")
class RustWatchdogReadinessBoundaryTests(unittest.TestCase):
    def test_rust_restart_entrypoints_have_no_legacy_python_or_ipc_dependency(self) -> None:
        source = "\n".join(
            path.read_text(encoding="utf-8-sig") for path in (WATCHDOG, RESTART, DRAIN)
        ).casefold()
        forbidden = (
            ".py",
            "bridgepath",
            "resolvecodexruntimepythonexecutable",
            "codex_desktop_bridge",
            "codex-discord-watchdog-restart-runtime.ps1",
            "pending_codex_desktop_request",
            "\\\\.\\pipe",
        )
        for token in forbidden:
            with self.subTest(token=token):
                self.assertNotIn(token, source)

    def test_watchdog_delegates_restart_readiness_to_the_rust_runtime_cli(self) -> None:
        source = WATCHDOG.read_text(encoding="utf-8-sig")
        self.assertIn("--restart-readiness", source)
        self.assertIn("--restart-quiet-seconds", source)
        self.assertIn("--restart-wait-timeout-seconds", source)
        self.assertNotIn("Wait-CodexThreadsQuietForRestart", source)

    def test_readiness_function_never_invokes_a_fake_python_bridge(self) -> None:
        with tempfile.TemporaryDirectory() as raw_temp:
            root = Path(raw_temp)
            python_record = root / "python-invoked.txt"
            rust_record = root / "rust-invoked.txt"
            cwd_record = root / "rust-cwd.txt"
            fake_python = root / "fake-python.cmd"
            fake_rust = root / "fake-rust-runtime.cmd"
            legacy_helper = root / "codex-discord-watchdog-restart-runtime.ps1"

            fake_python.write_text(
                f'@echo invoked>"{python_record}"\n@exit /b 91\n',
                encoding="utf-8",
            )
            fake_rust.write_text(
                f'@echo %*>"{rust_record}"\n@cd>"{cwd_record}"\n@exit /b 0\n',
                encoding="utf-8",
            )
            legacy_helper.write_text(
                textwrap.dedent(
                    """
                    function Wait-CodexThreadsQuietForRestart {
                        & $env:CODEX_DISCORD_PYTHON $BridgePath
                        if ($LASTEXITCODE -ne 0) { throw 'fake Python failure' }
                    }
                    """
                ),
                encoding="utf-8",
            )

            command = textwrap.dedent(
                f"""
                $ErrorActionPreference = 'Stop'
                $source = Get-Content -LiteralPath {str(WATCHDOG)!r} -Raw
                $tokens = $null
                $errors = $null
                $ast = [Management.Automation.Language.Parser]::ParseInput(
                    $source, [ref]$tokens, [ref]$errors
                )
                if ($errors.Count -ne 0) {{ throw $errors[0].Message }}
                $function = $ast.Find({{
                    param($node)
                    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and
                    $node.Name -ceq 'Wait-RustThreadsQuietForRestart'
                }}, $true)
                if ($null -eq $function) {{ throw 'readiness function missing' }}
                Invoke-Expression $function.Extent.Text
                $RepoRoot = {str(root)!r}
                $BinaryPath = {str(fake_rust)!r}
                $EnvPath = {str(root / '.env')!r}
                $RestartQuietSeconds = 17
                $RestartWaitTimeoutSeconds = 23
                Wait-RustThreadsQuietForRestart
                """
            )
            env = os.environ.copy()
            env["CODEX_DISCORD_PYTHON"] = str(fake_python)
            completed = subprocess.run(
                ["powershell.exe", "-NoProfile", "-Command", command],
                capture_output=True,
                encoding="utf-8",
                errors="replace",
                env=env,
                timeout=15,
                check=False,
            )
            output = completed.stdout + completed.stderr
            self.assertEqual(completed.returncode, 0, output)
            self.assertFalse(python_record.exists(), output)
            recorded = rust_record.read_text(encoding="utf-8-sig").strip()
            self.assertEqual(
                recorded.replace("\\\\", "\\"),
                "--restart-readiness --restart-quiet-seconds 17 "
                f"--restart-wait-timeout-seconds 23 --env {root / '.env'}",
            )
            self.assertEqual(
                Path(cwd_record.read_text(encoding="utf-8-sig").strip()).resolve(),
                root.resolve(),
            )


if __name__ == "__main__":
    unittest.main()
