"""Filesystem/process contract; only temporary fixture watchdogs are invoked."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / 'scripts' / 'Recover-CdrDeployment.ps1'


class DeploymentRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        binary = self.root / 'fixture.exe'
        binary.write_bytes(b'not executable; hash fixture only')
        watchdog = self.root / 'watchdog.ps1'
        # Exercise real recovery and marker/hash/receipt logic. Only OS process
        # providers are fixtures; this no longer treats any watchdog call as success.
        watchdog.write_text(r'''
param([string]$RepoRoot,[string]$BinaryPath,[string]$RecoverDeploymentStatePath)
$ErrorActionPreference='Stop'
if(-not $RecoverDeploymentStatePath){throw 'generic watchdog prohibited'}
. 'SOURCE/codex-discord-rust-drain.ps1'
. 'SOURCE/codex-discord-rust-control.ps1'
. 'SOURCE/scripts/CdrDeploymentRecovery.ps1'
. 'SOURCE/scripts/CdrLaunchJournal.ps1'
$StopPath=Join-Path $RepoRoot '.codex_discord_rust.stop'
$DisablePath=Join-Path $RepoRoot '.codex_discord_bot.disabled'
$DrainPreparePath=Join-Path $RepoRoot '.codex_discord_rust.drain.prepare'
$DrainAckPath=Join-Path $RepoRoot '.codex_discord_rust.drain.ack'
$RestartPath=Join-Path $RepoRoot '.codex_discord_rust.restart'
function Get-Process {param($Id,$ErrorAction) if($Id -eq 77 -and (Test-Path (Join-Path $RepoRoot 'started'))){[pscustomobject]@{Id=77;Path=$BinaryPath}}}
function Get-VerifiedRuntimeProcess {if(Test-Path (Join-Path $RepoRoot 'started')){[pscustomobject]@{Id=77}}}
function Get-RustProcessIdentity {param($Process) if($Process){'77|99'}else{''}}
function Get-VerifiedRuntimeIdentity {Get-RustProcessIdentity (Get-VerifiedRuntimeProcess)}
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$true;Bootstrap=$false}}
function Clear-DeadRuntimeArtifacts {}
function Start-RustRuntime {
 Set-CdrLaunchStarting
 [IO.File]::WriteAllText((Join-Path $RepoRoot 'started'),'yes')
 Set-CdrLaunchChild ([pscustomobject]@{Id=77;Path=$BinaryPath})
}
function Stop-VerifiedRuntime {throw 'force kill forbidden'}
$control=Enter-CdrControl $RepoRoot
try {Invoke-CdrDeploymentRecovery $RecoverDeploymentStatePath}finally{$control.Dispose()}
'''.replace('SOURCE', ROOT.as_posix().replace("'", "''")), encoding='utf-8')
        self.marker = self.root / '.codex_discord_bot.disabled'
        self.stop = self.root / '.codex_discord_rust.stop'
        self.marker.write_text('owned', encoding='utf-8')
        self.stop.write_text('owned', encoding='utf-8')
        self.state = self.root / 'state.json'
        self.state.write_text(json.dumps(dict(
            RepoRoot=str(self.root), BinaryPath=str(binary), Marker='owned',
            LockPath=str(self.root / 'guard.lock'), Watchdog=str(watchdog),
            BaselineHash=hashlib.sha256(binary.read_bytes()).hexdigest().upper(),
            CandidateHash='unused', LogPath=str(self.root / 'recovery.log'),
            RuntimePid=42, RuntimeTicks='99',
        )), encoding='utf-8')

    def recover(self):
        return subprocess.run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass',
                               '-File', str(SCRIPT), '-StatePath', str(self.state)],
                              capture_output=True, text=True, timeout=20)

    def test_dead_worker_recovers_and_repeat_is_safe(self):
        for _ in range(2):
            result = self.recover()
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(self.marker.exists())
            self.assertFalse(self.stop.exists())
            self.assertEqual((self.root / 'started').read_text(), 'yes')

    def test_foreign_marker_is_preserved(self):
        self.marker.write_text('user-disabled', encoding='utf-8')
        result = self.recover()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.marker.read_text(), 'user-disabled')
        self.assertTrue(self.stop.exists())
        self.assertFalse((self.root / 'started').exists())

    def test_other_lock_io_error_is_not_busy_or_success(self):
        state = json.loads(self.state.read_text())
        state['LockPath'] = str(self.root / 'missing-parent' / 'guard.lock')
        self.state.write_text(json.dumps(state), encoding='utf-8')
        result = self.recover()
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn('deployment_busy', result.stdout)
        self.assertTrue(self.marker.exists())
        self.assertFalse((self.root / 'started').exists())

    def test_live_worker_fences_recovery_then_process_death_releases(self):
        # Hold the real Windows exclusive file handle, signal readiness through stdout.
        lock_path = str(self.root / 'guard.lock').replace("'", "''")
        command = f"$f=[IO.File]::Open('{lock_path}','OpenOrCreate','ReadWrite','None'); Write-Output 'ready'; [Console]::Out.Flush(); [Console]::ReadLine() | Out-Null"
        worker = subprocess.Popen(['powershell.exe', '-NoProfile', '-Command', command],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        try:
            self.assertEqual(worker.stdout.readline().strip(), 'ready')
            result = self.recover()
            self.assertEqual(result.returncode, 75, result.stderr)
            self.assertIn('deployment_busy', result.stdout)
            self.assertTrue(self.marker.exists())
            self.assertFalse((self.root / 'started').exists())
        finally:
            worker.kill()  # Only this test's own fixture process, never the bot.
            worker.communicate(timeout=10)
        result = self.recover()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.marker.exists())
        self.assertTrue((self.root / 'started').exists())


if __name__ == '__main__':
    unittest.main()
