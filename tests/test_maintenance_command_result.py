"""Capture definite native success for a durable backup receipt, never late output."""
import unittest
from test_maintenance_v2_engine import MaintenanceV2EngineTests
from test_maintenance_v2_native import NATIVE


class MaintenanceCommandResultTests(unittest.TestCase):
    run_case = MaintenanceV2EngineTests.run_case

    def test_opt_in_result_is_returned_only_after_definite_success(self):
        self.run_case(NATIVE + r'''
$result=Invoke-CdrMaintenanceCommand $s $shell @('-NoProfile','-Command','Write-Output probe_ok; exit 0') 5 -PassThru
if($null -eq $result -or $result.ExitCode -ne 0 -or $result.Stdout.Trim() -cne 'probe_ok'){throw 'definite output unavailable'}
if($null -ne (Read-CdrMaintenanceState $StatePath).ActiveCommand){throw 'result before command committed'}
''')

    def test_nonzero_exit_never_returns_a_backup_result(self):
        self.run_case(NATIVE + r'''
$result=$null
try{$result=Invoke-CdrMaintenanceCommand $s $shell @('-NoProfile','-Command','Write-Output probe_ok; exit 7') 5 -PassThru;throw 'failure accepted'}
catch{if($_.Exception.Message -notmatch 'native_command_failed exit=7'){throw}}
if($null -ne $result){throw 'failed command result returned'}
''')


if __name__ == '__main__':
    unittest.main()
