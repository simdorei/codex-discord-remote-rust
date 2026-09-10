from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(os.name == "nt", "Windows restart entry contract")
class RustWatchdogRestartEntryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="rust restart entry ")
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        for name in ("codex-discord-rust-watchdog.ps1", "codex-discord-rust-drain.ps1"):
            shutil.copyfile(ROOT / name, self.repo / name)
        shutil.copyfile(ROOT / 'codex-discord-rust-control.ps1', self.repo / 'codex-discord-rust-control.ps1')
        (self.repo / 'scripts').mkdir()
        shutil.copyfile(ROOT / 'scripts/CdrDeploymentRecovery.ps1', self.repo / 'scripts/CdrDeploymentRecovery.ps1')
        for name in ('CdrLaunchJournal.ps1', 'CdrRestartTransaction.ps1'):
            shutil.copyfile(ROOT / 'scripts' / name, self.repo / 'scripts' / name)
        (self.repo / "probe.exe").write_bytes(b"not an executable; process boundary is substituted")
        fence = "version=1\nruntime_id=runtime-a\nprocess_identity=42|99\nnonce=current\n"
        for name in ("restart", "drain.prepare", "drain.ack"):
            (self.repo / f".codex_discord_rust.{name}").write_text(
                fence + ("state=sealed\n" if name.endswith("ack") else ""), encoding="utf-8",
            )

    def run_entry(
        self, *, old_alive: bool, fail_first: bool = False, started_alive: bool = False,
    ) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["RESTART_ENTRY_ROOT"] = str(self.repo)
        command = """
            $ErrorActionPreference = 'Stop'
            $root = $env:RESTART_ENTRY_ROOT
            $source = Get-Content -LiteralPath (Join-Path $root 'codex-discord-rust-watchdog.ps1') -Raw
            $entryIndex = $source.IndexOf('# CONTROL_ENTRY:')
            if ($entryIndex -lt 0) { throw 'watchdog entry boundary missing' }
            # Execute the real initialization and helper definitions, then substitute only
            # process/probe boundaries. Actual normal-entry branching and marker IO run.
            . ([scriptblock]::Create($source.Substring(0, $entryIndex))) `
                -RepoRoot $root -BinaryPath (Join-Path $root 'probe.exe')
            $entry = [scriptblock]::Create($source.Substring($entryIndex))
            $script:oldAlive = OLD_ALIVE
            $script:startCount = 0
            $script:newAlive = $false
            function Get-VerifiedRuntimeProcess {
                if ($script:oldAlive) { [pscustomobject]@{ Id=42 } }
                elseif ($script:newAlive) { [pscustomobject]@{ Id=77 } } else { $null }
            }
            function Get-RustProcessIdentity { param($Process); if ($null -ne $Process) { "$($Process.Id)|99" } else { '' } }
            function Get-Process { param($Id,$ErrorAction)
                if($Id -eq 77 -and $script:newAlive){[pscustomobject]@{Id=77;Path=$BinaryPath}}
            }
            function Get-HeartbeatHealth { [pscustomobject]@{ Healthy=$true; Bootstrap=$false; State='healthy' } }
            function Enter-RestartDrain { throw 'unexpected second restart preparation' }
            function Wait-RustThreadsQuietForRestart { throw 'unexpected second readiness probe' }
            function Wait-RustRuntimeExit {
                param($Process, $ExpectedIdentity, $Reason)
                if ($ExpectedIdentity -ne '42|99' -or $Reason -ne 'restart_requested') { throw 'wrong exit binding' }
                Write-Output 'WAIT_EXIT'
                $script:oldAlive = $false
            }
            function Start-Process { throw 'test must not start a real process' }
            function Stop-VerifiedRuntime { throw 'test must not kill a process' }
            function Start-RustRuntime {
                param([switch]$ResumeRemoteMcp)
                if (-not $ResumeRemoteMcp) { throw 'restart handoff flag lost' }
                $script:startCount += 1
                Write-Output "START_ATTEMPT=$script:startCount"
                if (FAIL_FIRST -and $script:startCount -eq 1) {
                    if (STARTED_ALIVE) {
                        Set-CdrLaunchStarting
                        $script:RustRestartStartedProcess = [pscustomobject]@{ Id=77; HasExited=$false }
                    }
                    throw 'controlled launch failure'
                }
                $script:newAlive = $true
                Set-CdrLaunchStarting
                Set-CdrLaunchChild ([pscustomobject]@{Id=77;Path=$BinaryPath})
            }
            if (FAIL_FIRST) {
                try { & $entry; throw 'expected launch failure' }
                catch { if ($_.Exception.Message -notmatch 'controlled launch failure') { throw } }
                if (STARTED_ALIVE) {
                    if (Test-Path -LiteralPath $DrainAckPath) { throw 'unsafe restart authorization restored' }
                    $journal=Read-CdrLaunchJournal (Join-Path $RepoRoot '.codex_discord_rust.restart.launch')
                    if($journal.Phase -ne 'launching'){throw 'unknown launch was not fenced'}
                    try { & $entry; throw 'unsafe second start was accepted' }
                    catch { if ($_.Exception.Message -notmatch 'launch_outcome_unknown') { throw } }
                    exit 0
                }
                $journal=Read-CdrLaunchJournal (Join-Path $RepoRoot '.codex_discord_rust.restart.launch')
                if($journal.Phase -ne 'prepared' -or $journal.Fence.Nonce -ne 'current') {
                    throw 'retry authorization journal was lost'
                }
            }
            & $entry
        """.replace("OLD_ALIVE", "$true" if old_alive else "$false").replace(
            "FAIL_FIRST", "$true" if fail_first else "$false"
        ).replace(
            "STARTED_ALIVE", "$true" if started_alive else "$false"
        )
        return subprocess.run(
            ["powershell.exe", "-NoProfile", "-Command", command],
            env=env, capture_output=True, encoding="utf-8", errors="replace", timeout=15,
        )

    def assert_consumed(self) -> None:
        for name in ("restart", "drain.prepare", "drain.ack"):
            self.assertFalse((self.repo / f".codex_discord_rust.{name}").exists(), name)

    def test_already_bound_restart_waits_for_old_exit_without_second_probe(self) -> None:
        result = self.run_entry(old_alive=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertLess(result.stdout.index("WAIT_EXIT"), result.stdout.index("START_ATTEMPT=1"))
        self.assertEqual(result.stdout.count("START_ATTEMPT="), 1)
        self.assertNotIn("restart_drain_resume", (self.repo / "discord_launcher.log").read_text(encoding="utf-8-sig"))
        self.assert_consumed()

    def test_definite_launch_failure_retains_authorization_for_next_watchdog(self) -> None:
        result = self.run_entry(old_alive=False, fail_first=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("START_ATTEMPT=1", result.stdout)
        self.assertIn("START_ATTEMPT=2", result.stdout)
        self.assert_consumed()

    def test_live_uncertain_launch_never_authorizes_a_second_start(self) -> None:
        result = self.run_entry(old_alive=False, fail_first=True, started_alive=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stdout.count("START_ATTEMPT="), 1)
        self.assertFalse((self.repo / ".codex_discord_rust.drain.ack").exists())
        self.assertTrue((self.repo / ".codex_discord_rust.restart.launch").exists())

    def test_failed_launch_does_not_overwrite_another_drain_fence(self) -> None:
        prepare = self.repo / ".codex_discord_rust.drain.prepare"
        foreign = prepare.read_text(encoding="utf-8").replace("nonce=current", "nonce=another")
        prepare.write_text(foreign, encoding="utf-8")
        env = os.environ.copy()
        env["RESTART_ENTRY_ROOT"] = str(self.repo)
        result = subprocess.run(
            ["powershell.exe", "-NoProfile", "-Command", """
                $ErrorActionPreference = 'Stop'
                $root = $env:RESTART_ENTRY_ROOT
                . (Join-Path $root 'codex-discord-rust-drain.ps1')
                $DrainPreparePath = Join-Path $root '.codex_discord_rust.drain.prepare'
                $DrainAckPath = Join-Path $root '.codex_discord_rust.drain.ack'
                $fence = Get-RestartDrainFence -Path (Join-Path $root '.codex_discord_rust.restart')
                function Get-VerifiedRuntimeProcess { $null }
                function Get-RuntimePid { 0 }
                try { Restore-RestartDrainAfterFailedLaunch -Fence $fence; throw 'changed fence accepted' }
                catch { if ($_.Exception.Message -notmatch 'state changed') { throw } }
                exit 0
            """],
            env=env, capture_output=True, encoding="utf-8", errors="replace", timeout=15,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(prepare.read_text(encoding="utf-8"), foreign)


if __name__ == "__main__":
    unittest.main()
