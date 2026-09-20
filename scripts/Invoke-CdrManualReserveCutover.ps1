[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$StatePath,[switch]$Monitor,
    [string]$RecoverFromWorkerIdentity,[string]$RecoveryOwner,[int]$RecoveryCount=0)
$ErrorActionPreference='Stop'
function Read-CdrCutoverStateFile {
    param([string]$Path,[switch]$AsText)
    $deadline=[DateTimeOffset]::UtcNow.AddSeconds(2)
    while($true) {
        $reader=$null
        try {
            $stream=[IO.File]::Open($Path,'Open','Read',([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
            $reader=[IO.StreamReader]::new($stream)
            $text=$reader.ReadToEnd()
            if ($AsText) { return $text }
            return ($text | ConvertFrom-Json)
        } catch [IO.IOException] {
            if([DateTimeOffset]::UtcNow -ge $deadline){throw}
        } finally {if($null -ne $reader){$reader.Dispose()}}
        Start-Sleep -Milliseconds 50
    }
}
$script:CutoverStatePath=[IO.Path]::GetFullPath($StatePath)
$script:CutoverBundle=[IO.Path]::GetDirectoryName($script:CutoverStatePath)
$script:CutoverState=Read-CdrCutoverStateFile $script:CutoverStatePath
$RepoRoot=[IO.Path]::GetFullPath($script:CutoverState.RepoRoot)
$requestPath=Join-Path $script:CutoverBundle 'request.json'
$hash=(Get-FileHash -LiteralPath $requestPath -Algorithm SHA256).Hash
if ($hash -cne $script:CutoverState.RequestHash) { throw 'Cutover request changed.' }
$request=[IO.File]::ReadAllText($requestPath) | ConvertFrom-Json
if ($script:CutoverState.Version -ne 1 -or $request.Version -ne 1 -or
    $script:CutoverState.Operation -notmatch '^[a-f0-9]{32}$' -or
    [IO.Path]::GetDirectoryName($script:CutoverBundle) -ine (Join-Path $RepoRoot 'maintenance_backups\manual-reserve') -or
    [IO.Path]::GetFileName($script:CutoverBundle) -cne $script:CutoverState.Operation) {
    throw 'Cutover state is outside its operation namespace.'
}
foreach ($field in @('Operation','RepoRoot','BaselineHash','CandidateHash','TargetIdentity')) {
    if ($script:CutoverState.$field -cne $request.$field) { throw 'Immutable cutover identity changed.' }
}
$supports=@('codex-discord-rust-watchdog.ps1','codex-discord-rust-control.ps1','codex-discord-rust-drain.ps1',
    'scripts/CdrLaunchJournal.ps1','scripts/CdrForceRestart.ps1','scripts/CdrManualReserveCutover.ps1',
    'scripts/Invoke-CdrManualReserveCutover.ps1')
if (@($request.ProgramPins).Count -ne $supports.Count) { throw 'Incomplete cutover program pins.' }
foreach ($source in $supports) {
    $pins=@($request.ProgramPins | Where-Object { $_.Path -ceq $source })
    if ($pins.Count -ne 1 -or (Get-FileHash -LiteralPath (Join-Path $RepoRoot $source) -Algorithm SHA256).Hash -cne $pins[0].Hash) {
        throw 'Reviewed cutover source changed.'
    }
}
$BinaryPath=Join-Path $RepoRoot 'target\release\cdr-runtime.exe'
$CandidatePath=Join-Path $RepoRoot 'target\force-bang\release\cdr-runtime.exe'
$EnvPath=Join-Path $RepoRoot '.env'
foreach ($pair in @{
    LockPath='.codex_discord_rust.runtime.lock';HeartbeatPath='.codex_discord_rust.heartbeat'
    RestartPath='.codex_discord_rust.restart';DrainIdentityPath='.codex_discord_rust.drain.identity'
    DrainPreparePath='.codex_discord_rust.drain.prepare';DrainAckPath='.codex_discord_rust.drain.ack'
    StopPath='.codex_discord_rust.stop';DisablePath='.codex_discord_bot.disabled'
    LauncherLog='discord_launcher.log';StdoutLog='codex_discord_rust.log';StderrLog='codex_discord_rust.error.log'
}.GetEnumerator()) { Set-Variable -Name $pair.Key -Value (Join-Path $RepoRoot $pair.Value) }
$HealthHeartbeatMaxAgeSeconds=45;$HealthHeartbeatStartupGraceSeconds=120
$script:CutoverSeal='manual-reserve:'+$script:CutoverState.Operation+':'+$script:CutoverState.RequestHash
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $RepoRoot $supports[0]),[ref]$tokens,[ref]$errors)
if ($errors) { throw 'Watchdog parse error.' }
foreach ($definition in $ast.FindAll({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst]},$false)) {
    Invoke-Expression $definition.Extent.Text
}
foreach ($source in $supports[1..5]) { . (Join-Path $RepoRoot $source) }
Set-Location -LiteralPath $RepoRoot

