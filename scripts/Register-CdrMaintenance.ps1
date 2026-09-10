param(
    [Parameter(Mandatory=$true)][ValidateSet('live-handshake-v1')][string]$ShutdownPolicy,
    [Parameter(Mandatory=$true)][string]$CandidatePath,
    [Parameter(Mandatory=$true)][string]$CandidateHash,
    [Parameter(Mandatory=$true)][string]$OperatorPath,
    [Parameter(Mandatory=$true)][string]$OperatorHash,
    [switch]$ValidateOnly
)
$ErrorActionPreference='Stop'
$RepoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
# Ticket template: replace the example root and notification channel after review.
if ($RepoRoot -cne 'C:\example\codex-discord-remote-rust') { throw 'maintenance_ticket_root_mismatch' }
$BinaryPath = Join-Path $RepoRoot 'target\release\cdr-runtime.exe'
$EnvPath = Join-Path $RepoRoot '.env'
. (Join-Path $RepoRoot 'codex-discord-rust-control.ps1')
. (Join-Path $RepoRoot 'codex-discord-rust-drain.ps1')
. (Join-Path $PSScriptRoot 'CdrMaintenanceState.ps1')
. (Join-Path $PSScriptRoot 'CdrMaintenanceSchedule.ps1')
$control = Enter-CdrControl $RepoRoot
try {
    Assert-CdrNoPendingRestart $RepoRoot
    foreach ($name in @('.codex_discord_bot.disabled','.codex_discord_rust.stop','.codex_discord_runtime.cutover')) {
        if (Test-Path -LiteralPath (Join-Path $RepoRoot $name)) { throw 'maintenance_existing_marker_preserved' }
    }
    $baseline = Get-CdrArtifactHash $BinaryPath
    Assert-CdrMaintenanceShutdownPolicy $ShutdownPolicy
    $CandidatePath = [IO.Path]::GetFullPath($CandidatePath)
    $OperatorPath = [IO.Path]::GetFullPath($OperatorPath)
    if ($CandidatePath -ceq $BinaryPath -or $CandidateHash -ceq $baseline -or
        $CandidateHash -notmatch '^[A-F0-9]{64}$' -or $OperatorHash -notmatch '^[A-F0-9]{64}$' -or
        (Get-CdrArtifactHash $CandidatePath) -cne $CandidateHash -or
        (Get-CdrArtifactHash $OperatorPath) -cne $OperatorHash) { throw 'maintenance_candidate_pins_invalid' }
    $identityPath = Join-Path $RepoRoot '.codex_discord_rust.drain.identity'
    $fields = Read-RestartDrainFields $identityPath @('version','runtime_id','pid','state')
    if (-not $fields -or $fields.state -ne 'open') { throw 'maintenance_runtime_not_open' }
    $process = Get-Process -Id ([int]$fields.pid) -ErrorAction Stop
    if ($process.Path -cne $BinaryPath) { throw 'maintenance_wrong_runtime' }
    $ticks = $process.StartTime.ToUniversalTime().Ticks; $ticks -= ($ticks % 10)
    $identity = "$($process.Id)|$ticks"
    if ($ValidateOnly) { Write-Output 'maintenance_registration_preconditions_ready; nothing armed'; return }
    $operation = [guid]::NewGuid().ToString('N')
    $bundle = Join-Path $RepoRoot ('.codex-discord-backups/maintenance-v2-'+$operation)
    $null = New-Item -ItemType Directory -Path $bundle -ErrorAction Stop
    # Immutable reviewed binaries are copied once; workers verify hashes every entry.
    [IO.File]::Copy($CandidatePath,(Join-Path $bundle 'candidate.exe'),$false)
    [IO.File]::Copy($OperatorPath,(Join-Path $bundle 'operator.exe'),$false)
    $statePath = Get-CdrMaintenancePath $RepoRoot
    $taskName = 'Codex Maintenance V2 '+$operation
    $shellPath = (Get-Command powershell.exe -ErrorAction Stop).Source
    $user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
    $createdAt = [DateTimeOffset]::UtcNow
    $state = [pscustomobject]@{
        Version=2; ShutdownPolicy=$ShutdownPolicy; Operation=$operation; RepoRoot=$RepoRoot; BinaryPath=$BinaryPath
        CompletionPolicy='runtime-proof-v1'
        PreviousCompletedHash=$(if([IO.File]::Exists($statePath+'.completed')){Get-CdrArtifactHash ($statePath+'.completed')}else{''})
        Phase='planned'; Attempts=0; Halted=$false; LastError=''; Bundle=$bundle
        CreatedAt=$createdAt.ToString('o'); Deadline=$createdAt.AddMinutes(30).ToString('o')
        Fence=[pscustomobject]@{RuntimeId=$fields.runtime_id;ProcessIdentity=$identity;Nonce=$operation}
        BaselineHash=$baseline; CandidateHash=$CandidateHash; OperatorHash=$OperatorHash
        EnvHash=(Get-CdrArtifactHash $EnvPath)
        ProgramPins=@(Get-CdrMaintenanceProgramPaths | ForEach-Object {
            [pscustomobject]@{Path=$_;Hash=(Get-CdrArtifactHash (Join-Path $RepoRoot $_))}
        })
        CandidatePath=(Join-Path $bundle 'candidate.exe'); OperatorPath=(Join-Path $bundle 'operator.exe')
        CargoPath=(Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe')
        TaskName=$taskName; TaskUser=$user; PowerShellPath=$shellPath
        NotifyChannel='900000000000000001'; DiscordReceipt=''; Heartbeats=@()
        FailureNoticePhase='none'; FailureReceipt=''; FailureNoticeError=''
        ActiveCommand=$null
        PreStopBackup=$null; PostStopBackup=$null; FailureObservation=$null
    }
    if($state.PreviousCompletedHash){[IO.File]::Copy(($statePath+'.completed'),(Join-Path $bundle 'previous-completion.json'),$false)}
    # Ownership publication comes BEFORE all disabled/prepare markers and scheduling.
    Write-NewCdrMarker $statePath ($state | ConvertTo-Json -Depth 10 -Compress)
    $action = New-ScheduledTaskAction -Execute $shellPath -Argument (Get-CdrMaintenanceTaskArguments $statePath $operation) -WorkingDirectory $RepoRoot
    $trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1) `
        -RepetitionInterval (New-TimeSpan -Minutes 1) -RepetitionDuration (New-TimeSpan -Minutes 30)
    $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
    $settings = New-ScheduledTaskSettingsSet -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Minutes 30)
    $null = Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings
    Assert-CdrMaintenanceRecoveryArmed $state
    Write-Output "maintenance_armed operation=$operation; independent worker begins in one minute"
} finally { $control.Dispose() }
