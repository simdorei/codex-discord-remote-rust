[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$FixtureRoot)
$ErrorActionPreference='Stop'
$sourceRoot=Split-Path -Parent $PSScriptRoot
[void][IO.Directory]::CreateDirectory($FixtureRoot)
$fake=Join-Path $FixtureRoot 'cleanup-candidate.exe'
Add-Type -OutputAssembly $fake -OutputType ConsoleApplication -TypeDefinition @'
using System;
using System.IO;
using System.Diagnostics;
using System.Threading;
public class CleanupCandidate {
    public static void Main(string[] args) {
        string root=args[0]; int pid=Process.GetCurrentProcess().Id;
        File.AppendAllText(Path.Combine(root,"launch-count.txt"),"launch\n");
        File.WriteAllText(Path.Combine(root,".codex_discord_rust.runtime.lock"),"pid="+pid+"\n");
        File.WriteAllText(Path.Combine(root,"codex_discord_rust.error.log"),"discord_ready user=1 application=2\n");
        for(int i=0;i<600;i++) {
            File.WriteAllText(Path.Combine(root,".codex_discord_rust.heartbeat"),
                "pid="+pid+"\nupdated_at="+DateTimeOffset.UtcNow.ToUnixTimeSeconds()+"\n");
            Thread.Sleep(200);
        }
    }
}
'@

