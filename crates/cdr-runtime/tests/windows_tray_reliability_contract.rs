#![cfg(windows)]
//! Offline tray faults and bootstrap admission. Never launches the real tray/bot.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn run(body: &str) -> Output {
    let temp = tempfile::tempdir().unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in [
        "codex-discord-tray.ps1",
        "codex-discord-tray-runtime.ps1",
        "codex-discord-tray-restart-runtime.ps1",
    ] {
        fs::copy(repo.join(name), temp.path().join(name)).unwrap();
    }
    fs::write(
        temp.path().join(".codex_discord_rust.runtime.lock"),
        "pid=42\n",
    )
    .unwrap();
    let prelude = r"
$ErrorActionPreference='Stop'
$ScriptDir=$env:TRAY_CONTRACT_ROOT
$RuntimeMode='rust'
$LauncherLogPath=Join-Path $ScriptDir 'launcher.log'
. (Join-Path $ScriptDir 'codex-discord-tray-runtime.ps1')
$tokens=$null; $errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $ScriptDir 'codex-discord-tray.ps1'),[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'TRAY_PARSE_ERROR'}
foreach($name in @('Write-LauncherLog','Limit-TrayText','Write-TrayStatusError','Update-TrayStatus')) {
    $f=$ast.FindAll({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -ceq $name},$true)|Select-Object -First 1
    if($f){. ([scriptblock]::Create($f.Extent.Text))}
}
$script:MissingSince=$null; $script:LastRunningStatus=$null; $script:LastTrayIcon=$null
$script:LastStatusError=$null; $script:LastStatusErrorAt=[datetime]::MinValue
$StoppedGraceSeconds=30
$script:spawnCount=0
function Test-CdrTrayInteractiveSession {return $true}
function Get-RustTrayProcess { [pscustomobject]@{ProcessId=42;StartTime=[datetime]'2026-09-01T00:00:00Z'} }
function Get-CdrTrayMutexBusy { param($Name); return $false }
function Start-Process {
    [CmdletBinding()]param($FilePath,$ArgumentList,$WorkingDirectory,$WindowStyle,[switch]$PassThru)
    $script:spawnCount++; $script:spawnFile=$FilePath; $script:spawnArgs=$ArgumentList; $script:spawnRoot=$WorkingDirectory
    [pscustomobject]@{Id=91}
}
$identity=Get-RustTrayProcessIdentity (Get-RustTrayProcess)
";
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-STA",
            "-Command",
            &format!("{prelude}\n{body}"),
        ])
        .env("TRAY_CONTRACT_ROOT", temp.path())
        .env("CODEX_DISCORD_RUNTIME", "rust")
        .output()
        .unwrap()
}