function Read-CdrLatestCutoverState {
    $latest=Read-CdrCutoverStateFile $StatePath
    if ($latest.Version -ne 1 -or $latest.RequestHash -cne $hash) { throw 'Cutover state/request identity changed.' }
    foreach ($field in @('Operation','RepoRoot','BaselineHash','CandidateHash','TargetIdentity')) {
        if ($latest.$field -cne $request.$field) { throw 'Immutable cutover identity changed.' }
    }
    return $latest
}
$crashRecovery=[bool]$RecoverFromWorkerIdentity
if ($crashRecovery -ne [bool]$RecoveryOwner -or
    ($crashRecovery -and ($RecoveryCount -lt 1 -or $Monitor)) -or
    (-not $crashRecovery -and $RecoveryCount -ne 0)) { throw 'Incomplete crash-recovery claim.' }

if ($Monitor) {
    $monitorGuard=[IO.File]::Open((Join-Path $script:CutoverBundle 'monitor.lock'),'OpenOrCreate','ReadWrite','None')
    $watched=$script:CutoverState.WorkerIdentity
    $monitorIdentity=Get-RustProcessIdentity ([Diagnostics.Process]::GetCurrentProcess())
    $claimedRecovery=0
    try {
        Write-AtomicRestartMarker (Join-Path $script:CutoverBundle 'monitor.ready') $monitorIdentity
        while ($true) {
            $state=Read-CdrCutoverStateFile $StatePath
            if ((Test-CdrCutoverSettled $state) -or $state.LastError -or $state.WorkerIdentity -cne $watched) { exit 0 }
            $owner=Get-CdrPinnedProcess ([pscustomobject]@{Identity=$watched;Path=(Join-Path $PSHOME 'powershell.exe')})
            if ($null -eq $owner) { break }
            $owner.Dispose()
            Start-Sleep -Seconds 1
        }
        # The polling snapshot predates confirmed exit. Claim recovery from the
        # latest state under the same force -> control order as every worker.
        $recoveryForce=$null;$recoveryControl=$null
        try {
            $recoveryForce=Enter-CdrForceGate $RepoRoot
            $recoveryControl=Enter-CdrForceControl $RepoRoot
            $state=Read-CdrLatestCutoverState
            if ((Test-CdrCutoverSettled $state) -or $state.LastError -or $state.WorkerIdentity -cne $watched) { exit 0 }
            $owner=Get-CdrPinnedProcess ([pscustomobject]@{Identity=$watched;Path=(Join-Path $PSHOME 'powershell.exe')})
            if ($null -ne $owner) { $owner.Dispose(); exit 0 }
            if ([int]$state.Recoveries -ge 3) { throw 'Recovery crash budget exhausted; owned seal remains.' }
            $state.Recoveries=[int]$state.Recoveries+1
            $state.RecoveryIdentity=$monitorIdentity
            Write-AtomicRestartMarker $StatePath ($state | ConvertTo-Json -Depth 12 -Compress)
            $claimedRecovery=[int]$state.Recoveries
        } finally {
            if ($null -ne $recoveryControl) { $recoveryControl.Dispose() }
            if ($null -ne $recoveryForce) { $recoveryForce.Dispose() }
        }
    } finally { $monitorGuard.Dispose() }
    # Release non-reentrant locks before entry, which must revalidate this exact
    # conditional claim after acquiring them again. No stale-state authority.
    & $PSCommandPath -StatePath $StatePath -RecoverFromWorkerIdentity $watched -RecoveryOwner $monitorIdentity -RecoveryCount $claimedRecovery
    exit $LASTEXITCODE
}

