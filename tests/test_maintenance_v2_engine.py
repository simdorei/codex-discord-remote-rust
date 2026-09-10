"""Real durable state engine; only OS/HTTP/probe boundaries are injected."""
import os
import subprocess
import tempfile
import unittest
from maintenance_v2_fixture import ROOT, LOAD


class MaintenanceV2EngineTests(unittest.TestCase):
    def run_case(self, body):
        with tempfile.TemporaryDirectory() as temp:
            result = subprocess.run(['powershell.exe', '-NoProfile', '-Command', LOAD + body + '\nexit 0'],
                env={**os.environ, 'V2_ROOT': temp, 'V2_SOURCE': str(ROOT)},
                capture_output=True, encoding='utf-8', errors='replace', timeout=25)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_normal_order_and_durable_mutation_then_single_launch(self):
        self.run_case(r'''
Invoke-CdrMaintenanceEngine $StatePath $op
$s=Read-CdrMaintenanceState $StatePath
if($s.Phase -ne 'verified' -or $s.DiscordReceipt -ne '12345'){throw 'not verified'}
$expected=@('prepared:ACK','stop_requested:stop','installing:install','mutation_started:cleanup','launch_ready:launch','launched:heartbeats','notifying:notify')
$previous=-1
foreach($entry in $expected){$index=$script:calls.IndexOf($entry);if($index -le $previous){throw "wrong order $entry"};$previous=$index}
Invoke-CdrMaintenanceEngine $StatePath $op
if(@($script:calls|Where-Object{$_ -like '*:launch'}).Count -ne 1){throw 'relaunch on completion'}
''')

    def test_each_prelaunch_interruption_resumes_at_recorded_phase(self):
        # Changed policy: only pre-shutdown or definite post-stop failures may resume.
        for action in ['armed', 'preflight', 'backup', 'package', 'install', 'cleanup', 'full_readiness']:
            with self.subTest(action=action):
                self.run_case(r'''
$script:fail='ACTION'
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'fault not injected'}catch{if($_.Exception.Message -notmatch '^injected_'){throw}}
if(@($script:calls|Where-Object{$_ -like '*:launch'}).Count){throw 'launch before failed precondition'}
Invoke-CdrMaintenanceEngine $StatePath $op
if((Read-CdrMaintenanceState $StatePath).Phase -ne 'verified'){throw 'did not resume'}
if(@($script:calls|Where-Object{$_ -like '*:launch'}).Count -ne 1){throw 'wrong launch count'}
'''.replace('ACTION', action))

    def test_unknown_notification_does_not_replay_launch_cleanup_or_post(self):
        self.run_case(r'''
$script:fail='notify'
try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'unknown accepted'}catch{if($_.Exception.Message -notmatch 'notification_outcome_unknown'){throw}}
foreach($action in @('launch','cleanup','notify')){
 if(@($script:calls|Where-Object{$_ -like "*:$action"}).Count -ne 1){throw "replayed $action"}
}
if(-not (Read-CdrMaintenanceState $StatePath).Halted){throw 'unknown not halted'}
''')

    def test_budget_is_persistent_across_reloaded_attempts(self):
        self.run_case(r'''
function Invoke-CdrMaintenancePreflight {Effect 'preflight';throw 'retryable_preflight_failure'}
1..3|ForEach-Object{try{Invoke-CdrMaintenanceEngine $StatePath $op}catch{}}
$before=$script:calls.Count
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'budget reset'}catch{if($_.Exception.Message -notmatch 'budget_exhausted'){throw}}
$s=Read-CdrMaintenanceState $StatePath
if($s.Attempts -ne 3 -or -not $s.Halted -or $script:calls.Count -ne $before){throw 'budget not durable'}
if(@($script:calls|Where-Object{$_ -like '*:stop'}).Count){throw 'stop before ACK'}
''')

    def test_expired_operation_does_not_run_any_action(self):
        self.run_case(r'''
$s=Read-CdrMaintenanceState $StatePath
$s.CreatedAt=$now.AddMinutes(-40).ToString('o');$s.Deadline=$now.AddMinutes(-10).ToString('o')
Save-CdrMaintenanceState $s $StatePath
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'expired accepted'}catch{if($_.Exception.Message -notmatch 'budget_exhausted'){throw}}
if($script:calls.Count){throw 'expired action'}
''')

    def test_changed_state_owner_is_preserved(self):
        self.run_case(r'''
$s=Read-CdrMaintenanceState $StatePath
$changed=Read-CdrMaintenanceState $StatePath;$changed.Operation='f'*32
Write-AtomicRestartMarker $StatePath ($changed|ConvertTo-Json -Depth 10)
try{Save-CdrMaintenanceState $s $StatePath;throw 'owner replaced'}catch{if($_.Exception.Message -notmatch 'owner_changed'){throw}}
if(([IO.File]::ReadAllText($StatePath)|ConvertFrom-Json).Operation -cne ('f'*32)){throw 'foreign overwritten'}
''')

    def test_actual_catalog_does_not_accept_uncertified_installed_hash(self):
        self.run_case(r'''
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceState.ps1')
$RepoRoot=$env:V2_SOURCE
try{Assert-CdrCertifiedBaseline '75F713070CA47D014A99E3FF2A73B7D336E2B7529B4A28064E6393FE8EB08E8A';throw 'unproven accepted'}
catch{if($_.Exception.Message -notmatch 'T1_installed_drain_contract_unproven'){throw}}
''')

    def test_old_scheduler_operation_cannot_consume_new_state_budget(self):
        self.run_case(r'''
$before=[IO.File]::ReadAllBytes($StatePath)
try{Invoke-CdrMaintenanceEngine -StatePath $StatePath -ExpectedOperation ('f'*32);throw 'old scheduler adopted new operation'}
catch{if($_.Exception.Message -notmatch 'expected_operation_mismatch'){throw}}
if([Convert]::ToBase64String($before) -cne [Convert]::ToBase64String([IO.File]::ReadAllBytes($StatePath))){throw 'new state changed'}
if($script:calls.Count){throw 'old operation caused action'}
''')


if __name__ == '__main__':
    unittest.main()