fn pass(body: &str) {
    let output = run(body);
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn once_read_failure_is_unknown_not_stopped() {
    let output = run(r"
function Get-Content {
    [CmdletBinding()]param($LiteralPath,[switch]$Raw,$Encoding)
    if($LiteralPath -like '*.runtime.lock'){throw 'INJECTED_LOCK_READ_FAILURE'}
    Microsoft.PowerShell.Management\Get-Content @PSBoundParameters
}
& (Join-Path $ScriptDir 'codex-discord-tray.ps1') -Once
# -Command otherwise normalizes any nonzero child script exit to 1.
exit $LASTEXITCODE
");
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("unknown"));
}

#[test]
fn observation_error_recovers_without_reusing_old_pid() {
    pass(
        r"
$script:broken=$true
function Get-BotProcess {if($script:broken){throw 'INJECTED_READ'};return (Get-RustTrayProcess)}
$a=Get-BotStatus
if($a.State -ne 'unknown' -or $a.Running -or $null -ne $a.Pid){throw 'ERROR_IS_NOT_UNKNOWN'}
$script:broken=$false
$b=Get-BotStatus
if($b.State -ne 'running' -or $b.Pid -ne 42){throw 'DID_NOT_RECOVER'}
",
    );
}

#[test]
fn log_write_failure_does_not_escape() {
    pass(
        r"
$LauncherLogPath=$ScriptDir
Write-LauncherLog 'INJECTED_LOG_WRITE_FAILURE'
",
    );
}

#[test]
fn ui_tick_and_error_logging_failures_are_contained() {
    pass(
        r"
Add-Type -AssemblyName System.Drawing
$LauncherLogPath=$ScriptDir
$statusItem=New-Object PSObject
$statusItem|Add-Member ScriptProperty Text {''} {throw 'INJECTED_UI_WRITE_FAILURE'}
$notify=[pscustomobject]@{Text='';Icon=$null}
function Get-BotStatus {[pscustomobject]@{State='running';Running=$true;Pid=42;Text='running';Icon='Application'}}
Update-TrayStatus
Update-TrayStatus
",
    );
}

#[test]
fn unknown_status_does_not_use_restart_grace_or_cached_health() {
    pass(
        r"
Add-Type -AssemblyName System.Drawing
$statusItem=[pscustomobject]@{Text=''}; $notify=[pscustomobject]@{Text='';Icon=$null}
$script:LastRunningStatus=[pscustomobject]@{Running=$true;Pid=42}
function Get-BotStatus {[pscustomobject]@{State='unknown';Running=$false;Pid=$null;Text='Codex Discord bridge status unavailable';Icon='Warning';Diagnostic='injected'}}
Update-TrayStatus
if($statusItem.Text -notlike '*unavailable*' -or $null -ne $script:LastRunningStatus -or $null -ne $script:MissingSince){throw 'STALE_HEALTH_REUSED'}
",
    );
}

#[test]
fn bootstrap_starts_only_sta_tray_for_exact_runtime() {
    pass(
        r"
$r=Start-CdrTrayForRuntime -Root $ScriptDir -ExpectedRuntimeIdentity $identity
if($r.State -ne 'spawn_requested' -or $script:spawnCount -ne 1){throw 'SPAWN_COUNT'}
if($script:spawnFile -notlike '*WindowsPowerShell\v1.0\powershell.exe' -or $script:spawnRoot -cne $ScriptDir){throw 'SPAWN_EXECUTABLE_OR_ROOT'}
if($script:spawnArgs -notcontains '-STA' -or ($script:spawnArgs -join ' ') -notlike '*codex-discord-tray.ps1*'){throw 'NOT_STA_TRAY'}
if(($script:spawnArgs -join ' ') -match 'headless|watchdog|restart|cdr-runtime.exe'){throw 'BOT_CONTROL_SPAWN'}
",
    );
}

#[test]
fn bootstrap_skips_noninteractive_mismatched_or_existing_owner() {
    pass(
        r"
function Test-CdrTrayInteractiveSession {return $false}
$r=Start-CdrTrayForRuntime -Root $ScriptDir -ExpectedRuntimeIdentity $identity
if($r.State -ne 'noninteractive' -or $script:spawnCount){throw 'NONINTERACTIVE_SPAWN'}
function Test-CdrTrayInteractiveSession {return $true}
$r=Start-CdrTrayForRuntime -Root $ScriptDir -ExpectedRuntimeIdentity '43|1'
if($r.State -ne 'runtime_mismatch' -or $script:spawnCount){throw 'MISMATCH_SPAWN'}
function Get-CdrTrayMutexBusy {param($Name);return $true}
$r=Start-CdrTrayForRuntime -Root $ScriptDir -ExpectedRuntimeIdentity $identity
if($r.State -ne 'owner_present' -or $script:spawnCount){throw 'DUPLICATE_SPAWN'}
",
    );
}

#[test]
fn bootstrap_rechecks_identity_after_mutex_probe() {
    pass(
        r"
$script:reads=0
function Get-RustTrayProcess {$script:reads++;$id=if($script:reads -eq 1){42}else{43};[pscustomobject]@{ProcessId=$id;StartTime=[datetime]'2026-09-01T00:00:00Z'}}
$r=Start-CdrTrayForRuntime -Root $ScriptDir -ExpectedRuntimeIdentity $identity
if($r.State -ne 'runtime_mismatch' -or $script:spawnCount){throw 'CHANGED_RUNTIME_SPAWNED'}
",
    );
}

#[test]
fn uncertain_spawn_is_not_retried_or_reported_as_healthy() {
    pass(
        r"
function Start-Process {[CmdletBinding()]param($FilePath,$ArgumentList,$WorkingDirectory,$WindowStyle,[switch]$PassThru);$script:spawnCount++;throw 'INJECTED_UNKNOWN_SPAWN'}
$r=Start-CdrTrayForRuntime -Root $ScriptDir -ExpectedRuntimeIdentity $identity
if($r.State -ne 'unknown' -or $script:spawnCount -ne 1){throw 'UNKNOWN_RETRIED_OR_MASKED'}
",
    );
}

#[test]
fn bootstrap_is_wired_after_exact_lock_and_all_tray_scripts_are_pinned() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = fs::read_to_string(root.join("codex-discord-rust-watchdog.ps1")).unwrap();
    let function = source
        .split("function Start-RustRuntime {")
        .nth(1)
        .unwrap()
        .split(" CONTROL_ENTRY")
        .next()
        .unwrap();
    assert_eq!(function.matches("Start-CdrTrayForRuntime").count(), 0);
    assert!(
        function.find("[int]$verified.Id -eq $process.Id").unwrap()
            < function
                .rfind("$script:CdrTrayStartedIdentity = Get-RustProcessIdentity")
                .unwrap()
    );
    let pins = fs::read_to_string(root.join("scripts/CdrMaintenanceState.ps1")).unwrap();
    for name in [
        "codex-discord-tray.ps1",
        "codex-discord-tray-runtime.ps1",
        "codex-discord-tray-restart-runtime.ps1",
    ] {
        assert!(pins.contains(name), "missing pin {name}");
    }
}
#[test]
fn ui_recovery_is_reported_once_only_after_successful_publication() {
    pass(
        r"
Add-Type -AssemblyName System.Drawing
$script:messages=@(); $script:uiBroken=$true; $script:publishedText=''
function Write-LauncherLog {param([string]$Message); $script:messages+=,$Message}
$statusItem=New-Object PSObject
$statusItem|Add-Member ScriptProperty Text {$script:publishedText} {param($value);if($script:uiBroken){throw 'INJECTED_UI_FAILURE'};$script:publishedText=$value}
$notify=[pscustomobject]@{Text='';Icon=$null}
function Get-BotStatus {[pscustomobject]@{State='running';Running=$true;Pid=42;Text='running pid=42';Icon='Application'}}
Update-TrayStatus
Update-TrayStatus
if(@($script:messages|Where-Object {$_ -eq 'tray_status_recovered'}).Count -ne 0 -or $null -eq $script:LastStatusError){throw 'FALSE_UI_RECOVERY'}
$script:uiBroken=$false
Update-TrayStatus
Update-TrayStatus
if(@($script:messages|Where-Object {$_ -eq 'tray_status_recovered'}).Count -ne 1 -or $null -ne $script:LastStatusError){throw 'REAL_RECOVERY_NOT_EXACTLY_ONCE'}
if($script:publishedText -ne 'running pid=42' -or $notify.Text -ne 'running pid=42' -or $script:spawnCount){throw 'RECOVERY_NOT_PUBLISHED_OR_SPAWNED'}
",
    );
}

