[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$FixtureRoot)
$ErrorActionPreference='Stop'
$sourceRoot=Split-Path -Parent $PSScriptRoot
# Called by the Windows cutover contract with its standard assertion/process helpers.
function Wait-MonitorFixture($Condition,[string]$Label) {
    $deadline=[DateTimeOffset]::UtcNow.AddSeconds(20)
    while (-not (& $Condition)) {
        if ([DateTimeOffset]::UtcNow -gt $deadline) { throw ('Monitor fixture timeout: '+$Label) }
        Start-Sleep -Milliseconds 50
    }
}
foreach ($case in @('failure','complete','claim-race')) {
    $root=Join-Path $FixtureRoot $case
    $operation=[guid]::NewGuid().ToString('N')
    $bundle=Join-Path $root ('maintenance_backups\manual-reserve\'+$operation)
    [void][IO.Directory]::CreateDirectory($bundle)
    $supports=@('codex-discord-rust-watchdog.ps1','codex-discord-rust-control.ps1','codex-discord-rust-drain.ps1',
        'scripts/CdrLaunchJournal.ps1','scripts/CdrForceRestart.ps1','scripts/CdrManualReserveCutover.ps1',
        'scripts/Invoke-CdrManualReserveCutover.ps1','scripts/CdrDeploymentRecovery.ps1','scripts/CdrRestartTransaction.ps1')
    foreach ($rel in $supports) {
        $dest=Join-Path $root $rel
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($dest))
        [IO.File]::Copy((Join-Path $sourceRoot $rel),$dest,$true)
    }
    $entry=Join-Path $root 'scripts/Invoke-CdrManualReserveCutover.ps1'
    $body=[IO.File]::ReadAllText($entry)
    $poll='$state=Read-CdrCutoverStateFile $StatePath'
    Assert-True (($body.Split(@($poll),[StringSplitOptions]::None)).Count -eq 2) 'Polling boundary changed'
    $pause=@'
if (-not [IO.File]::Exists((Join-Path $script:CutoverBundle 'observed'))) {
    [IO.File]::WriteAllText((Join-Path $script:CutoverBundle 'observed'),$state.LastError)
    while (-not [IO.File]::Exists((Join-Path $script:CutoverBundle 'release-monitor'))) { Start-Sleep -Milliseconds 50 }
}
'@
    $body=$body.Replace($poll,($poll+"`n"+$pause))
    if ($case -eq 'claim-race') {
        $call='& $PSCommandPath -StatePath $StatePath -RecoverFromWorkerIdentity $watched -RecoveryOwner $monitorIdentity -RecoveryCount $claimedRecovery'
        Assert-True ($body.Contains($call)) 'Conditional entry boundary changed'
        $pauseClaim='[IO.File]::WriteAllText((Join-Path $script:CutoverBundle ''claimed''),''ready''); while (-not [IO.File]::Exists((Join-Path $script:CutoverBundle ''release-entry''))) { Start-Sleep -Milliseconds 50 }; '
        $body=$body.Replace($call,($pauseClaim+$call))
    }
    [IO.File]::WriteAllText($entry,$body)
    $core=Join-Path $root 'scripts/CdrManualReserveCutover.ps1'
    # Only copied fixture code gets deterministic pause/terminal boundaries.
    # Entry, catch/finally, claim validation, locks and real monitor all execute.
    $fixtureCore=@'
function Invoke-CdrManualReserveCutover {
    [IO.File]::AppendAllText((Join-Path $script:CutoverBundle 'worker-count'),"worker`n")
    while (-not [IO.File]::Exists((Join-Path $script:CutoverBundle 'observed'))) { Start-Sleep -Milliseconds 50 }
    $case=[IO.File]::ReadAllText((Join-Path $script:CutoverBundle 'case'))
    if ($case -eq 'claim-race') { [Diagnostics.Process]::GetCurrentProcess().Kill(); return }
    if ($case -eq 'failure') { throw 'fixture reported failure' }
    Save-CdrCutoverPhase 'complete'
    [IO.File]::Delete($DisablePath)
}
'@
    [IO.File]::AppendAllText($core,("`n"+$fixtureCore))
    [IO.File]::WriteAllText((Join-Path $bundle 'case'),$case)
    foreach ($rel in @('target/release/cdr-runtime.exe','target/force-bang/release/cdr-runtime.exe')) {
        $dest=Join-Path $root $rel
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($dest))
        [IO.File]::Copy((Join-Path $PSHOME 'powershell.exe'),$dest,$true)
    }
    $binaryHash=Get-CdrArtifactHash (Join-Path $root 'target/release/cdr-runtime.exe')
    $pins=@($supports[0..6] | ForEach-Object {@{Path=$_;Hash=(Get-CdrArtifactHash (Join-Path $root $_))}})
    $request=@{Version=1;Operation=$operation;RepoRoot=$root;BaselineHash=$binaryHash;CandidateHash=$binaryHash;TargetIdentity='1|1';ProgramPins=$pins}
    $requestPath=Join-Path $bundle 'request.json'
    [IO.File]::WriteAllText($requestPath,($request|ConvertTo-Json -Depth 8))
    $state=@{Version=1;Operation=$operation;RepoRoot=$root;BaselineHash=$binaryHash;CandidateHash=$binaryHash;TargetIdentity='1|1';
        RequestHash=(Get-CdrArtifactHash $requestPath);Phase='prepared';WorkerIdentity='';RecoveryIdentity='';Recoveries=0;LastError='';UpdatedAt='';
        Captured=@();CandidateIdentity='';DatabaseBackup='';DatabaseBackupHash='';Retirement='';Readiness=$null}
    $statePath=Join-Path $bundle 'state.json'
    [IO.File]::WriteAllText($statePath,($state|ConvertTo-Json -Depth 12))
    [IO.File]::WriteAllText((Join-Path $root '.codex_discord_bot.disabled'),'fixture-only')
    $processPins=[Collections.Generic.List[object]]::new()
    try {
        $startup=New-CimInstance -ClassName Win32_ProcessStartup -ClientOnly -Property @{ShowWindow=[uint16]0}
        $created=Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
            CommandLine=('"'+(Join-Path $PSHOME 'powershell.exe')+'" -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "'+$entry+'" -StatePath "'+$statePath+'"');
            CurrentDirectory=$root;ProcessStartupInformation=$startup}
        Assert-True ($created.ReturnValue -eq 0) 'Monitor race worker did not launch'
        Wait-MonitorFixture { [IO.File]::Exists((Join-Path $bundle 'worker-count')) } 'worker armed monitor'
        Wait-MonitorFixture { [IO.File]::Exists((Join-Path $bundle 'observed')) } 'pre-terminal snapshot'
        Assert-True ([IO.File]::ReadAllText((Join-Path $bundle 'observed')) -ceq '') 'Snapshot was already terminal'
        $current=Read-CdrCutoverStateFile $statePath
        foreach ($identity in @($current.WorkerIdentity,$current.RecoveryIdentity)) {
            $processPins.Add([pscustomobject]@{Identity=$identity;Path=(Join-Path $PSHOME 'powershell.exe')})
        }
        Wait-MonitorFixture {
            $live=Get-CdrPinnedProcess $processPins[0]
            if ($null -eq $live) { return $true };$live.Dispose();return $false
        } 'original worker exit'
        if ($case -eq 'claim-race') {
            [IO.File]::WriteAllText((Join-Path $bundle 'release-monitor'),'')
            Wait-MonitorFixture { [IO.File]::Exists((Join-Path $bundle 'claimed')) } 'claim released before entry'
            $f=[IO.File]::Open((Join-Path $root '.codex_discord_rust.force.lock'),'Open','ReadWrite','None');$c=$null
            try {
                $c=[IO.File]::Open((Join-Path $root '.codex_discord_rust.control.lock'),'Open','ReadWrite','None')
                $current=Read-CdrCutoverStateFile $statePath
                $current.LastError='fixture reported failure'
                Write-AtomicRestartMarker $statePath ($current|ConvertTo-Json -Depth 12 -Compress)
            } finally { if ($null -ne $c) { $c.Dispose() };$f.Dispose() }
        }
        $before=Get-CdrArtifactHash $statePath
        [IO.File]::WriteAllText((Join-Path $bundle 'release-monitor'),'')
        [IO.File]::WriteAllText((Join-Path $bundle 'release-entry'),'')
        Wait-MonitorFixture {
            $live=Get-CdrPinnedProcess $processPins[1]
            if ($null -eq $live) { return $true };$live.Dispose();return $false
        } 'monitor/recovery entry exit'
        $current=Read-CdrCutoverStateFile $statePath
        Assert-True ((Get-CdrArtifactHash $statePath) -ceq $before) 'Monitor rewrote newer terminal evidence'
        Assert-True ([IO.File]::ReadAllLines((Join-Path $bundle 'worker-count')).Count -eq 1) 'Automatic second worker executed'
        $expectedRecoveries=if($case -eq 'claim-race'){1}else{0}
        Assert-True ($current.Recoveries -eq $expectedRecoveries) 'Unexpected recovery claim count'
        if ($case -eq 'complete') {
            Assert-True ($current.Phase -eq 'complete' -and -not $current.LastError -and -not [IO.File]::Exists((Join-Path $root '.codex_discord_bot.disabled'))) 'Completion rewound'
        } else { Assert-True ($current.LastError -ceq 'fixture reported failure') 'Reported failure lost' }
        Write-Output ('PASS real monitor stale-state boundary '+$case+' preserves latest evidence and refuses unreviewed reentry')
    } finally {
        if ([IO.File]::Exists($statePath)) {
            $last=Read-CdrCutoverStateFile $statePath
            foreach ($identity in @($last.WorkerIdentity,$last.RecoveryIdentity)) {
                if ($identity) { $processPins.Add([pscustomobject]@{Identity=$identity;Path=(Join-Path $PSHOME 'powershell.exe')}) }
            }
        }
        foreach ($pin in $processPins) {
            $live=Get-CdrPinnedProcess $pin
            if ($null -ne $live) { try { if (-not $live.WaitForExit(2000)) { $live.Kill();[void]$live.WaitForExit(3000) } } finally { $live.Dispose() } }
        }
    }
}
