"""Task Scheduler canonicalizes local user names; compare actual Windows identities."""
import unittest
from test_maintenance_v2_engine import MaintenanceV2EngineTests


class MaintenanceScheduleIdentityTests(unittest.TestCase):
    run_case = MaintenanceV2EngineTests.run_case

    def test_same_user_aliases_pass_but_other_user_is_rejected(self):
        for form in ('full', 'short', 'sid', 'foreign'):
            with self.subTest(form=form):
                self.run_case(r'''
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceSchedule.ps1')
$s=Read-CdrMaintenanceState $StatePath
$identity=[Security.Principal.WindowsIdentity]::GetCurrent()
$s|Add-Member TaskUser $identity.Name
$s|Add-Member PowerShellPath (Get-Command powershell.exe).Source
$actual=switch('FORM'){
 'full'{$identity.Name}
 'short'{$identity.Name.Split('\')[-1]}
 'sid'{$identity.User.Value}
 'foreign'{'S-1-5-18'}
}
if('FORM' -eq 'foreign' -and $identity.User.Value -eq $actual){throw 'fixture unexpectedly runs as LocalSystem'}
function Get-ScheduledTask {param($TaskName,$ErrorAction)
 [pscustomobject]@{
  Settings=[pscustomobject]@{Enabled=$true}
  Actions=@([pscustomobject]@{Execute=$s.PowerShellPath;Arguments=(Get-CdrMaintenanceTaskArguments $StatePath $s.Operation);WorkingDirectory=$RepoRoot})
  Principal=[pscustomobject]@{UserId=$actual}
  Triggers=@([pscustomobject]@{Enabled=$true;Repetition=[pscustomobject]@{Interval='PT1M';Duration='PT30M'}})
 }
}
if('FORM' -eq 'foreign'){
 try{Assert-CdrMaintenanceRecoveryArmed $s;throw 'foreign task owner accepted'}
 catch{if($_.Exception.Message -notmatch 'independent_recovery_not_armed'){throw}}
}else{Assert-CdrMaintenanceRecoveryArmed $s}
'''.replace('FORM',form))


if __name__ == '__main__':
    unittest.main()
