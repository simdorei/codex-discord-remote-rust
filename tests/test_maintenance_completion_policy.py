"""M01/M05: these assertions must fail against the pre-fix real modules."""
import unittest
from test_maintenance_v2_engine import MaintenanceV2EngineTests
from maintenance_completion_fixture import CONNECTED


class MaintenanceCompletionPolicyTests(unittest.TestCase):
    run_case = MaintenanceV2EngineTests.run_case

    def test_rejected_notice_does_not_leave_healthy_runtime_locked(self):
        self.run_case(CONNECTED + r'''
try {Invoke-CdrMaintenanceEngine $StatePath $op} catch {}
if(Test-Path $DisablePath){throw 'M01: healthy runtime remains disabled by notification failure'}
if(Test-Path $StatePath){throw 'M01: healthy runtime ownership remains active'}
$receipt=Get-Content ($StatePath+'.completed') -Raw | ConvertFrom-Json
if($receipt.Phase -cne 'verified' -or $receipt.CompletionPolicy -cne 'runtime-proof-v1'){throw 'runtime completion missing'}
if($script:starts -ne 1 -or $script:posts -ne 1){throw 'wrong side-effect count'}
''')

    def test_actual_notification_adapter_preserves_safe_http_diagnostics(self):
        self.run_case(CONNECTED + r'''
$caught=$null
try {Send-CdrMaintenanceMessage $s 'synthetic notice' 'fixture-nonce'} catch {$caught=$_}
if(-not $caught){throw 'rejection reported successful'}
if($caught.Exception.Data['HttpStatus'] -ne 403 -or $caught.Exception.Data['DiscordCode'] -ne 50013){throw 'M05: original HTTP/service error discarded'}
if($caught.Exception.Data['Outcome'] -cne 'rejected'){throw 'definite rejection confused with transport uncertainty'}
if($caught.Exception.Message -match 'fixture-secret'){throw 'raw credential-like text leaked'}
''')


if __name__ == '__main__':
    unittest.main()
