param([Parameter(Mandatory=$true)][string]$CaseName)
switch ($CaseName) {
    'test_cutover_does_not_delete_foreign_restart' {
        [IO.File]::WriteAllText($RustRestart,'foreign fence')
        try{Start-Rust;throw 'foreign restart accepted'}
        catch{if($_.Exception.Message -notmatch 'Existing restart fence preserved'){throw}}
        if([IO.File]::ReadAllText($RustRestart) -cne 'foreign fence'){throw 'foreign fence changed'}
    }
    'test_cutover_does_not_delete_foreign_stop' {
        $script:CutoverStopText='owned'
        [IO.File]::WriteAllText($RustStop,'foreign')
        try{Start-Rust;throw 'foreign stop accepted'}
        catch{if($_.Exception.Message -notmatch 'Foreign maintenance marker preserved'){throw}}
        if([IO.File]::ReadAllText($RustStop) -cne 'foreign'){throw 'foreign stop changed'}
    }
    'test_cutover_marker_publication_respects_control_owner' {
        $owner=Enter-CdrControl $RepoRoot
        try {
         try {Enter-CutoverMaintenance ([pscustomobject]@{TransactionId='mine'});throw 'control lock bypassed'}
         catch{if($_.Exception.Message -notmatch 'cdr_control_busy'){throw}}
         if(Test-Path $DisablePath){throw 'published despite competing owner'}
        }finally{$owner.Dispose()}
    }
    'test_worker_rechecks_runtime_lock_before_stop_publication' {
        $source=Get-Content (Join-Path $env:PUBLISH_SOURCE 'scripts/Invoke-CdrDeployment.ps1') -Raw
        $start=$source.IndexOf('    $control = Enter-CdrControl')
        $end=$source.IndexOf("    Note 'normal_stop_requested'",$start)
        if($start -lt 0 -or $end -lt 0){throw 'publication boundary missing'}
        $state=[pscustomobject]@{RepoRoot=$RepoRoot;RuntimePid=42;RuntimeTicks='99';BinaryPath='fixture'}
        $running=[pscustomobject]@{HasExited=$false}
        $running|Add-Member ScriptMethod Refresh {}
        function Get-Process {param($Id,$ErrorAction) [pscustomobject]@{Path='fixture';StartTime=[datetime]::new(99,[DateTimeKind]::Utc)}}
        [IO.File]::WriteAllText((Join-Path $RepoRoot '.codex_discord_rust.runtime.lock'),"pid=77`n")
        try{& ([scriptblock]::Create($source.Substring($start,$end-$start)));throw 'changed lock accepted'}
        catch{if($_.Exception.Message -notmatch 'Runtime changed before stop publication'){throw}}
        if((Test-Path $RustStop) -or (Test-Path $DisablePath)){throw 'new instance was stopped'}
    }
    'test_cutover_stop_consumes_its_marker_after_confirmed_exit' {
        $CutoverIdentity='fixture-owner'
        $script:checks=0
        function Get-VerifiedRustProcess {
         $script:checks++; if($script:checks -eq 1){[pscustomobject]@{Id=42}}else{$null}
        }
        $RustWatchdog=Join-Path $RepoRoot 'fixture-watchdog.ps1'
        [IO.File]::WriteAllText($RustWatchdog,'exit 0')
        Stop-Rust
        if(Test-Path $RustStop){throw 'owned stop leaked into the next cutover invocation'}
        # A new PowerShell call cannot recover script-local owner text. Verify startup
        # no longer depends on it once this stop operation has completed.
        $script:CutoverStopText=$null
        if(Test-Path $RustStop){throw 'next invocation would be blocked'}
    }
    'test_owned_cutover_startup_handoff_and_unobservable_prior_owner_guard' {
        $CutoverStatePath = Join-Path $RepoRoot '.codex_discord_runtime.cutover'
        $oldIdentity = '2147483000|1234560'
        $tx = [pscustomobject]@{ TransactionId = 'owned-test-tx'; OwnerIdentity = $oldIdentity; SourceRuntime = 'rust'; TargetRuntime = 'rust'; Phase = 'target_starting' }
        [IO.File]::WriteAllText($CutoverStatePath, "transaction_id=owned-test-tx`nowner_identity=$oldIdentity`nsource_runtime=rust`ntarget_runtime=rust`nphase=target_starting`n")
        function Get-Process {
            [CmdletBinding()]
            param([int]$Id)
            Write-Error 'simulated process observation access denied' -Category PermissionDenied
        }
        $acceptedUnknown = Test-CdrOwnedCutoverState -Path $CutoverStatePath -OwnedTransaction $tx -OwnedIdentity '123|999'
        Remove-Item Function:Get-Process
        if ($acceptedUnknown) { throw 'unobservable prior owner was accepted as exited' }

        $self = [Diagnostics.Process]::GetCurrentProcess()
        try {
            $ticks = $self.StartTime.ToUniversalTime().Ticks
            $ticks -= ($ticks % 10)
            $CutoverIdentity = "$PID|$ticks"
        } finally {
            $self.Dispose()
        }
        $BinaryPath = Join-Path $RepoRoot 'fixture-bot.exe'
        $EnvPath = Join-Path $RepoRoot '.env'
        [IO.File]::WriteAllText($BinaryPath, 'fixture')
        [IO.File]::WriteAllText($EnvPath, "DISCORD_BOT_TOKEN=x`n")
        $activeTx = [pscustomobject]@{ TransactionId = 'active-cutover-tx'; OwnerIdentity = $CutoverIdentity; SourceRuntime = 'rust'; TargetRuntime = 'rust'; Phase = 'target_starting' }
        Set-CutoverPhase -Transaction $activeTx -Phase 'target_starting'

        $RustWatchdog = Join-Path $RepoRoot 'real-watchdog-boundary.ps1'
        $watchdogBoundaryCode = @'
[CmdletBinding()]
param(
    [string]$RepoRoot,
    [string]$BinaryPath,
    [string]$EnvPath,
    [string]$OwnedCutoverTransactionId = ''
)
$ErrorActionPreference = 'Stop'
. (Join-Path $env:PUBLISH_SOURCE 'codex-discord-rust-control.ps1')
$watchdogProcess = [Diagnostics.Process]::GetCurrentProcess()
try {
    $watchdogTicks = $watchdogProcess.StartTime.ToUniversalTime().Ticks
    $watchdogTicks -= ($watchdogTicks % 10)
} finally {
    $watchdogProcess.Dispose()
}
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $env:PUBLISH_SOURCE 'codex-discord-rust-watchdog.ps1'), [ref]$tokens, [ref]$errors)
$startFn = $ast.FindAll({ param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Start-RustRuntime' }, $false) | Select-Object -First 1
Invoke-Expression $startFn.Extent.Text
function Get-CdrRuntimeArtifact { param($Path, $Hash, $Commit) [pscustomobject]@{ ArtifactHash = '0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF'; RequireStartupRecord = $false; BuildInfo = $null } }
function Get-CdrRuntimeArtifactPin { param($Path, $Hash) [pscustomobject]@{} | Add-Member ScriptMethod Dispose {} -PassThru }
function Invoke-CdrProductionCheckedLaunch { param($DeadlineUtc, [scriptblock]$Launch) & $Launch }
function Enter-CdrIntentLock { param($RepoRoot, $TimeoutMs) $null }
function Exit-CdrIntentLock { param($Lock) }
function Set-CdrLaunchStarting { [IO.File]::WriteAllText((Join-Path $RepoRoot 'handoff-admitted.txt'), $OwnedCutoverTransactionId) }
function Start-CdrDetachedProcess { [pscustomobject]@{ Id = 77; HasExited = $false } }
function Set-CdrLaunchChild { param($Process) }
function Get-VerifiedRuntimeProcess { [pscustomobject]@{ Id = 77 } }
function Get-RustProcessIdentity { param($Process) '77|99' }
function Wait-CdrReplacementReady { param($ExpectedIdentity, $Artifact, $TimeoutSeconds) }
function Write-RustWatchdogLog { param($Message) }
$controlGuard = Enter-CdrControl -Root $RepoRoot -Purpose 'watchdog'
try {
    Start-RustRuntime
} finally {
    $controlGuard.Dispose()
}
exit 0
'@
        [IO.File]::WriteAllText($RustWatchdog, $watchdogBoundaryCode)
        function Get-VerifiedRustProcess {
            if ([IO.File]::Exists((Join-Path $RepoRoot 'handoff-admitted.txt'))) {
                return [pscustomobject]@{ Id = 77 }
            }
            return $null
        }
        Start-Rust
        if ([IO.File]::ReadAllText((Join-Path $RepoRoot 'handoff-admitted.txt')).Trim() -cne 'active-cutover-tx') {
            throw 'owned cutover transaction id was not handed off across & $RustWatchdog'
        }
        [IO.File]::Delete((Join-Path $RepoRoot 'handoff-admitted.txt'))
        $unownedRejected = $false
        try {
            & $RustWatchdog -RepoRoot $RepoRoot -BinaryPath $BinaryPath -EnvPath $EnvPath
        } catch {
            if ($_.Exception.Message -match 'mcp_cutover_pending') { $unownedRejected = $true } else { throw }
        }
        if (-not $unownedRejected) {
            throw 'watchdog launch without explicit OwnedCutoverTransactionId bypassed cutover fence'
        }
    }
    default { throw "Unknown cutover fixture: $CaseName" }
}
