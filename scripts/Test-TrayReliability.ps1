[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)]
    [ValidateSet('baseline','prepare','focused','maintenance-core','maintenance-native','maintenance-other','runtime','install','quality','runtime-lib','store','formatted-contracts')]
    [string]$Group,
    [string]$RepoRoot,
    [string]$EvidenceDirectory
)
$ErrorActionPreference='Stop'
if ([string]::IsNullOrWhiteSpace($RepoRoot)) { $RepoRoot=Join-Path $PSScriptRoot '..' }
$RepoRoot=[IO.Path]::GetFullPath($RepoRoot)
if ([string]::IsNullOrWhiteSpace($EvidenceDirectory)) { $EvidenceDirectory=Join-Path $RepoRoot 'target/qa/tray-reliability-r4-20260917' }
$EvidenceDirectory=[IO.Path]::GetFullPath($EvidenceDirectory)
[void][IO.Directory]::CreateDirectory($EvidenceDirectory)
$utf8=[Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding=$utf8

function Get-SourceSnapshot {
    $extensions=@('.rs','.sql','.ps1','.psm1','.vbs','.sh','.toml','.json','.cmd','.bat')
    $files=@(Get-ChildItem (Join-Path $RepoRoot 'crates'),(Join-Path $RepoRoot 'scripts') -Recurse -File | Where-Object {$_.Extension -in $extensions})
    $files+=@(Get-ChildItem $RepoRoot -File | Where-Object {$_.Extension -in $extensions -or $_.Name -eq 'Cargo.lock'})
    foreach($path in @('.agents/plugins/marketplace.json','plugins/codex-discord-remote/.codex-plugin/plugin.json')) { $files+=Get-Item (Join-Path $RepoRoot $path) }
    foreach($file in ($files | Sort-Object FullName -Unique)) {
        [pscustomobject]@{path=$file.FullName.Substring($RepoRoot.Length+1).Replace('\','/');sha256=(Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
    }
}
function Save-Json([string]$Name,$Value) {
    [IO.File]::WriteAllText((Join-Path $EvidenceDirectory $Name),($Value|ConvertTo-Json -Depth 12),$utf8)
}
function Invoke-RecordedNative([string]$Tag,[string]$Program,[string]$Arguments,[int]$TimeoutSeconds=220) {
    $receiptPath=Join-Path $EvidenceDirectory ($Tag+'.json')
    if(Test-Path -LiteralPath $receiptPath) { throw "Evidence already exists: $receiptPath; use a new evidence directory for a rerun." }
    $before=@(Get-SourceSnapshot)
    Save-Json ($Tag+'.source-before.json') $before
    $info=[Diagnostics.ProcessStartInfo]::new()
    $info.FileName=(Get-Command $Program -ErrorAction Stop).Source
    $info.Arguments=$Arguments
    $info.WorkingDirectory=$RepoRoot
    $info.UseShellExecute=$false
    $info.CreateNoWindow=$true
    $info.RedirectStandardOutput=$true
    $info.RedirectStandardError=$true
    $info.StandardOutputEncoding=$utf8
    $info.StandardErrorEncoding=$utf8
    # Do not weaken product environment checks to accommodate cmd.exe drive state.
    $removed=@($info.EnvironmentVariables.Keys | Where-Object {([string]$_).Contains('=')})
    foreach($key in $removed) { $info.EnvironmentVariables.Remove($key) }
    $process=[Diagnostics.Process]::new()
    $process.StartInfo=$info
    $watch=[Diagnostics.Stopwatch]::StartNew()
    try {
        if(-not $process.Start()) { throw "Could not start $Program" }
        $stdout=$process.StandardOutput.ReadToEndAsync()
        $stderr=$process.StandardError.ReadToEndAsync()
        $timedOut=-not $process.WaitForExit($TimeoutSeconds*1000)
        if($timedOut) { $process.Kill(); $process.WaitForExit() }
        $out=$stdout.GetAwaiter().GetResult()
        $err=$stderr.GetAwaiter().GetResult()
        $code=$process.ExitCode
        [IO.File]::WriteAllText((Join-Path $EvidenceDirectory ($Tag+'.stdout.log')),$out,$utf8)
        [IO.File]::WriteAllText((Join-Path $EvidenceDirectory ($Tag+'.stderr.log')),$err,$utf8)
        $after=@(Get-SourceSnapshot)
        Save-Json ($Tag+'.source-after.json') $after
        $difference=@(Compare-Object $before $after -Property path,sha256)
        $receipt=[ordered]@{tag=$Tag;program=$info.FileName;arguments=$Arguments;exit_code=$code;timed_out=$timedOut;elapsed_ms=$watch.ElapsedMilliseconds;removed_invalid_env_key_count=$removed.Count;source_count=$after.Count;source_unchanged=($difference.Count -eq 0);source_difference=$difference;finished_at=[DateTimeOffset]::Now.ToString('o')}
        Save-Json ($Tag+'.json') $receipt
        Write-Output ($receipt|ConvertTo-Json -Depth 12 -Compress)
        $out -split "`n" | Where-Object {$_ -match 'running \d+ tests|test result:|FAILED|failures:|rustfmt_workspace_complete'} | Write-Output
        if($timedOut -or $code -ne 0 -or $difference.Count -ne 0) {
            Write-Output $err
            Write-Output (($out -split "`n"|Select-Object -Last 45)-join "`n")
            throw "QA check failed: $Tag (native=$code, timeout=$timedOut, source_difference=$($difference.Count))"
        }
    } finally { $watch.Stop(); $process.Dispose() }
}
function Prepare-Installer {
    Invoke-RecordedNative 'prepare' 'cargo.exe' 'build --offline --locked --target-dir target -p cdr-runtime -p cdr-pro --bins'
    $binaries=@()
    foreach($name in @('cdr-runtime.exe','cdr-offline-soak.exe','cdr-pro-helper.exe')) {
        $path=Join-Path $RepoRoot ('target/debug/'+$name)
        if(-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Required QA binary missing: $path" }
        $binaries+=[pscustomobject]@{path=$path;sha256=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()}
    }
    Save-Json 'prepared-binaries.json' $binaries
}
$sets=@{
    'formatted-contracts'=@('completion_message_contract','final_watch_contract','luna_reserve_auto_switch_contract','command_plan_contract')
    'baseline'=@('install_codex_persistence_contract','install_profile_contract','install_profile_location_contract','python_free_installer_contract','maintenance_native_contract')
    'focused'=@('windows_tray_contract','windows_tray_reliability_contract','windows_tray_revision3_contract','windows_powershell_utf8_contract','windows_qa_formatting_contract')
    'maintenance-core'=@('maintenance_ownership_contract','maintenance_readiness_contract','maintenance_operator_pins_contract','maintenance_mutex_contract')
    'maintenance-native'=@('maintenance_native_contract')
    'maintenance-other'=@('maintenance_backup_native_contract','maintenance_completion_native_contract','maintenance_diagnostics_contract','maintenance_entry_native_contract','maintenance_reconcile_native_contract','maintenance_snapshot_native_contract')
    'runtime'=@('windows_watchdog_contract','restart_native_contract','restart_notification_native_contract','restart_boundary_contract','deployment_recovery_native_contract','recovery_safety_native_contract','restart_readiness_contract')
    'install'=@('installer_staging_contract','install_codex_persistence_contract','install_profile_contract','install_profile_location_contract','python_free_installer_contract','python_free_operations_contract','shell_wrapper_contract')
}
if($Group -eq 'prepare') { Prepare-Installer }
elseif($Group -eq 'runtime-lib') {
    Invoke-RecordedNative 'runtime-lib' 'cargo.exe' 'test --offline --locked --target-dir target -p cdr-runtime --lib -- --test-threads=2'
}
elseif($Group -eq 'store') {
    Invoke-RecordedNative 'store' 'cargo.exe' 'test --offline --locked --no-fail-fast --target-dir target -p cdr-store -- --test-threads=2'
}
elseif($Group -eq 'quality') {
    Invoke-RecordedNative 'clippy' 'cargo.exe' 'clippy --offline --locked --target-dir target --workspace --all-targets -- -D warnings'
    $formatScript=Join-Path $RepoRoot 'scripts/Test-RustFormatting.ps1'
    Invoke-RecordedNative 'format' 'powershell.exe' ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "'+$formatScript+'" -RepoRoot "'+$RepoRoot+'"')
    Invoke-RecordedNative 'diff-check' 'git.exe' 'diff --check'
} else {
    if($Group -eq 'install') { Prepare-Installer }
    $targets=($sets[$Group]|ForEach-Object {'--test '+$_}) -join ' '
    Invoke-RecordedNative $Group 'cargo.exe' ('test --offline --locked --no-fail-fast --target-dir target -p cdr-runtime '+$targets+' -- --test-threads=1')
}
exit 0