$force=$null;$control=$null;$ownsWorkerState=$false
try {
    if (Test-CdrCallerInsideRuntime $script:CutoverState.TargetIdentity) { throw 'Cutover worker must be detached from the target tree.' }
    $force=Enter-CdrForceGate $RepoRoot
    $control=Enter-CdrForceControl $RepoRoot
    $latest=Read-CdrLatestCutoverState
    if ($crashRecovery) {
        if ((Test-CdrCutoverSettled $latest) -or $latest.LastError -or
            $latest.WorkerIdentity -cne $RecoverFromWorkerIdentity -or
            $latest.RecoveryIdentity -cne $RecoveryOwner -or [int]$latest.Recoveries -ne $RecoveryCount) { exit 0 }
        $previous=Get-CdrPinnedProcess ([pscustomobject]@{Identity=$RecoverFromWorkerIdentity;Path=(Join-Path $PSHOME 'powershell.exe')})
        if ($null -ne $previous) { $previous.Dispose(); exit 0 }
    }
    $script:CutoverState=$latest
    $script:CutoverState.WorkerIdentity=Get-RustProcessIdentity ([Diagnostics.Process]::GetCurrentProcess())
    $script:CutoverState.LastError=''
    $ownsWorkerState=$true
    Save-CdrCutoverPhase $script:CutoverState.Phase
    if (-not (Test-CdrCutoverSettled $script:CutoverState)) {
        if ((Get-CdrArtifactHash $CandidatePath) -cne $script:CutoverState.CandidateHash) { throw 'Candidate changed before cutover.' }
        $startup=New-CimInstance -ClassName Win32_ProcessStartup -ClientOnly -Property @{ShowWindow=[uint16]0}
        $command='"'+(Join-Path $PSHOME 'powershell.exe')+'" -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "'+$PSCommandPath+'" -StatePath "'+$StatePath+'" -Monitor'
        $created=Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
            CommandLine=$command;CurrentDirectory=$RepoRoot;ProcessStartupInformation=$startup
        }
        if ($created.ReturnValue -ne 0) { throw 'Independent recovery monitor could not start.' }
        $readyPath=Join-Path $script:CutoverBundle 'monitor.ready'
        $armed=$false;$deadline=[DateTimeOffset]::UtcNow.AddSeconds(15)
        while ([DateTimeOffset]::UtcNow -lt $deadline) {
            if ([IO.File]::Exists($readyPath)) {
                $ready=Read-CdrCutoverStateFile $readyPath -AsText
                if ($ready -match ('^'+[string]$created.ProcessId+'\|\d+$')) {
                    $monitorProcess=Get-CdrPinnedProcess ([pscustomobject]@{Identity=$ready;Path=(Join-Path $PSHOME 'powershell.exe')})
                    if ($null -ne $monitorProcess) { $monitorProcess.Dispose();$armed=$true;break }
                }
            }
            Start-Sleep -Milliseconds 100
        }
        if (-not $armed) { throw 'Recovery monitor is not armed; no termination permitted.' }
        $script:CutoverState.RecoveryIdentity=$ready
        Save-CdrCutoverPhase $script:CutoverState.Phase
    }
    Invoke-CdrManualReserveCutover
} catch {
    if ($ownsWorkerState) {
        $script:CutoverState.LastError=$_.Exception.Message
        Save-CdrCutoverPhase $script:CutoverState.Phase
    }
    exit 1
} finally {
    if ($null -ne $control) { $control.Dispose() }
    if ($null -ne $force) { $force.Dispose() }
}
