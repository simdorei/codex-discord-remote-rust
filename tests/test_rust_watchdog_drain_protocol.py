from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[1]
SUPPORT = ROOT / "codex-discord-rust-drain.ps1"
WATCHDOG = ROOT / "codex-discord-rust-watchdog.ps1"
RESTART = ROOT / "codex-discord-rust-restart.ps1"


@unittest.skipUnless(os.name == "nt", "Rust watchdog contracts run on Windows")
@unittest.skipUnless(shutil.which("powershell.exe"), "powershell.exe is required")
class RustWatchdogDrainProtocolTests(unittest.TestCase):
    def run_powershell(self, body: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["powershell.exe", "-NoProfile", "-Command", body],
            capture_output=True,
            encoding="utf-8",
            errors="replace",
            timeout=15,
            check=False,
        )

    def test_watchdog_orders_ack_before_probe_and_probe_before_restart_marker(self) -> None:
        source = WATCHDOG.read_text(encoding="utf-8-sig")
        branch = source[source.index("if ($CheckRestartReady -or $PrepareRestart)") :]
        enter = branch.index("Enter-RestartDrain")
        assert_bound = branch.index("Assert-RestartDrainBound", enter)
        probe = branch.index("Wait-RustThreadsQuietForRestart", assert_bound)
        assert_after = branch.index("Assert-RestartDrainBound", probe)
        marker = branch.index("Write-BoundRestartMarker", assert_after)
        self.assertLess(enter, assert_bound)
        self.assertLess(assert_bound, probe)
        self.assertLess(probe, assert_after)
        self.assertLess(assert_after, marker)

    def test_watchdog_resumes_a_crashed_prepare_handshake_without_unsealing(self) -> None:
        source = WATCHDOG.read_text(encoding="utf-8-sig")
        self.assertIn("restart_drain_resume", source)
        self.assertIn("(Test-Path -LiteralPath $DrainPreparePath -PathType Leaf)", source)
        self.assertNotIn("Remove-Item -LiteralPath $DrainPreparePath", source)

    def test_prepare_failure_never_stops_or_kills_the_live_runtime(self) -> None:
        source = WATCHDOG.read_text(encoding="utf-8-sig")
        branch = source[source.index("if ($CheckRestartReady -or $PrepareRestart)") :]
        branch = branch[: branch.index("if (Test-Path -LiteralPath $StopPath)")]
        self.assertNotIn("Write-AtomicRestartMarker -Path $StopPath", branch)
        self.assertNotIn("Wait-RustRuntimeExit", branch)
        self.assertNotIn("Stop-VerifiedRuntime", branch)

    def test_hidden_deferred_restart_logs_the_actual_failure_message(self) -> None:
        source = RESTART.read_text(encoding="utf-8-sig")
        deferred = source[source.index("if ($Deferred)") :]
        deferred = deferred[: deferred.index("$currentIdentity")]
        self.assertIn("$_.Exception.Message", deferred)
        self.assertIn("restart_failed", deferred)

    def test_default_queued_restart_sets_an_explicit_success_exit_code(self) -> None:
        source = RESTART.read_text(encoding="utf-8-sig")
        queued = source[source.index("$escapedScript =") :]
        self.assertTrue(queued.rstrip().endswith("exit 0"))

    def test_timeout_leaves_prepare_visible_and_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as raw_temp:
            root = Path(raw_temp)
            command = textwrap.dedent(
                f"""
                . {str(SUPPORT)!r}
                $DrainIdentityPath = {str(root / 'identity')!r}
                $DrainPreparePath = {str(root / 'prepare')!r}
                $DrainAckPath = {str(root / 'ack')!r}
                function Get-RuntimeDrainIdentity {{
                    [pscustomobject]@{{ RuntimeId='runtime-a'; ProcessIdentity='42|99' }}
                }}
                function Get-VerifiedRuntimeIdentity {{ '42|99' }}
                function Get-VerifiedRuntimeProcess {{ [pscustomobject]@{{ Id=42 }} }}
                try {{
                    Enter-RestartDrain -Process ([pscustomobject]@{{ Id=42 }}) `
                        -ExpectedProcessIdentity '42|99' -TimeoutSeconds 0
                    throw 'expected timeout'
                }} catch {{
                    if ($_.Exception.Message -notmatch 'runtime_remains_sealed=true') {{ throw }}
                }}
                if (-not (Test-Path -LiteralPath $DrainPreparePath)) {{
                    throw 'prepare was silently removed'
                }}
                """
            )
            completed = self.run_powershell(command)
            self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)

    def test_stale_ack_is_rejected_instead_of_authorizing_restart(self) -> None:
        with tempfile.TemporaryDirectory() as raw_temp:
            root = Path(raw_temp)
            prepare = root / "prepare"
            ack = root / "ack"
            prepare.write_text(
                "version=1\nruntime_id=runtime-a\n"
                "process_identity=42|99\nnonce=current\n",
                encoding="utf-8",
            )
            ack.write_text(
                "version=1\nruntime_id=runtime-a\n"
                "process_identity=42|99\nnonce=stale\nstate=sealed\n",
                encoding="utf-8",
            )
            command = textwrap.dedent(
                f"""
                . {str(SUPPORT)!r}
                $DrainIdentityPath = {str(root / 'identity')!r}
                $DrainPreparePath = {str(prepare)!r}
                $DrainAckPath = {str(ack)!r}
                function Get-RuntimeDrainIdentity {{
                    [pscustomobject]@{{ RuntimeId='runtime-a'; ProcessIdentity='42|99' }}
                }}
                function Get-VerifiedRuntimeIdentity {{ '42|99' }}
                function Get-VerifiedRuntimeProcess {{ [pscustomobject]@{{ Id=42 }} }}
                try {{
                    Enter-RestartDrain -Process ([pscustomobject]@{{ Id=42 }}) `
                        -ExpectedProcessIdentity '42|99' -TimeoutSeconds 1
                    throw 'stale ack was accepted'
                }} catch {{
                    if ($_.Exception.Message -notmatch 'does not match') {{ throw }}
                }}
                exit 0
                """
            )
            completed = self.run_powershell(command)
            self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)

    def test_orphan_cleanup_requires_absence_of_verified_runtime(self) -> None:
        with tempfile.TemporaryDirectory() as raw_temp:
            root = Path(raw_temp)
            paths = [root / name for name in ("identity", "prepare", "ack")]
            for path in paths:
                path.write_text("sentinel", encoding="utf-8")
            command = textwrap.dedent(
                f"""
                . {str(SUPPORT)!r}
                $DrainIdentityPath = {str(paths[0])!r}
                $DrainPreparePath = {str(paths[1])!r}
                $DrainAckPath = {str(paths[2])!r}
                function Get-VerifiedRuntimeProcess {{ [pscustomobject]@{{ Id=42 }} }}
                try {{ Clear-OrphanedRestartDrainArtifacts; throw 'cleanup was accepted' }}
                catch {{ if ($_.Exception.Message -notmatch 'Refusing') {{ throw }} }}
                exit 0
                """
            )
            completed = self.run_powershell(command)
            self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
            self.assertTrue(all(path.exists() for path in paths))

            command = textwrap.dedent(
                f"""
                . {str(SUPPORT)!r}
                $DrainIdentityPath = {str(paths[0])!r}
                $DrainPreparePath = {str(paths[1])!r}
                $DrainAckPath = {str(paths[2])!r}
                function Get-VerifiedRuntimeProcess {{ $null }}
                Clear-OrphanedRestartDrainArtifacts
                """
            )
            completed = self.run_powershell(command)
            self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
            self.assertTrue(all(not path.exists() for path in paths))


if __name__ == "__main__":
    unittest.main()