#[test]
fn absent_or_invalid_spawn_pid_is_unknown_without_retry() {
    pass(
        r"
$variants=@($null,[pscustomobject]@{Id=0},[pscustomobject]@{Id=-1},[pscustomobject]@{Id='not-a-pid'},[pscustomobject]@{NoId=91})
function Start-Process {
    [CmdletBinding()]param($FilePath,$ArgumentList,$WorkingDirectory,$WindowStyle,[switch]$PassThru)
    $script:spawnCount++;return $script:spawnResult
}
$cases=0
foreach($variant in $variants){
    $cases++;$script:spawnResult=$variant;$script:spawnCount=0
    $result=Start-CdrTrayForRuntime -Root $ScriptDir -ExpectedRuntimeIdentity $identity
    if($result.State -ne 'unknown' -or $null -ne $result.Pid -or $script:spawnCount -ne 1){throw 'INVALID_SPAWN_PID_ACCEPTED_OR_RETRIED'}
}
if($cases -ne 5){throw 'SPAWN_VARIANT_NOT_EXECUTED'}
",
    );
}

#[test]
fn real_bootstrap_unknown_spawn_consumes_the_single_attempt() {
    pass(
        r"
function Start-Process {
    [CmdletBinding()]param($FilePath,$ArgumentList,$WorkingDirectory,$WindowStyle,[switch]$PassThru)
    $script:spawnCount++;throw 'INJECTED_POST_SPAWN_UNCERTAINTY'
}
$first=Invoke-CdrTrayAfterWatchdog -Root $ScriptDir -StartedIdentity $identity -ObservedIdentity ''
$second=Invoke-CdrTrayAfterWatchdog -Root $ScriptDir -StartedIdentity $identity -ObservedIdentity ''
if($first.State -ne 'unknown' -or $null -ne $first.Pid -or $second.State -ne 'already_attempted' -or $script:spawnCount -ne 1){throw 'UNKNOWN_SPAWN_ATTEMPT_REPLAYED'}
$claims=@(Get-ChildItem -LiteralPath $ScriptDir -Force -Filter '.codex_discord_tray.bootstrap.*.attempted')
if($claims.Count -ne 1){throw 'DURABLE_ATTEMPT_MISSING'}
",
    );
}

