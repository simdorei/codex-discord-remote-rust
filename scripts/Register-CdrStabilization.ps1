param(
    [Parameter(Mandatory=$true)][ValidateSet('live-handshake-v1')][string]$ShutdownPolicy,
    [Parameter(Mandatory=$true)][string]$CandidatePath,
    [Parameter(Mandatory=$true)][string]$CandidateHash,
    [Parameter(Mandatory=$true)][string]$ExpectedRoot,
    [Parameter(Mandatory=$true)][string]$ExpectedBaselineHash,
    [Parameter(Mandatory=$true)][string]$ExpectedEnvironmentHash,
    [Parameter(Mandatory=$true)][string]$CapabilityPath,
    [Parameter(Mandatory=$true)][string]$CapabilityHash,
    [Parameter(Mandatory=$true)][string]$LaunchManifestPath,
    [Parameter(Mandatory=$true)][string]$LaunchManifestHash,
    [Parameter(Mandatory=$true)][string]$NotifyChannel,
    [switch]$ValidateOnly
)
$ErrorActionPreference='Stop'
$RepoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
# This separately selected policy has no disposal/release/retirement action.
if ($RepoRoot -cne [IO.Path]::GetFullPath($ExpectedRoot)) { throw 'maintenance_ticket_root_mismatch' }
$OperatorPath=$CandidatePath; $OperatorHash=$CandidateHash
if ($NotifyChannel -cnotmatch '^\d{17,20}$') { throw 'maintenance_notification_target_invalid' }
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
    if ($baseline -cne $ExpectedBaselineHash -or (Get-CdrArtifactHash $EnvPath) -cne $ExpectedEnvironmentHash) {
        throw 'maintenance_operating_baseline_or_environment_changed'
    }
    foreach ($pair in @(@($CapabilityPath,$CapabilityHash),@($LaunchManifestPath,$LaunchManifestHash))) {
        if ($pair[1] -cnotmatch '^[A-F0-9]{64}$' -or (Get-CdrArtifactHash $pair[0]) -cne $pair[1]) {
            throw 'maintenance_compatibility_input_pin_invalid'
        }
    }
    foreach ($name in @('.codex_discord_rust.compatibility.required','.codex_discord_rust.compatibility.json')) {
        if (Test-Path -LiteralPath (Join-Path $RepoRoot $name)) { throw 'maintenance_existing_compatibility_intent_preserved' }
    }
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
    [IO.File]::Copy($CapabilityPath,(Join-Path $bundle 'capabilities.json'),$false)
    [IO.File]::Copy($LaunchManifestPath,(Join-Path $bundle 'launch-artifacts.json'),$false)
    $statePath = Get-CdrMaintenancePath $RepoRoot
    $taskName = 'Codex Maintenance V2 '+$operation
    $shellPath = (Get-Command powershell.exe -ErrorAction Stop).Source
    $user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
    $createdAt = [DateTimeOffset]::UtcNow
    $state = [pscustomobject]@{
        Version=2; ShutdownPolicy=$ShutdownPolicy; Operation=$operation; RepoRoot=$RepoRoot; BinaryPath=$BinaryPath
        CompletionPolicy='runtime-proof-v1'; DeploymentPolicy='stabilization-held-v1'
        Compatibility=[pscustomobject]@{
            DatabasePath=(Join-Path $RepoRoot 'discord_mirror.sqlite')
            CapabilityPath=(Join-Path $bundle 'capabilities.json'); CapabilityHash=$CapabilityHash
            LaunchManifestPath=(Join-Path $bundle 'launch-artifacts.json'); LaunchManifestHash=$LaunchManifestHash
            IncidentThread='01a06156-56cd-70b0-af02-2de7445ba4c7'
            OriginalJob='b3d5a1a3-5c3e-4764-967b-0cef767efde9'
            OriginalDisposition='held-no-replay-no-disposal-no-release'
        }
        PreviousCompletedHash=$(if([IO.File]::Exists($statePath+'.completed')){Get-CdrArtifactHash ($statePath+'.completed')}else{''})
        Phase='planned'; Attempts=0; Halted=$false; LastError=''; Bundle=$bundle
        CreatedAt=$createdAt.ToString('o'); Deadline=$createdAt.AddMinutes(30).ToString('o')
        Fence=[pscustomobject]@{RuntimeId=$fields.runtime_id;ProcessIdentity=$identity;Nonce=$operation}
        BaselineHash=$baseline; CandidateHash=$CandidateHash; OperatorHash=$OperatorHash
        EnvHash=(Get-CdrArtifactHash $EnvPath)
        ProgramPins=@(Get-CdrMaintenanceProgramPaths 'stabilization-held-v1' | ForEach-Object {
            [pscustomobject]@{Path=$_;Hash=(Get-CdrArtifactHash (Join-Path $RepoRoot $_))}
        })
        CandidatePath=(Join-Path $bundle 'candidate.exe'); OperatorPath=(Join-Path $bundle 'operator.exe')
        CargoPath=(Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe')
        TaskName=$taskName; TaskUser=$user; PowerShellPath=$shellPath
        NotifyChannel=$NotifyChannel; DiscordReceipt=''; Heartbeats=@()
        FailureNoticePhase='none'; FailureReceipt=''; FailureNoticeError=''
        ActiveCommand=$null
        PreStopBackup=$null; PostStopBackup=$null; FailureObservation=$null
    }
    Assert-CdrStabilizationState $state
    if($state.NotifyChannel -cne '1543277263418826775'){throw 'maintenance_notification_target_not_reviewed'}
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
