[CmdletBinding()]
param([string]$Repository, [string]$FixtureRoot, [string]$Scenario)
$ErrorActionPreference='Stop'
$deployment=$Scenario.StartsWith('deployment-')
$RepoRoot=[IO.Path]::GetFullPath($FixtureRoot)
$BinaryPath=Join-Path $RepoRoot 'fixture-runtime.exe'
$EnvPath=Join-Path $RepoRoot '.env'
$LockPath=Join-Path $RepoRoot '.codex_discord_rust.runtime.lock'
$StdoutLog=Join-Path $RepoRoot 'stdout.log'
$StderrLog=Join-Path $RepoRoot 'stderr.log'
$LauncherLog=Join-Path $RepoRoot 'launcher.log'
$relativeEnvironment=$Scenario -in @('relative-env','wrong-relative-env')
if ($relativeEnvironment) { $EnvPath=Join-Path $RepoRoot 'selected.env' }
$DbPath=Join-Path $RepoRoot 'store.sqlite'
$HeartbeatPath=Join-Path $RepoRoot '.codex_discord_rust.heartbeat'
$StopPath=Join-Path $RepoRoot '.codex_discord_rust.stop'
$DisablePath=Join-Path $RepoRoot '.codex_discord_bot.disabled'
$DrainPreparePath=Join-Path $RepoRoot '.codex_discord_rust.drain.prepare'
$DrainAckPath=Join-Path $RepoRoot '.codex_discord_rust.drain.ack'
$DrainIdentityPath=Join-Path $RepoRoot '.codex_discord_rust.drain.identity'
$RestartPath=Join-Path $RepoRoot '.codex_discord_rust.restart'
$HealthHeartbeatMaxAgeSeconds=45
$HealthHeartbeatStartupGraceSeconds=120
$capPath=Join-Path $RepoRoot 'capabilities.json'
$contractPath=Join-Path $RepoRoot '.codex_discord_rust.compatibility.json'
$requiredPath=Join-Path $RepoRoot '.codex_discord_rust.compatibility.required'
$source=Join-Path $Repository 'scripts/candidates/async-recovery-v1/codex-discord-rust-watchdog.ps1'
$module=Join-Path $Repository 'scripts/candidates/async-recovery-v1/CdrAsyncRecoveryCompatibility.ps1'
$launchModule=Join-Path $Repository 'scripts/candidates/async-recovery-v1/CdrRuntimeLaunchCompatibility.ps1'
. (Join-Path $Repository 'codex-discord-rust-control.ps1')
. $module
if (Test-Path -LiteralPath $launchModule) { . $launchModule }
$tokens=$null; $errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($source,[ref]$tokens,[ref]$errors)
if ($errors.Count) { throw 'candidate watchdog parse failed' }
# Execute only the real candidate entry's optional journal initialization, if
# present. Do not manufacture it in the normal/legacy regression fixtures.
$journalInitializers=@($ast.EndBlock.Statements | Where-Object {
    $_ -is [Management.Automation.Language.AssignmentStatementAst] -and
        $_.Left.Extent.Text -ceq '$script:CdrLaunchJournalPath'
})
foreach ($initializer in $journalInitializers) { Invoke-Expression $initializer.Extent.Text }
foreach ($name in @('Start-RustRuntime','Get-RuntimePid','Get-VerifiedRustProcessById',
        'Get-VerifiedRuntimeProcess','Get-RustProcessIdentity','Write-RustWatchdogLog',
        'Get-VerifiedRuntimeIdentity','Get-HeartbeatHealth','Clear-DeadRuntimeArtifacts','Wait-RustRuntimeExit')) {
    $function=$ast.FindAll({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst]},$false) |
        Where-Object Name -eq $name
    if (@($function).Count -ne 1) { throw "missing exact function $name" }
    Invoke-Expression $function.Extent.Text
}
# Compile a private native peer. It has no networking, Discord, SQL, or Codex code.
Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.IO;
using System.Threading;
public static class LauncherPeer {
    public static int Main(string[] args) {
        string root=Environment.CurrentDirectory;
        if (args.Length>0 && args[0]=="--fixture-old") {
            Thread.Sleep(30000);
            return 0;
        }
        if (args.Length>0 && args[0]=="--admin") {
            File.AppendAllText(Path.Combine(root,"probes.log"),"probe\n");
            string real=Environment.GetEnvironmentVariable("CDR_TEST_NATIVE_PROBE");
            if (!String.IsNullOrEmpty(real)) {
                var start=new ProcessStartInfo(real);
                start.Arguments=String.Join(" ",Array.ConvertAll(args,
                    value=>"\""+value.Replace("\"","\\\"")+"\""));
                start.WorkingDirectory=root;
                start.UseShellExecute=false;
                start.CreateNoWindow=true;
                start.WindowStyle=ProcessWindowStyle.Hidden;
                start.RedirectStandardOutput=true;
                start.RedirectStandardError=true;
                using (var process=Process.Start(start)) {
                    var output=process.StandardOutput.ReadToEndAsync();
                    var errors=process.StandardError.ReadToEndAsync();
                    process.WaitForExit();
                    Console.Write(output.GetAwaiter().GetResult());
                    Console.Error.Write(errors.GetAwaiter().GetResult());
                    return process.ExitCode;
                }
            }
            Console.Write(File.ReadAllText(Path.Combine(root,"probe-response.json")));
            return 0;
        }
        File.AppendAllText(Path.Combine(root,"starts.log"),"start\n");
        File.WriteAllText(Path.Combine(root,"selected-env.txt"),
            Path.GetFullPath(args[1])+"\n"+File.ReadAllText(args[1]));
        File.WriteAllText(Path.Combine(root,".codex_discord_rust.runtime.lock"),
            "pid="+Process.GetCurrentProcess().Id+"\n");
        File.WriteAllText(Path.Combine(root,".codex_discord_rust.heartbeat"),
            "pid="+Process.GetCurrentProcess().Id+"\nupdated_at="+DateTimeOffset.UtcNow.ToUnixTimeSeconds()+"\n");
        Thread.Sleep(30000);
        return 0;
    }
}
'@ -OutputAssembly $BinaryPath -OutputType ConsoleApplication
[IO.File]::WriteAllText($EnvPath,"CODEX_DISCORD_MIRROR_DB=$DbPath")
if (-not $deployment) {
    [IO.File]::WriteAllText($DbPath,'opaque native-peer fixture; actual SQLite contracts use the real Rust binary')
}
function Hash([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
$cap=[ordered]@{protocol='cdr-artifact-capabilities-v1'; artifact_sha256=(Hash $BinaryPath)
    async_resolution_max_format=1; async_recovery_policy_max_format=1}
if ($Scenario -eq 'unsupported') { $cap.async_recovery_policy_max_format=0 }
[IO.File]::WriteAllText($capPath,($cap | ConvertTo-Json -Compress))
$contract=[ordered]@{protocol='cdr-runtime-launch-v1'; root=$RepoRoot
    candidate_path=$BinaryPath; candidate_sha256=(Hash $BinaryPath)
    environment_path=$EnvPath; environment_sha256=(Hash $EnvPath)
    database_path=$DbPath; capability_path=$capPath; capability_sha256=(Hash $capPath)}
$caller=Join-Path $RepoRoot 'caller'
$parentEnv=Join-Path $caller 'selected.env'
if ($relativeEnvironment) {
    [void][IO.Directory]::CreateDirectory($caller)
    [IO.File]::WriteAllText($parentEnv,"CODEX_DISCORD_MIRROR_DB=$DbPath")
    if ($Scenario -eq 'wrong-relative-env') {
        [IO.File]::WriteAllText($EnvPath,'CODEX_DISCORD_MIRROR_DB=other.sqlite')
        $contract.environment_path=$parentEnv
        $contract.environment_sha256=Hash $parentEnv
    } else { [IO.File]::WriteAllText($parentEnv,'CODEX_DISCORD_MIRROR_DB=other.sqlite') }
}
if ($Scenario -notin @('legacy','legacy-journal')) {
    [IO.File]::WriteAllText($contractPath,($contract | ConvertTo-Json -Compress))
    [IO.File]::WriteAllText($requiredPath,(Hash $contractPath))
}
if ($Scenario -eq 'changed-env') { [IO.File]::AppendAllText($EnvPath,([Environment]::NewLine+"CHANGED=1")) }
if ($Scenario -eq 'partial') { [IO.File]::Delete($requiredPath) }
$response=[ordered]@{protocol='cdr-recovery-compatibility-v1'; read_only=$true; compatible=$true
    admission_authorized=$false; recovery_authorized=$false
    database=$DbPath; supported_async_resolution_format=1; required_async_resolution_format=1
    supported_async_recovery_policy_format=1; required_async_recovery_policy_format=1
    configured_environment_verified=$true; environment=$EnvPath}
if ($Scenario -eq 'wrong-db') { $response.database=Join-Path $RepoRoot 'other.sqlite' }
if ($Scenario -eq 'wrong-relative-env') { $response.environment=$parentEnv }
[IO.File]::WriteAllText((Join-Path $RepoRoot 'probe-response.json'),($response | ConvertTo-Json -Compress))
$script:pinChecks=0
function Set-CdrLaunchStarting {
    if ($Scenario -eq 'valid') {
        foreach ($path in @($BinaryPath,$EnvPath,$capPath,$contractPath,$requiredPath)) {
            $writer=$null
            try { $writer=[IO.File]::Open($path,'Open','Write','ReadWrite') }
            catch [IO.IOException] { $script:pinChecks++; continue }
            finally { if ($null -ne $writer) { $writer.Dispose() } }
            throw "artifact was not pinned through launch: $path"
        }
        $other=$null
        try { $other=Enter-CdrControl -Root $RepoRoot }
        catch { $script:pinChecks++; return }
        finally { if ($null -ne $other) { $other.Dispose() } }
        throw 'second control writer was admitted'
    }
}
function Set-CdrLaunchChild { param($Process) }
if ($deployment -or $Scenario -in @('normal-journal','legacy-journal','owned-journal','launching-journal',
        'relative-env','wrong-relative-env')) {
    . (Join-Path $Repository 'codex-discord-rust-drain.ps1')
    . (Join-Path $Repository 'scripts/CdrLaunchJournal.ps1')
}
if ($deployment) {
    . (Join-Path $Repository 'scripts/candidates/async-recovery-v1/CdrDeploymentRecovery.ps1')
    # Obtain and definitively stop only our retained private original process.
    $oldStart=New-Object Diagnostics.ProcessStartInfo
    $oldStart.FileName=$BinaryPath
    $oldStart.Arguments='--fixture-old'
    $oldStart.WorkingDirectory=$RepoRoot
    $oldStart.UseShellExecute=$false
    $oldStart.CreateNoWindow=$true
    $oldStart.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $old=[Diagnostics.Process]::Start($oldStart)
    try {
        $oldId=$old.Id
        $oldTicks=$old.StartTime.ToUniversalTime().Ticks.ToString()
        $old.Kill()
        if (-not $old.WaitForExit(3000)) { throw 'private original process did not exit' }
    } finally {
        if (-not $old.HasExited) { $old.Kill(); [void]$old.WaitForExit(3000) }
        $old.Dispose()
    }
    $recoveryStatePath=Join-Path $RepoRoot 'fixture-recovery.json'
    $recoveryMarker='fixture-recovery-owner'
    $artifact=Get-CdrArtifactHash $BinaryPath
    $recoveryState=[ordered]@{Version=1;RepoRoot=$RepoRoot;BinaryPath=$BinaryPath
        RuntimePid=$oldId;RuntimeTicks=$oldTicks;Marker=$recoveryMarker
        BaselineHash=$artifact;CandidateHash=$artifact;LogPath=(Join-Path $RepoRoot 'recovery.log')}
    if ($Scenario -eq 'deployment-versionless') { $recoveryState.Remove('Version') }
    if ($Scenario -eq 'deployment-newer') { $recoveryState.Version=2 }
    [IO.File]::WriteAllText($recoveryStatePath,($recoveryState|ConvertTo-Json -Compress))
    [IO.File]::WriteAllText($StopPath,$recoveryMarker)
    [IO.File]::WriteAllText($DisablePath,$recoveryMarker)
    if ($Scenario -eq 'deployment-unknown') {
        $record=New-CdrLaunchJournal ($recoveryStatePath+'.launch') ('deployment:'+$recoveryMarker) -ArtifactHash $artifact
        $record.Phase='launching'
        Save-CdrLaunchJournal ($recoveryStatePath+'.launch') $record
        $unknownBefore=[IO.File]::ReadAllText($recoveryStatePath+'.launch')
    }
}
if ($Scenario -in @('owned-journal','launching-journal')) {
    $script:CdrLaunchJournalPath=Join-Path $RepoRoot 'fixture.launch'
    $record=New-CdrLaunchJournal $script:CdrLaunchJournalPath 'fixture-owned' -ArtifactHash (Hash $BinaryPath)
    if ($Scenario -eq 'launching-journal') {
        $record.Phase='launching'
        Save-CdrLaunchJournal $script:CdrLaunchJournalPath $record
    }
}
if ($relativeEnvironment) {
    # Isolate the environment counterexample from the separate missing-journal
    # initialization defect, as on an already initialized real restart path.
    $script:CdrLaunchJournalPath=$null
    $EnvPath='selected.env'
    Set-Location -LiteralPath $caller
    [Environment]::CurrentDirectory=$caller
}
$controlGuard=$null
$script:RustRestartStartedProcess=$null
$ownedChild=$null
$caught=''
try {
    if ($Scenario -ne 'no-control') { $controlGuard=Enter-CdrControl -Root $RepoRoot }
    try {
        if ($Scenario -eq 'expired') { Start-RustRuntime -DeadlineUtc ([DateTimeOffset]::UtcNow.AddSeconds(-1)) }
        elseif ($deployment) { Invoke-CdrDeploymentRecovery -StatePath $recoveryStatePath }
        else { Start-RustRuntime }
        if ($Scenario -eq 'owned-journal') {
            $ownedChild=$script:RustRestartStartedProcess
            $record=Read-CdrLaunchJournal $script:CdrLaunchJournalPath
            if ($record.Phase -cne 'child' -or
                $record.ChildIdentity -cne (Get-RustProcessIdentity $ownedChild)) {
                throw 'actual journal did not durably record the exact launched child'
            }
            $secondError=''
            try { Start-RustRuntime } catch { $secondError=$_.Exception.Message }
            if (-not $secondError.Contains('Launch already attempted')) {
                throw 'actual journal did not reject duplicate launch'
            }
            $after=Read-CdrLaunchJournal $script:CdrLaunchJournalPath
            if ($after.Phase -cne 'child' -or $after.ChildIdentity -cne $record.ChildIdentity) {
                throw 'duplicate attempt changed the original child journal'
            }
        }
    } catch { $caught=$_.Exception.Message }
    $starts=0; $probes=0
    if (Test-Path -LiteralPath (Join-Path $RepoRoot 'starts.log')) {
        $starts=@(Get-Content -LiteralPath (Join-Path $RepoRoot 'starts.log')).Count
    }
    if (Test-Path -LiteralPath (Join-Path $RepoRoot 'probes.log')) {
        $probes=@(Get-Content -LiteralPath (Join-Path $RepoRoot 'probes.log')).Count
    }
    $expected=if ($Scenario -in @('valid','legacy','normal-journal','legacy-journal','owned-journal','relative-env','deployment-compatible','deployment-versionless')) { 1 } else { 0 }
    [Console]::WriteLine((@{scenario=$Scenario; starts=$starts; probes=$probes; error=$caught; pin_checks=$script:pinChecks} | ConvertTo-Json -Compress))
    if ($starts -ne $expected) { throw "expected runtime starts=$expected; actual=$starts" }
    if ($expected -eq 0 -and -not $caught) { throw 'held launch had no error' }
    if ($expected -eq 1 -and $caught) { throw "valid launch failed: $caught" }
    if ($Scenario -eq 'valid' -and ($script:pinChecks -ne 6 -or $probes -ne 1)) {
        throw 'valid launch did not retain all pins and its control lease'
    }
    if ($Scenario -eq 'legacy' -and $probes -ne 0) { throw 'unarmed legacy startup changed' }
    if ($Scenario -eq 'normal-journal' -and $probes -ne 1) { throw 'normal journal start did not probe' }
    if ($Scenario -eq 'legacy-journal' -and $probes -ne 0) { throw 'legacy journal start unexpectedly probed' }
    if ($Scenario -eq 'owned-journal' -and $probes -ne 2) { throw 'duplicate journal check did not use the real start path' }
    if ($Scenario -eq 'launching-journal') {
        if ((Read-CdrLaunchJournal $script:CdrLaunchJournalPath).Phase -cne 'launching') {
            throw 'unknown launch journal was changed'
        }
    }
    if ($Scenario -eq 'relative-env') {
        $selected=[IO.File]::ReadAllText((Join-Path $RepoRoot 'selected-env.txt')).Split([char]10)[0]
        if (-not [StringComparer]::OrdinalIgnoreCase.Equals($selected,(Join-Path $RepoRoot 'selected.env'))) {
            throw 'native child read a different environment file'
        }
    }
    if ($Scenario -in @('deployment-compatible','deployment-versionless')) {
        if ($probes -ne 1 -or -not [IO.File]::Exists($recoveryStatePath+'.completed') -or
            [IO.File]::Exists($recoveryStatePath+'.launch') -or [IO.File]::Exists($DisablePath)) {
            throw 'compatible recovery did not complete its exact child journal and receipt'
        }
        $receipt=[IO.File]::ReadAllText($recoveryStatePath+'.completed') | ConvertFrom-Json
        if ($receipt.ReplacementIdentity -cne (Get-RustProcessIdentity $script:RustRestartStartedProcess)) {
            throw 'recovery completion receipt names another child'
        }
    }
    if ($Scenario -in @('deployment-incompatible','deployment-unknown')) {
        if ([IO.File]::ReadAllText($DisablePath) -cne $recoveryMarker -or
            [IO.File]::Exists($recoveryStatePath+'.completed')) {
            throw 'failed or unknown recovery lost its seal or fabricated completion'
        }
        $journal=Read-CdrLaunchJournal ($recoveryStatePath+'.launch')
        if ($Scenario -eq 'deployment-incompatible' -and ($journal.Phase -cne 'prepared' -or $probes -ne 1)) {
            throw 'incompatible recovery advanced launch state or skipped the real probe'
        }
        if ($Scenario -eq 'deployment-unknown' -and ($probes -ne 0 -or
            [IO.File]::ReadAllText($recoveryStatePath+'.launch') -cne $unknownBefore)) {
            throw 'unknown recovery probed, relaunched, or rewrote its evidence'
        }
    }
    if ($Scenario -eq 'deployment-newer') {
        if ($probes -ne 0 -or [IO.File]::Exists($recoveryStatePath+'.launch') -or
            [IO.File]::Exists($recoveryStatePath+'.completed') -or
            [IO.File]::ReadAllText($StopPath) -cne $recoveryMarker -or
            [IO.File]::ReadAllText($DisablePath) -cne $recoveryMarker) {
            throw 'unsupported recovery version changed intent, probed or created a launch journal'
        }
    }
    $expectedError=@{
        'unsupported'='recovery policy capability is unsupported'
        'changed-env'='Pinned artifact hash mismatch'
        'no-control'='retained root control lease'
        'partial'='Incomplete armed compatibility installation'
        'wrong-db'='probe identity or capability mismatch'
        'expired'='maintenance_deadline_exceeded_before_launch'
        'launching-journal'='Launch already attempted'
        'wrong-relative-env'='Pinned launch identity differs'
        'deployment-incompatible'='Compatibility probe rejected candidate'
        'deployment-unknown'='launch_outcome_unknown'
        'deployment-newer'='maintenance_v2_or_unknown'
    }
    if ($expectedError.ContainsKey($Scenario) -and -not $caught.Contains($expectedError[$Scenario])) {
        throw "scenario rejected at the wrong boundary: $caught"
    }
    if ($Scenario -eq 'wrong-db' -and $probes -ne 1) { throw 'database identity was not probed' }
    if ($Scenario -in @('unsupported','changed-env','no-control','partial','expired') -and $probes -ne 0) {
        throw 'a pre-probe rejection executed the candidate'
    }
    if ($Scenario -eq 'wrong-relative-env' -and $probes -ne 0) { throw 'wrong environment identity was probed' }
} finally {
    # Stop only the retained child created by this fixture invocation.
    $children=@($ownedChild,$script:RustRestartStartedProcess) | Where-Object { $null -ne $_ } |
        Group-Object -Property Id | ForEach-Object { $_.Group[0] }
    foreach ($child in $children) {
        try {
            if (-not $child.HasExited) {
                $child.Kill()
                [void]$child.WaitForExit(3000)
            }
        } finally { $child.Dispose() }
    }
    if ($null -ne $controlGuard) { $controlGuard.Dispose() }
}