#[test]
fn unknown_ui_ticks_throttle_logs_and_never_reuse_cached_health() {
    pass(
        r"
Add-Type -AssemblyName System.Drawing
$script:messages=@();$script:observationBroken=$false;$script:observedPid=42
function Write-LauncherLog {param([string]$Message);$script:messages+=,$Message}
function Get-BotStatus {
    if($script:observationBroken){return [pscustomobject]@{State='unknown';Running=$false;Pid=$null;Text='status unavailable';Icon='Warning';Diagnostic='read_failed'}}
    return [pscustomobject]@{State='running';Running=$true;Pid=$script:observedPid;Text=('running pid='+$script:observedPid);Icon='Application'}
}
$statusItem=[pscustomobject]@{Text=''};$notify=[pscustomobject]@{Text='';Icon=$null}
Update-TrayStatus
$script:observationBroken=$true
1..20|ForEach-Object {Update-TrayStatus}
if(@($script:messages|Where-Object {$_ -like 'tray_status_unavailable *'}).Count -ne 1){throw 'UNBOUNDED_STATUS_LOGS'}
if($null -ne $script:LastRunningStatus -or $null -ne $script:MissingSince -or $statusItem.Text -ne 'status unavailable'){throw 'UNKNOWN_REUSED_CACHED_HEALTH'}
$script:LastStatusErrorAt=[datetime]::UtcNow.AddSeconds(-61)
Update-TrayStatus
if(@($script:messages|Where-Object {$_ -like 'tray_status_unavailable *'}).Count -ne 2 -or @($script:messages|Where-Object {$_ -eq 'tray_status_recovered'}).Count){throw 'LOG_THROTTLE_OR_FALSE_RECOVERY'}
$script:observationBroken=$false;$script:observedPid=43
Update-TrayStatus
Update-TrayStatus
if(@($script:messages|Where-Object {$_ -eq 'tray_status_recovered'}).Count -ne 1 -or $script:LastRunningStatus.Pid -ne 43 -or $notify.Text -ne 'running pid=43' -or $script:spawnCount){throw 'RECOVERY_REUSED_OLD_PID_OR_SPAWNED'}
",
    );
}
