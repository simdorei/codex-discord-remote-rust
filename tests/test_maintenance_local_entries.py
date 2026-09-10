import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from maintenance_v2_fixture import ROOT
from test_maintenance_v2_engine import MaintenanceV2EngineTests


class MaintenanceLocalEntryTests(unittest.TestCase):
    run_case = MaintenanceV2EngineTests.run_case

    def test_completed_receipt_without_supported_policy_is_rejected(self):
        for policy in (None, 'unknown', 'live-handshake-v1'):
            with self.subTest(policy=policy), tempfile.TemporaryDirectory() as temp:
                root=Path(temp)
                (root/'scripts').mkdir()
                shutil.copyfile(ROOT/'scripts/Invoke-CdrMaintenance.ps1', root/'scripts/Invoke-CdrMaintenance.ps1')
                shutil.copyfile(ROOT/'codex-discord-rust-control.ps1',root/'codex-discord-rust-control.ps1')
                state=root/'.codex_discord_rust.maintenance.v2'
                receipt=state.with_name(state.name+'.completed')
                payload=dict(Version=2,Operation='a'*32,Phase='verified')
                if policy is not None:
                    payload['ShutdownPolicy']=policy
                receipt.write_text(json.dumps(payload),encoding='utf-8')
                before=receipt.read_bytes()
                result=subprocess.run(['powershell.exe','-NoProfile','-File',str(root/'scripts/Invoke-CdrMaintenance.ps1'),
                    '-StatePath',str(state),'-ExpectedOperation','a'*32],capture_output=True,encoding='utf-8',errors='replace',timeout=15)
                if policy=='live-handshake-v1':
                    self.assertEqual(result.returncode,0,result.stderr)
                    self.assertIn('maintenance_previously_completed',result.stdout)
                else:
                    self.assertNotEqual(result.returncode,0)
                    self.assertIn('shutdown_policy',result.stderr)
                self.assertEqual(receipt.read_bytes(),before)
                self.assertFalse((root/'.codex_discord_rust.stop').exists())

    def test_failure_observation_separates_alive_exited_unknown_and_redacts_code(self):
        for status in ('alive','exited','unknown'):
            with self.subTest(status=status):
                self.run_case(r'''
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceFailure.ps1')
$s=Read-CdrMaintenanceState $StatePath
$s.LastError='graceful_exit_timeout private-details-not-for-discord'
$DrainAckPath=Join-Path $RepoRoot 'ack';$StopPath=Join-Path $RepoRoot 'stop'
function Get-Process {param($Id,$ErrorAction)
 if('STATUS' -eq 'unknown'){throw 'observation denied'}
 if('STATUS' -eq 'alive'){[pscustomobject]@{Id=42}}
}
function Get-RustProcessIdentity {'42|99'}
$observation=Get-CdrMaintenanceFailureObservation $s
if($observation.Original -cne 'STATUS'){throw 'wrong liveness observation'}
if($observation.Code -cne 'graceful_exit_timeout' -or ($observation|ConvertTo-Json) -match 'private-details'){throw 'unsafe error code'}
if(-not $observation.ObservedAt -or $observation.Ack -cne 'missing' -or $observation.Stop -cne 'missing'){throw 'observation incomplete'}
'''.replace('STATUS',status))


if __name__ == '__main__':
    unittest.main()
