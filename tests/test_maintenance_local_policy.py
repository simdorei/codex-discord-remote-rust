"""Same-PC deployment policy; no fabricated installed-artifact certification."""
import unittest
from test_maintenance_v2_engine import MaintenanceV2EngineTests

LOCAL = r'''
$s=Read-CdrMaintenanceState $StatePath
$s|Add-Member ShutdownPolicy 'live-handshake-v1' -Force
Save-CdrMaintenanceState $s $StatePath
function Invoke-CdrMaintenancePreStopBackup {Effect 'backup'}
function Assert-CdrMaintenancePreStopBackup {Effect 'backup_bound'}
'''


class MaintenanceLocalPolicyTests(unittest.TestCase):
    run_case = MaintenanceV2EngineTests.run_case

    def test_live_policy_does_not_require_or_fabricate_old_certificate(self):
        self.run_case(LOCAL + r'''
function Assert-CdrCertifiedBaseline {throw 'obsolete certificate gate invoked'}
Invoke-CdrMaintenanceEngine $StatePath $op
if((Read-CdrMaintenanceState $StatePath).Phase -ne 'verified'){throw 'not completed'}
$backup=$script:calls.IndexOf('planned:backup')
$ack=$script:calls.IndexOf('prepared:ACK')
if($backup -lt 0 -or $ack -le $backup){throw 'backup was not before drain'}
''')

    def test_backup_failure_never_publishes_drain_or_stop(self):
        self.run_case(LOCAL + r'''
$script:fail='backup'
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'backup failure not checked'}
catch{if($_.Exception.Message -ne 'injected_backup'){throw}}
if(@($script:calls|Where-Object{$_ -match ':(ACK|stop|install|launch)$'}).Count){throw 'effect after failed backup'}
''')

    def test_resuming_prepared_rechecks_backup_before_drain(self):
        self.run_case(LOCAL + r'''
$s=Read-CdrMaintenanceState $StatePath;$s.Phase='prepared';Save-CdrMaintenanceState $s $StatePath
$script:fail='backup_bound'
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'missing backup accepted'}
catch{if($_.Exception.Message -ne 'injected_backup_bound'){throw}}
if(@($script:calls|Where-Object{$_ -match ':(ACK|stop|install|launch)$'}).Count){throw 'effect without bound backup'}
''')

    def test_legacy_or_unknown_policy_is_not_silently_adopted(self):
        for value in ('missing', 'unknown'):
            with self.subTest(value=value):
                self.run_case(LOCAL + r'''
$s=Read-CdrMaintenanceState $StatePath
if('VALUE' -eq 'missing'){$s.PSObject.Properties.Remove('ShutdownPolicy')}else{$s.ShutdownPolicy='unknown'}
Write-AtomicRestartMarker $StatePath ($s|ConvertTo-Json -Depth 10)
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'bad policy accepted'}
catch{if($_.Exception.Message -notmatch 'shutdown_policy'){throw}}
if($script:calls.Count){throw 'action before policy check'}
'''.replace('VALUE', value))

    def test_shutdown_failure_cannot_resume_after_late_success(self):
        for action in ('ACK', 'stop', 'bound'):
            with self.subTest(action=action):
                self.run_case(LOCAL + r'''
$script:fail='ACTION'
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'missing fault'}
catch{if($_.Exception.Message -notmatch '^injected_'){throw}}
if(-not (Read-CdrMaintenanceState $StatePath).Halted){throw 'shutdown failure not durable'}
$before=$script:calls.Count
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'late success resumed'}
catch{if($_.Exception.Message -notmatch 'budget_exhausted_or_halted'){throw}}
if($script:calls.Count -ne $before){throw 'late shutdown observed as successful retry'}
if(@($script:calls|Where-Object{$_ -match ':(install|cleanup|launch)$'}).Count){throw 'mutation after shutdown failure'}
'''.replace('ACTION', action))


if __name__ == '__main__':
    unittest.main()
