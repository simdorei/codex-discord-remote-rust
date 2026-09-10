"""Real legacy recovery must never adopt the v2 operation, even before markers."""
import unittest
from test_maintenance_recovery_safety import MaintenanceRecoverySafetyTests


class MaintenanceV2LegacyTests(unittest.TestCase):
    run_case = MaintenanceRecoverySafetyTests.run_case
    def test_v2_state_cannot_publish_stop_before_ack(self):
        self.run_case(r'''
$s=Get-Content $statePath -Raw|ConvertFrom-Json
$s|Add-Member Version 2
[IO.File]::WriteAllText($statePath,($s|ConvertTo-Json))
[IO.File]::Delete($StopPath)
$script:oldAlive=$true
try{Invoke-CdrDeploymentRecovery $statePath;throw 'v2 adopted'}
catch{if($_.Exception.Message -notmatch 'maintenance_v2'){throw}}
if(Test-Path $StopPath){throw 'stop published without ACK'}
if($script:waits -ne 0 -or $script:starts -ne 0){throw 'legacy process action'}
''')

    def test_v2_owner_blocks_legacy_even_before_disabled(self):
        self.run_case(r'''
[IO.File]::Delete($StopPath);[IO.File]::Delete($DisablePath)
[IO.File]::WriteAllText((Join-Path $RepoRoot '.codex_discord_rust.maintenance.v2'),'owned state')
try{Invoke-CdrDeploymentRecovery $statePath;throw 'pending v2 ignored'}
catch{if($_.Exception.Message -notmatch 'maintenance_v2'){throw}}
if($script:starts -ne 0 -or $script:waits -ne 0){throw 'legacy action'}
''')


if __name__ == '__main__':
    unittest.main()
