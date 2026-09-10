# Same-PC backup/package evidence. No bot-off checkpoint claim or automatic restore.
function Get-CdrMaintenanceBackupHash([string]$Path) {
    $item=Get-Item -LiteralPath $Path -ErrorAction Stop
    if ($item.PSIsContainer -or $item.Length -eq 0 -or
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'maintenance_backup_not_regular_file' }
    return Get-CdrArtifactHash $Path
}

function Assert-CdrMaintenanceBackupReceipt($State, [string]$Name) {
    if ($Name -notin @('PreStopBackup','PostStopBackup')) { throw 'maintenance_backup_stage_invalid' }
    $r=$State.$Name
    if (-not $r -or $r.Operation -cne $State.Operation -or $r.Policy -cne $State.ShutdownPolicy -or
        $r.Stage -cne $Name -or $r.BaselineHash -cne $State.BaselineHash -or
        $r.CandidateHash -cne $State.CandidateHash -or
        $r.SourceDb -cne (Join-Path $RepoRoot 'discord_mirror.sqlite') -or $r.Verified -ne $true) {
        throw 'maintenance_backup_receipt_not_bound'
    }
    $directory=Join-Path $RepoRoot '.codex-discord-backups'
    $path=[IO.Path]::GetFullPath([string]$r.SnapshotPath)
    if ([IO.Path]::GetDirectoryName($path) -cne $directory -or
        [IO.Path]::GetFileName($path) -cnotmatch '^discord_mirror\.v\d+-cutover\.\d{8}T\d{6}Z\.[a-f0-9]{12}\.sqlite$' -or
        ((Get-Item -LiteralPath $directory).Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        $r.SnapshotHash -notmatch '^[A-F0-9]{64}$' -or
        (Get-CdrMaintenanceBackupHash $path) -cne $r.SnapshotHash -or
        (Get-CdrMaintenanceBackupHash (Join-Path $State.Bundle 'baseline.exe')) -cne $State.BaselineHash) {
        throw 'maintenance_backup_file_changed_or_wrong_path'
    }
}

function Assert-CdrMaintenancePreStopBackup($State) {
    Assert-CdrMaintenanceBackupReceipt $State 'PreStopBackup'
    if ($State.PreStopBackup.PackageVerified -ne $true) { throw 'maintenance_package_not_verified' }
}

function New-CdrMaintenanceSnapshotReceipt($State, [string]$StatePath, [string]$Name) {
    if ($null -ne $State.$Name) { Assert-CdrMaintenanceBackupReceipt $State $Name; return }
    Assert-CdrMaintenanceArtifacts $State
    Assert-CdrMaintenanceDeadline $State
    # The pinned candidate opens the source read-only and validates snapshot integrity/version.
    $result=Invoke-CdrMaintenanceCommand $State $State.CandidatePath @('--backup-store','--env',$EnvPath) 120 -PassThru
    if ($null -eq $result -or $result.ExitCode -ne 0 -or
        $result.Stdout.Trim() -cnotmatch '^backup_created path=([^\r\n]+)$') {
        throw 'maintenance_backup_result_invalid'
    }
    $snapshot=$Matches[1]
    if ($Name -eq 'PostStopBackup' -and $snapshot -ceq $State.PreStopBackup.SnapshotPath) {
        throw 'maintenance_post_stop_snapshot_not_new'
    }
    $receipt=[pscustomobject]@{
        Operation=$State.Operation; Policy=$State.ShutdownPolicy; Stage=$Name
        BaselineHash=$State.BaselineHash; CandidateHash=$State.CandidateHash
        SourceDb=(Join-Path $RepoRoot 'discord_mirror.sqlite')
        SnapshotPath=$snapshot; SnapshotHash=(Get-CdrMaintenanceBackupHash $snapshot)
        Verified=$true; PackageVerified=$false; ObservedAt=[DateTimeOffset]::UtcNow.ToString('o')
    }
    $State.$Name=$receipt
    Assert-CdrMaintenanceBackupReceipt $State $Name
    Assert-CdrMaintenanceDeadline $State
    Save-CdrMaintenanceState $State $StatePath
}

function Invoke-CdrMaintenancePreStopBackup($State, [string]$StatePath) {
    Assert-CdrMaintenanceArtifacts $State
    Assert-CdrMaintenanceDeadline $State
    $backup=Join-Path $State.Bundle 'baseline.exe'
    if (-not [IO.File]::Exists($backup)) { [IO.File]::Copy($BinaryPath,$backup,$false) }
    if ((Get-CdrMaintenanceBackupHash $backup) -cne $State.BaselineHash) { throw 'maintenance_baseline_backup_changed' }
    New-CdrMaintenanceSnapshotReceipt $State $StatePath 'PreStopBackup'
    # Local package checks are executable/hash/config checks, not bot-off archive certification.
    foreach ($path in @($State.CandidatePath,$State.OperatorPath)) {
        $stream=[IO.File]::OpenRead($path)
        try { if ($stream.ReadByte() -ne 77 -or $stream.ReadByte() -ne 90) { throw 'maintenance_package_not_PE' } }
        finally { $stream.Dispose() }
    }
    $result=Invoke-CdrMaintenanceCommand $State $State.CandidatePath @('--check-config','--env',$EnvPath) 30 -PassThru
    if ($null -eq $result -or $result.ExitCode -ne 0 -or $result.Stdout -cnotmatch '^config_valid ') {
        throw 'maintenance_package_config_not_verified'
    }
    Assert-CdrMaintenanceArtifacts $State
    Assert-CdrMaintenanceDeadline $State
    $State.PreStopBackup.PackageVerified=$true
    Save-CdrMaintenanceState $State $StatePath
    Assert-CdrMaintenancePreStopBackup $State
}
