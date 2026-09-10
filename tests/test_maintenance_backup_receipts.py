"""Actual receipt/backup module; external executable responses are synthetic."""
import unittest
from test_maintenance_v2_engine import MaintenanceV2EngineTests

BACKUP = r'''
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceBackup.ps1')
$s=Read-CdrMaintenanceState $StatePath
$EnvPath=Join-Path $RepoRoot '.env'
[IO.File]::WriteAllText($BinaryPath,'MZbaseline')
[IO.File]::WriteAllText($s.CandidatePath,'MZcandidate')
[IO.File]::WriteAllText($s.OperatorPath,'MZoperator')
$s.BaselineHash=Get-CdrArtifactHash $BinaryPath
$s.CandidateHash=Get-CdrArtifactHash $s.CandidatePath
Save-CdrMaintenanceState $s $StatePath
$snapshotDir=Join-Path $RepoRoot '.codex-discord-backups'
# LOAD's Directory.CreateDirectory(bundle) recursively creates this parent too.
if(-not [IO.Directory]::Exists($snapshotDir)){throw 'fixture parent missing after recursive bundle initialization'}
$snapshot=Join-Path $snapshotDir 'discord_mirror.v3-cutover.20260909T000000Z.aaaaaaaaaaaa.sqlite'
$script:nativeFailure=''
function Invoke-CdrMaintenanceCommand {
 param($State,$File,$Arguments,$LimitSeconds,[switch]$PassThru)
 if($Arguments[0] -eq $script:nativeFailure){throw 'injected_native_failure'}
 if($Arguments[0] -eq '--backup-store'){
  [IO.File]::WriteAllText($snapshot,'synthetic snapshot bytes; SQLite verified in separate integration test')
  return [pscustomobject]@{ExitCode=0;Stdout="backup_created path=$snapshot";Stderr=''}
 }
 return [pscustomobject]@{ExitCode=0;Stdout='config_valid token=[REDACTED]';Stderr=''}
}
'''


class MaintenanceBackupReceiptTests(unittest.TestCase):
    run_case = MaintenanceV2EngineTests.run_case

    def test_receipt_is_bound_and_reloaded_from_disk(self):
        self.run_case(BACKUP + r'''
Invoke-CdrMaintenancePreStopBackup $s $StatePath
$saved=Read-CdrMaintenanceState $StatePath
Assert-CdrMaintenancePreStopBackup $saved
if($saved.PreStopBackup.Operation -cne $op -or -not $saved.PreStopBackup.PackageVerified){throw 'receipt not committed'}
''')

    def test_changed_deleted_foreign_or_wrong_source_receipt_rejected(self):
        for fault in ('changed', 'deleted', 'foreign', 'source'):
            with self.subTest(fault=fault):
                self.run_case(BACKUP + r'''
Invoke-CdrMaintenancePreStopBackup $s $StatePath
switch('FAULT'){
 'changed'{[IO.File]::WriteAllText($snapshot,'corrupted')}
 'deleted'{[IO.File]::Delete($snapshot)}
 'foreign'{$s.PreStopBackup.Operation='f'*32}
 'source'{$s.PreStopBackup.SourceDb=Join-Path $RepoRoot 'wrong.sqlite'}
}
$rejected=$false
try{Assert-CdrMaintenancePreStopBackup $s}catch{$rejected=$true}
if(-not $rejected){throw 'bad backup accepted'}
'''.replace('FAULT', fault))

    def test_native_snapshot_or_package_failure_never_certifies_receipt(self):
        for fault in ('--backup-store', '--check-config'):
            with self.subTest(fault=fault):
                self.run_case(BACKUP + r'''
$script:nativeFailure='FAULT'
try{Invoke-CdrMaintenancePreStopBackup $s $StatePath;throw 'fault not checked'}
catch{if($_.Exception.Message -ne 'injected_native_failure'){throw}}
if((Read-CdrMaintenanceState $StatePath).PreStopBackup.PackageVerified -eq $true){throw 'failed package certified'}
'''.replace('FAULT', fault))

    def test_post_stop_cannot_reuse_pre_stop_snapshot(self):
        self.run_case(BACKUP + r'''
Invoke-CdrMaintenancePreStopBackup $s $StatePath
try{New-CdrMaintenanceSnapshotReceipt $s $StatePath 'PostStopBackup';throw 'stale snapshot accepted'}
catch{if($_.Exception.Message -ne 'maintenance_post_stop_snapshot_not_new'){throw}}
if($null -ne (Read-CdrMaintenanceState $StatePath).PostStopBackup){throw 'stale receipt committed'}
''')


if __name__ == '__main__':
    unittest.main()
