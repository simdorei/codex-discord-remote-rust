"""Execute the real watchdog entry/transaction after deterministic filesystem faults."""
import os
from pathlib import Path
import subprocess
import unittest
import test_rust_watchdog_restart_entry as fixtures

ROOT = Path(__file__).resolve().parents[1]


class RestartTransactionRecoveryTests(unittest.TestCase):
    setUp = fixtures.RustWatchdogRestartEntryTests.setUp

    def run_boundary(self, body, prepare=False):
        command = r'''
$ErrorActionPreference='Stop'
$root=$env:ENTRY_ROOT
$source=Get-Content (Join-Path $root 'codex-discord-rust-watchdog.ps1') -Raw
$boundary=$source.IndexOf('# CONTROL_ENTRY:')
. ([scriptblock]::Create($source.Substring(0,$boundary))) -RepoRoot $root -BinaryPath (Join-Path $root 'probe.exe') -PrepareRestart:PREPARE
$entry=[scriptblock]::Create($source.Substring($boundary))
$script:newAlive=$false;$script:starts=0
function Get-VerifiedRuntimeProcess {if($script:newAlive){[pscustomobject]@{Id=77;Path=$BinaryPath}}}
function Get-RustProcessIdentity {param($Process) if($Process){"$($Process.Id)|99"}else{''}}
function Get-Process {param($Id,$ErrorAction) if($Id -eq 77 -and $script:newAlive){[pscustomobject]@{Id=77;Path=$BinaryPath}}}
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$true;Bootstrap=$false}}
function Stop-VerifiedRuntime {throw 'kill forbidden'}
function Start-Process {throw 'real start forbidden'}
function Start-RustRuntime {
 param([switch]$ResumeRemoteMcp)
 if(-not $ResumeRemoteMcp){throw 'handoff flag lost'}
 Set-CdrLaunchStarting
 $script:starts++;$script:newAlive=$true
 Set-CdrLaunchChild ([pscustomobject]@{Id=77;Path=$BinaryPath})
}
'''.replace('PREPARE', '$true' if prepare else '$false')
        result = subprocess.run(
            ['powershell.exe', '-NoProfile', '-Command', command + body + '\nexit 0'],
            env={**os.environ, 'ENTRY_ROOT': str(self.repo)}, capture_output=True,
            encoding='utf-8', errors='replace', timeout=20)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_receipt_write_failure_next_entry_adopts_same_child(self):
        self.run_boundary(r'''
$realAtomic=(Get-Command Write-AtomicRestartMarker).ScriptBlock
$script:failReceipt=$true
function Write-AtomicRestartMarker {param($Path,$Text)
 if($Path.EndsWith('.restart.completed') -and $script:failReceipt){$script:failReceipt=$false;throw 'injected receipt failure'}
 & $realAtomic -Path $Path -Text $Text
}
try{& $entry;throw 'receipt error hidden'}catch{if($_.Exception.Message -notmatch 'injected receipt failure'){throw}}
if($script:starts -ne 1){throw 'first launch missing'}
$script:CdrLaunchJournalPath=$null;$script:RustRestartStartedProcess=$null
& $entry
if($script:starts -ne 1){throw 'duplicate launch'}
if(Test-Path (Join-Path $root '.codex_discord_rust.restart.launch')){throw 'journal not completed'}
if(-not (Test-Path (Join-Path $root '.codex_discord_rust.restart.completed'))){throw 'receipt missing'}
''')

    def test_prelaunch_interruption_after_claim_or_drain_cleanup_is_recoverable(self):
        self.run_boundary(r'''
$fence=Get-RestartDrainFence $RestartPath
$journalPath=Join-Path $root '.codex_discord_rust.restart.launch'
$record=New-CdrLaunchJournal $journalPath ('restart:'+$fence.Nonce) $fence
$record.ClaimPath=$RestartClaimPath
Save-CdrLaunchJournal $journalPath $record
[IO.File]::Move($RestartPath,$RestartClaimPath)
[IO.File]::Delete($DrainPreparePath)
# Simulate a later entry after a crash with partial cleanup. No in-memory receipt.
& $entry
if($script:starts -ne 1){throw 'recovery did not start exactly once'}
if(Test-Path $RestartClaimPath){throw 'claim leaked'}
''')

    def test_prepare_preserves_existing_invalid_restart_without_entering_drain(self):
        (self.repo / '.codex_discord_rust.restart').write_text('foreign-invalid', encoding='utf-8')
        self.run_boundary(r'''
function Enter-RestartDrain {throw 'DRAIN MUST NOT BE CALLED'}
try{& $entry;throw 'foreign marker accepted'}catch{if($_.Exception.Message -notmatch 'explicit preparation refused before drain'){throw}}
if([IO.File]::ReadAllText($RestartPath) -cne 'foreign-invalid'){throw 'foreign marker changed'}
''', prepare=True)

    def test_prepare_preserves_stop_and_disabled_without_entering_drain(self):
        (self.repo / '.codex_discord_rust.restart').unlink()
        for name in ('.codex_discord_rust.stop', '.codex_discord_bot.disabled'):
            with self.subTest(name=name):
                path = self.repo / name
                path.write_text('foreign', encoding='utf-8')
                self.run_boundary(r'''
function Enter-RestartDrain {throw 'DRAIN MUST NOT BE CALLED'}
try{& $entry;throw 'foreign marker accepted'}catch{if($_.Exception.Message -notmatch 'explicit preparation refused before drain'){throw}}
''', prepare=True)
                self.assertEqual(path.read_text(), 'foreign')
                path.unlink()
