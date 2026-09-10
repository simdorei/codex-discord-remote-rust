from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(os.name == "nt", "Windows launcher contract")
class WindowsLauncherDisableTests(unittest.TestCase):
    def test_dl1_disabled_marker_blocks_python_and_rust_direct_launches(self) -> None:
        for runtime in ("python", "rust"):
            with self.subTest(runtime=runtime), tempfile.TemporaryDirectory() as temp_dir:
                temp_path = Path(temp_dir)
                launcher = temp_path / "codex-discord-bot.cmd"
                shutil.copyfile(ROOT / launcher.name, launcher)
                (temp_path / ".codex_discord_bot.disabled").write_text(
                    "operator_disabled\n", encoding="utf-8"
                )
                (temp_path / ".codex_discord_runtime").write_text(
                    runtime + "\n", encoding="utf-8"
                )
                env = os.environ.copy()
                env.pop("CODEX_DISCORD_RUNTIME", None)

                completed = subprocess.run(
                    ["cmd.exe", "/c", str(launcher)],
                    cwd=temp_path,
                    env=env,
                    capture_output=True,
                    text=True,
                    errors="replace",
                    timeout=10.0,
                    check=False,
                )

                output = completed.stdout + completed.stderr
                self.assertEqual(completed.returncode, 0, output)
                self.assertIn("Codex Discord bot launch is disabled", output)
                self.assertNotIn("Rust runtime not found", output)
                self.assertNotIn("Script not found", output)

    def test_dl2_headless_delegation_obeys_the_same_disabled_marker(self) -> None:
        headless = (ROOT / "codex-discord-bot-headless.vbs").read_text(
            encoding="utf-8-sig"
        )

        self.assertIn('fso.BuildPath(scriptDir, "codex-discord-bot.cmd")', headless)
        self.assertIn('shell.Run """" & target & """", 0, False', headless)
        self.assertNotIn("codex_discord_bot.py", headless)
        self.assertNotIn("cdr-runtime.exe", headless)

    def test_dl3_missing_runtime_selection_defaults_to_rust(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            launcher = temp_path / "codex-discord-bot.cmd"
            shutil.copyfile(ROOT / launcher.name, launcher)
            (temp_path / "codex_discord_bot.py").write_text(
                "raise SystemExit('python must not start')\n", encoding="utf-8"
            )
            env = os.environ.copy()
            env.pop("CODEX_DISCORD_RUNTIME", None)

            completed = subprocess.run(
                ["cmd.exe", "/c", str(launcher)],
                cwd=temp_path,
                env=env,
                capture_output=True,
                text=True,
                errors="replace",
                timeout=10.0,
                check=False,
            )

            output = completed.stdout + completed.stderr
            self.assertNotEqual(completed.returncode, 0, output)
            self.assertIn("Rust runtime not found", output)
            self.assertNotIn("frontend bridge is starting", output)
            self.assertNotIn("Portable Python", output)

    def test_dl4_invalid_runtime_selection_fails_closed(self) -> None:
        for source in ("environment", "mode_file"):
            with self.subTest(source=source), tempfile.TemporaryDirectory() as temp_dir:
                temp_path = Path(temp_dir)
                launcher = temp_path / "codex-discord-bot.cmd"
                shutil.copyfile(ROOT / launcher.name, launcher)
                (temp_path / "codex_discord_bot.py").write_text(
                    "raise SystemExit('python must not start')\n", encoding="utf-8"
                )
                env = os.environ.copy()
                env.pop("CODEX_DISCORD_RUNTIME", None)
                if source == "environment":
                    env["CODEX_DISCORD_RUNTIME"] = "pyhton"
                    (temp_path / ".codex_discord_runtime").write_text(
                        "rust\n", encoding="utf-8"
                    )
                else:
                    (temp_path / ".codex_discord_runtime").write_text(
                        "pyhton\n", encoding="utf-8"
                    )

                completed = subprocess.run(
                    ["cmd.exe", "/c", str(launcher)],
                    cwd=temp_path,
                    env=env,
                    capture_output=True,
                    text=True,
                    errors="replace",
                    timeout=10.0,
                    check=False,
                )

                output = completed.stdout + completed.stderr
                self.assertNotEqual(completed.returncode, 0, output)
                self.assertIn("Unsupported Codex Discord runtime selection", output)
                self.assertNotIn("frontend bridge is starting", output)
                self.assertNotIn("Portable Python", output)

    def test_dl5_powershell_entrypoints_default_to_rust(self) -> None:
        entrypoints = {
            "watchdog": ROOT / "codex-discord-watchdog.ps1",
            "restart": ROOT
            / "plugins"
            / "codex-discord-remote"
            / "scripts"
            / "restart.ps1",
            "status": ROOT
            / "plugins"
            / "codex-discord-remote"
            / "scripts"
            / "status.ps1",
        }
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            sentinel = temp_path / "rust-entrypoint.txt"
            watchdog = temp_path / "codex-discord-watchdog.ps1"
            shutil.copyfile(entrypoints["watchdog"], watchdog)
            (temp_path / "codex-discord-rust-watchdog.ps1").write_text(
                """
param(
    [string]$RepoRoot,
    [switch]$DryRun,
    [switch]$LogHealthy,
    [switch]$CheckRestartReady,
    [int]$RestartQuietSeconds,
    [int]$RestartWaitTimeoutSeconds,
    [int]$HealthCpuPercent,
    [int]$HealthFreeMemoryMb,
    [int]$HealthHeartbeatMaxAgeSeconds,
    [int]$HealthHeartbeatStartupGraceSeconds,
    [int]$HealthBadSampleLimit
)
Set-Content -LiteralPath $env:STRICT_RUNTIME_SENTINEL -Value watchdog
exit 0
""".lstrip(),
                encoding="utf-8",
            )
            (temp_path / "codex-discord-rust-restart.ps1").write_text(
                """
param(
    [string]$RepoRoot,
    [switch]$DryRun,
    [switch]$Immediate,
    [switch]$Deferred,
    [string]$ExpectedBotIdentity,
    [int]$DelaySeconds,
    [int]$QuietSeconds,
    [int]$WaitTimeoutSeconds
)
Set-Content -LiteralPath $env:STRICT_RUNTIME_SENTINEL -Value restart
exit 0
""".lstrip(),
                encoding="utf-8",
            )
            (temp_path / "codex-discord-rust-status.ps1").write_text(
                """
param([string]$RepoRoot)
Set-Content -LiteralPath $env:STRICT_RUNTIME_SENTINEL -Value status
exit 0
""".lstrip(),
                encoding="utf-8",
            )
            env = os.environ.copy()
            env.pop("CODEX_DISCORD_RUNTIME", None)
            env["STRICT_RUNTIME_SENTINEL"] = str(sentinel)

            invocations = {
                "watchdog": [str(watchdog), "-DryRun"],
                "restart": [
                    str(entrypoints["restart"]),
                    "-RepoRoot",
                    str(temp_path),
                    "-DryRun",
                ],
                "status": [
                    str(entrypoints["status"]),
                    "-RepoRoot",
                    str(temp_path),
                ],
            }
            for name, arguments in invocations.items():
                with self.subTest(entrypoint=name):
                    sentinel.unlink(missing_ok=True)
                    completed = subprocess.run(
                        [
                            "powershell.exe",
                            "-NoProfile",
                            "-ExecutionPolicy",
                            "Bypass",
                            "-File",
                            *arguments,
                        ],
                        cwd=temp_path,
                        env=env,
                        capture_output=True,
                        text=True,
                        errors="replace",
                        timeout=15.0,
                        check=False,
                    )
                    output = completed.stdout + completed.stderr
                    self.assertEqual(completed.returncode, 0, output)
                    self.assertEqual(
                        sentinel.read_text(encoding="utf-8-sig").strip(), name
                    )

    def test_dl6_powershell_entrypoints_reject_invalid_runtime(self) -> None:
        entrypoints = [
            ROOT / "codex-discord-watchdog.ps1",
            ROOT
            / "plugins"
            / "codex-discord-remote"
            / "scripts"
            / "restart.ps1",
            ROOT
            / "plugins"
            / "codex-discord-remote"
            / "scripts"
            / "status.ps1",
        ]
        env = os.environ.copy()
        env["CODEX_DISCORD_RUNTIME"] = "pyhton"
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            for entrypoint in entrypoints:
                with self.subTest(entrypoint=entrypoint.name):
                    arguments = [str(entrypoint)]
                    if entrypoint != ROOT / "codex-discord-watchdog.ps1":
                        arguments.extend(["-RepoRoot", str(temp_path)])
                    completed = subprocess.run(
                        [
                            "powershell.exe",
                            "-NoProfile",
                            "-ExecutionPolicy",
                            "Bypass",
                            "-File",
                            *arguments,
                        ],
                        cwd=temp_path,
                        env=env,
                        capture_output=True,
                        text=True,
                        errors="replace",
                        timeout=15.0,
                        check=False,
                    )
                    output = completed.stdout + completed.stderr
                    self.assertNotEqual(completed.returncode, 0, output)
                    self.assertIn(
                        "Unsupported Codex Discord runtime selection", output
                    )

    def test_dl7_every_python_entrypoint_labels_manual_rollback(self) -> None:
        paths = [
            ROOT / "codex-discord-bot.cmd",
            ROOT / "codex-discord-watchdog.ps1",
            ROOT
            / "plugins"
            / "codex-discord-remote"
            / "scripts"
            / "restart.ps1",
            ROOT
            / "plugins"
            / "codex-discord-remote"
            / "scripts"
            / "status.ps1",
        ]
        for path in paths:
            with self.subTest(path=path.name):
                self.assertIn(
                    "Explicit manual Python rollback",
                    path.read_text(encoding="utf-8"),
                )


if __name__ == "__main__":
    unittest.main()
