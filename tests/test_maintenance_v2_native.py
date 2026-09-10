"""Real, harmless child processes exercise the deadline/durable child boundary."""
import unittest
from test_maintenance_v2_engine import MaintenanceV2EngineTests

NATIVE = r'''
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceCommand.ps1')
$s=Read-CdrMaintenanceState $StatePath
$shell=(Get-Command powershell.exe).Source
'''


class MaintenanceV2NativeTests(unittest.TestCase):
    run_case = MaintenanceV2EngineTests.run_case

    def test_successful_native_probe_clears_only_its_command_record(self):
        self.run_case(NATIVE + r'''
Invoke-CdrMaintenanceCommand $s $shell @('-NoProfile','-Command','Write-Output probe_ok; exit 0') 5
if($null -ne (Read-CdrMaintenanceState $StatePath).ActiveCommand){throw 'command not completed'}
''')

    def test_nonzero_exit_is_explicit_not_success_or_silent_fallback(self):
        self.run_case(NATIVE + r'''
try{Invoke-CdrMaintenanceCommand $s $shell @('-NoProfile','-Command','exit 7') 5;throw 'exit 7 accepted'}
catch{if($_.Exception.Message -notmatch 'native_command_failed exit=7'){throw}}
if($null -ne (Read-CdrMaintenanceState $StatePath).ActiveCommand){throw 'definite exit left unknown'}
''')

    def test_expired_wait_preserves_child_and_prohibits_command_replay(self):
        self.run_case(NATIVE + r'''
try{Invoke-CdrMaintenanceCommand $s $shell @('-NoProfile','-Command','Start-Sleep -Seconds 2; exit 0') 1;throw 'wait unbounded'}
catch{if($_.Exception.Message -notmatch 'command_deadline_outcome_unknown'){throw}}
$saved=Read-CdrMaintenanceState $StatePath
if($saved.ActiveCommand.Phase -ne 'child'){throw 'child identity not durable'}
$childPid=[int]$saved.ActiveCommand.Identity.Split('|')[0]
$child=Get-Process -Id $childPid -ErrorAction SilentlyContinue
if($child){[void]$child.WaitForExit(5000)} # Only this harmless fixture exits itself; no kill.
try{Invoke-CdrMaintenanceCommand $saved $shell @('-NoProfile','-Command','exit 0') 1;throw 'command replayed'}
catch{if($_.Exception.Message -notmatch 'command_outcome_unknown'){throw}}
''')

    def test_expired_deadline_prevents_even_native_child_creation(self):
        self.run_case(NATIVE + r'''
$s.Deadline=[DateTimeOffset]::UtcNow.AddSeconds(-1).ToString('o')
try{Invoke-CdrMaintenanceCommand $s $shell @('-NoProfile','-Command','exit 0');throw 'expired child spawned'}
catch{if($_.Exception.Message -notmatch 'deadline_exceeded'){throw}}
if($null -ne (Read-CdrMaintenanceState $StatePath).ActiveCommand){throw 'expired attempt recorded'}
''')

    def test_exit_zero_and_both_readers_ready_just_after_limit_preserves_unknown(self):
        self.run_case(NATIVE + r'''
$script:commandBase=[DateTimeOffset]::UtcNow;$script:observedLate=$false
function Get-CdrCommandUtcNow {
 if($script:observedLate){$script:commandBase.AddSeconds(2)}else{$script:commandBase}
}
function Wait-CdrCommandExit($Process,$OutReader,$ErrReader) {
 if(-not $Process.WaitForExit(5000)){throw 'fixture child hung'}
 [void]$OutReader.GetAwaiter().GetResult();[void]$ErrReader.GetAwaiter().GetResult()
 $script:observedLate=$true
 return $true
}
try{Invoke-CdrMaintenanceCommand $s $shell @('-NoProfile','-Command','exit 0') 1;throw 'late completed child accepted'}
catch{if($_.Exception.Message -notmatch 'command_deadline_outcome_unknown'){throw}}
if((Read-CdrMaintenanceState $StatePath).ActiveCommand.Phase -ne 'child'){throw 'late child record lost'}
if((Read-CdrMaintenanceState $StatePath).Phase -ne 'planned'){throw 'advanced after expired observation'}
''')


if __name__ == '__main__':
    unittest.main()
