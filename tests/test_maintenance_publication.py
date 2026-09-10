"""Execute candidate cutover and worker publication boundaries in temporary roots."""
from pathlib import Path
import os
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class MaintenancePublicationTests(unittest.TestCase):
    def run_case(self, body):
        with tempfile.TemporaryDirectory() as temp:
            prefix = r'''
$ErrorActionPreference='Stop'
$RepoRoot=$env:PUBLISH_ROOT
. (Join-Path $env:PUBLISH_SOURCE 'codex-discord-rust-control.ps1')
$source=Get-Content (Join-Path $env:PUBLISH_SOURCE 'codex-discord-runtime-cutover.ps1') -Raw
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseInput($source,[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'parse failed'}
$ast.FindAll({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst]},$false) |
 ForEach-Object {Invoke-Expression $_.Extent.Text}
. (Join-Path $env:PUBLISH_SOURCE 'scripts/CdrCutoverCompletion.ps1')
$DisablePath=Join-Path $RepoRoot '.codex_discord_bot.disabled'
$RustStop=Join-Path $RepoRoot '.codex_discord_rust.stop'
$RustRestart=Join-Path $RepoRoot '.codex_discord_rust.restart'
'''
            result = subprocess.run(
                ['powershell.exe', '-NoProfile', '-Command', prefix + body + '\nexit 0'],
                env={**os.environ, 'PUBLISH_ROOT': temp, 'PUBLISH_SOURCE': str(ROOT)},
                capture_output=True, encoding='utf-8', errors='replace', timeout=15)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_cutover_does_not_delete_foreign_restart(self):
        self.run_case(r'''
[IO.File]::WriteAllText($RustRestart,'foreign fence')
try{Start-Rust;throw 'foreign restart accepted'}
catch{if($_.Exception.Message -notmatch 'Existing restart fence preserved'){throw}}
if([IO.File]::ReadAllText($RustRestart) -cne 'foreign fence'){throw 'foreign fence changed'}
''')

    def test_cutover_does_not_delete_foreign_stop(self):
        self.run_case(r'''
$script:CutoverStopText='owned'
[IO.File]::WriteAllText($RustStop,'foreign')
try{Start-Rust;throw 'foreign stop accepted'}
catch{if($_.Exception.Message -notmatch 'Foreign maintenance marker preserved'){throw}}
if([IO.File]::ReadAllText($RustStop) -cne 'foreign'){throw 'foreign stop changed'}
''')

    def test_cutover_marker_publication_respects_control_owner(self):
        self.run_case(r'''
$owner=Enter-CdrControl $RepoRoot
try {
 try {Enter-CutoverMaintenance ([pscustomobject]@{TransactionId='mine'});throw 'control lock bypassed'}
 catch{if($_.Exception.Message -notmatch 'cdr_control_busy'){throw}}
 if(Test-Path $DisablePath){throw 'published despite competing owner'}
}finally{$owner.Dispose()}
''')

    def test_worker_rechecks_runtime_lock_before_stop_publication(self):
        self.run_case(r'''
$source=Get-Content (Join-Path $env:PUBLISH_SOURCE 'scripts/Invoke-CdrDeployment.ps1') -Raw
$start=$source.IndexOf('    $control = Enter-CdrControl')
$end=$source.IndexOf("    Note 'normal_stop_requested'",$start)
if($start -lt 0 -or $end -lt 0){throw 'publication boundary missing'}
$state=[pscustomobject]@{RepoRoot=$RepoRoot;RuntimePid=42;RuntimeTicks='99';BinaryPath='fixture'}
$running=[pscustomobject]@{HasExited=$false}
$running|Add-Member ScriptMethod Refresh {}
function Get-Process {param($Id,$ErrorAction) [pscustomobject]@{Path='fixture';StartTime=[datetime]::new(99,[DateTimeKind]::Utc)}}
[IO.File]::WriteAllText((Join-Path $RepoRoot '.codex_discord_rust.runtime.lock'),"pid=77`n")
try{& ([scriptblock]::Create($source.Substring($start,$end-$start)));throw 'changed lock accepted'}
catch{if($_.Exception.Message -notmatch 'Runtime changed before stop publication'){throw}}
if((Test-Path $RustStop) -or (Test-Path $DisablePath)){throw 'new instance was stopped'}
''')

    def test_cutover_stop_consumes_its_marker_after_confirmed_exit(self):
        self.run_case(r'''
$CutoverIdentity='fixture-owner'
$script:checks=0
function Get-VerifiedRustProcess {
 $script:checks++; if($script:checks -eq 1){[pscustomobject]@{Id=42}}else{$null}
}
$RustWatchdog=Join-Path $RepoRoot 'fixture-watchdog.ps1'
[IO.File]::WriteAllText($RustWatchdog,'exit 0')
Stop-Rust
if(Test-Path $RustStop){throw 'owned stop leaked into the next cutover invocation'}
# A new PowerShell call cannot recover script-local owner text. Verify startup
# no longer depends on it once this stop operation has completed.
$script:CutoverStopText=$null
if(Test-Path $RustStop){throw 'next invocation would be blocked'}
''')


if __name__ == '__main__':
    unittest.main()