function Invoke-CleanupCrashCase([bool]$ForeignSeal) {
    $case=if($ForeignSeal){'foreign'}else{'double-crash'}
    $root=Join-Path $FixtureRoot $case
    $operation=[guid]::NewGuid().ToString('N')
    $bundle=Join-Path $root ('maintenance_backups\manual-reserve\'+$operation)
    [void][IO.Directory]::CreateDirectory($bundle)
    $supports=@('codex-discord-rust-watchdog.ps1','codex-discord-rust-control.ps1','codex-discord-rust-drain.ps1',
        'scripts/CdrLaunchJournal.ps1','scripts/CdrForceRestart.ps1','scripts/CdrManualReserveCutover.ps1',
        'scripts/Invoke-CdrManualReserveCutover.ps1','scripts/CdrDeploymentRecovery.ps1','scripts/CdrRestartTransaction.ps1')
    foreach($rel in $supports) {
        $dest=Join-Path $root $rel
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($dest))
        [IO.File]::Copy((Join-Path $sourceRoot $rel),$dest,$true)
    }
    # Instrument only the copied test core, then pin that exact copy. The monitor
    # and entry point are unmodified production code; there is no production hook.
    $core=Join-Path $root 'scripts/CdrManualReserveCutover.ps1'
    $body=[IO.File]::ReadAllText($core).Replace('[IO.File]::Delete($DisablePath)',
        'Wait-TestCleanupBoundary; [IO.File]::Delete($DisablePath)')
    $gate=@'
function Wait-TestCleanupBoundary {
    $countPath=Join-Path $script:CutoverBundle 'cleanup-count.txt'
    $count=0
    if([IO.File]::Exists($countPath)) {$count=[int][IO.File]::ReadAllText($countPath)}
    $count++
    [IO.File]::WriteAllText($countPath,[string]$count)
    if($count -le 2) {
        [IO.File]::WriteAllText((Join-Path $script:CutoverBundle ('boundary-'+$count+'.txt')),
            (Get-RustProcessIdentity ([Diagnostics.Process]::GetCurrentProcess())))
        Start-Sleep -Seconds 120
        throw 'Fixture cleanup boundary was not killed'
    }
}
'@
    [IO.File]::WriteAllText($core,($body+"`n"+$gate))
    foreach($rel in @('target\release\cdr-runtime.exe','target\force-bang\release\cdr-runtime.exe')) {
        $dest=Join-Path $root $rel
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($dest))
        [IO.File]::Copy($fake,$dest,$true)
    }
    $installed=Join-Path $root 'target\release\cdr-runtime.exe'
    $candidate=Start-Process -FilePath $installed -ArgumentList ('"'+$root+'"') -WindowStyle Hidden -PassThru
    $processPins=[Collections.Generic.List[object]]::new()
    try {
        $candidateIdentity=Get-RustProcessIdentity $candidate
        $hash=Get-CdrArtifactHash $installed
        $pins=@($supports[0..6] | ForEach-Object {@{Path=$_;Hash=(Get-CdrArtifactHash (Join-Path $root $_))}})
        $request=@{Version=1;Operation=$operation;RepoRoot=$root;BaselineHash=$hash;CandidateHash=$hash;
            TargetIdentity='1|1';ProgramPins=$pins}
        $requestPath=Join-Path $bundle 'request.json'
        [IO.File]::WriteAllText($requestPath,($request|ConvertTo-Json -Depth 8))
        $requestHash=Get-CdrArtifactHash $requestPath
        $state=@{Version=1;Operation=$operation;RepoRoot=$root;BaselineHash=$hash;CandidateHash=$hash;
            TargetIdentity='1|1';RequestHash=$requestHash;Phase='launched';WorkerIdentity='';RecoveryIdentity='';
            Recoveries=0;LastError='';UpdatedAt='';Captured=@();CandidateIdentity=$candidateIdentity;
            DatabaseBackup='fixture-prior-backup';DatabaseBackupHash='';Retirement='fixture-prior-retirement';Readiness=$null}
        $statePath=Join-Path $bundle 'state.json'
        [IO.File]::WriteAllText($statePath,($state|ConvertTo-Json -Depth 12))
        $disable=Join-Path $root '.codex_discord_bot.disabled'
        [IO.File]::WriteAllText($disable,('manual-reserve:'+$operation+':'+$requestHash))
        $entry=Join-Path $root 'scripts/Invoke-CdrManualReserveCutover.ps1'
        $startup=New-CimInstance -ClassName Win32_ProcessStartup -ClientOnly -Property @{ShowWindow=[uint16]0}
        $created=Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
            CommandLine=('"'+(Join-Path $PSHOME 'powershell.exe')+'" -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "'+$entry+'" -StatePath "'+$statePath+'"');
            CurrentDirectory=$root;ProcessStartupInformation=$startup}
        Assert-True ($created.ReturnValue -eq 0) 'Cleanup worker failed to launch'
        $crashes=if($ForeignSeal){1}else{2}
        for($i=1;$i -le $crashes;$i++) {
            $boundary=Join-Path $bundle ('boundary-'+$i+'.txt')
            $deadline=[DateTimeOffset]::UtcNow.AddSeconds(25)
            while(-not [IO.File]::Exists($boundary)) {
                if([DateTimeOffset]::UtcNow -gt $deadline){throw ('Cleanup boundary timed out: '+$case+':'+$i)}
                Start-Sleep -Milliseconds 100
            }
            $current=Read-CdrCutoverStateFile $statePath
            Assert-True ($current.Phase -eq 'complete' -and [IO.File]::Exists($disable)) 'Did not reach persisted completion before cleanup'
            Assert-True ($current.Recoveries -eq ($i-1)) 'Unexpected recovery count'
            foreach($identity in @($current.WorkerIdentity,$current.RecoveryIdentity)) {
                $pin=[pscustomobject]@{Identity=$identity;Path=(Join-Path $PSHOME 'powershell.exe')}
                $processPins.Add($pin)
                $live=Get-CdrPinnedProcess $pin
                Assert-True ($null -ne $live) 'Cleanup reentry did not arm a live independent monitor'
                $live.Dispose()
            }
            $workerPin=[pscustomobject]@{Identity=([IO.File]::ReadAllText($boundary));Path=(Join-Path $PSHOME 'powershell.exe')}
            Assert-True ($workerPin.Identity -ceq $current.WorkerIdentity) 'Boundary belongs to another worker'
            if($ForeignSeal){[IO.File]::WriteAllText($disable,'foreign-maintenance-owner')}
            $worker=Get-CdrPinnedProcess $workerPin
            try {$worker.Kill();Assert-True ($worker.WaitForExit(3000)) 'Cleanup worker did not die'}finally{$worker.Dispose()}
        }
        $deadline=[DateTimeOffset]::UtcNow.AddSeconds(25)
        do {
            $current=Read-CdrCutoverStateFile $statePath
            $finished=if($ForeignSeal){[bool]$current.LastError}else{-not [IO.File]::Exists($disable)}
            if([DateTimeOffset]::UtcNow -gt $deadline){throw ('Independent cleanup did not settle: '+$case)}
            if(-not $finished){Start-Sleep -Milliseconds 100}
        } while(-not $finished)
        Assert-True ($current.Recoveries -eq $crashes) 'Monitor did not perform exact automatic reentries'
        Assert-True ($current.CandidateIdentity -ceq $candidateIdentity -and -not $candidate.HasExited) 'Recovery changed candidate identity'
        Assert-True ([IO.File]::ReadAllLines((Join-Path $root 'launch-count.txt')).Count -eq 1) 'Cleanup launched another candidate'
        if($ForeignSeal) {
            Assert-True ([IO.File]::ReadAllText($disable) -ceq 'foreign-maintenance-owner') 'Foreign seal was consumed'
            Assert-True ($current.LastError -like '*another owner*') 'Foreign cleanup refused for wrong reason'
            Write-Output 'PASS real monitor preserves foreign seal after complete-phase worker death'
        } else {
            Assert-True ($current.Phase -eq 'complete' -and -not $current.LastError) 'Cleanup did not complete'
            Write-Output 'PASS real monitor recovers two cleanup worker deaths after complete without manual reentry or duplicate launch'
        }
    } finally {
        if([IO.File]::Exists($statePath)) {
            $last=Read-CdrCutoverStateFile $statePath
            foreach($identity in @($last.WorkerIdentity,$last.RecoveryIdentity)) {
                if($identity){$processPins.Add([pscustomobject]@{Identity=$identity;Path=(Join-Path $PSHOME 'powershell.exe')})}
            }
        }
        foreach($pin in $processPins) {
            $live=Get-CdrPinnedProcess $pin
            if($null -ne $live){try{if(-not $live.WaitForExit(2000)){$live.Kill();[void]$live.WaitForExit(3000)}}finally{$live.Dispose()}}
        }
        if(-not $candidate.HasExited){$candidate.Kill();[void]$candidate.WaitForExit(3000)}
        $candidate.Dispose()
    }
}
Invoke-CleanupCrashCase $false
Invoke-CleanupCrashCase $true
