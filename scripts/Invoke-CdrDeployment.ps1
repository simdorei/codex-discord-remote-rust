param([Parameter(Mandatory=$true)][string]$StatePath)
$ErrorActionPreference = 'Stop'
$state = Get-Content -LiteralPath $StatePath -Raw -Encoding UTF8 | ConvertFrom-Json
if ($null -ne $state.Version -and $state.Version -ne 1) {
    throw 'maintenance_v2_or_unknown: legacy deployment refused'
}
. (Join-Path $state.RepoRoot 'codex-discord-rust-control.ps1')
$lock = [IO.File]::Open($state.LockPath, 'OpenOrCreate', 'ReadWrite', 'None')
$failed = $false
function Note([string]$message) {
    Add-Content -LiteralPath $state.LogPath -Encoding UTF8 -Value "$(Get-Date -Format o) $message"
}
function Hash([string]$path) { Get-CdrArtifactHash -Path $path }
try {
    Set-Location -LiteralPath $state.RepoRoot
    Note "worker_started pid=$PID"
    if ((Hash $state.BinaryPath) -ne $state.BaselineHash) { throw 'Baseline changed' }
    if ((Hash $state.CandidatePath) -ne $state.CandidateHash) { throw 'Candidate changed' }
    $running = Get-Process -Id $state.RuntimePid -ErrorAction Stop
    if ($running.Path -ne $state.BinaryPath -or
        $running.StartTime.ToUniversalTime().Ticks.ToString() -ne $state.RuntimeTicks) {
        throw 'Runtime identity changed'
    }
    $disabled = Join-Path $state.RepoRoot '.codex_discord_bot.disabled'
    $stop = Join-Path $state.RepoRoot '.codex_discord_rust.stop'
    foreach ($name in @('.codex_discord_bot.disabled', '.codex_discord_rust.stop',
        '.codex_discord_rust.restart', '.codex_discord_rust.drain.prepare')) {
        if (Test-Path -LiteralPath (Join-Path $state.RepoRoot $name)) { throw 'Existing maintenance marker' }
    }
    Copy-Item -LiteralPath $state.BinaryPath -Destination $state.BackupPath -ErrorAction Stop
    if ((Hash $state.BackupPath) -ne $state.BaselineHash) { throw 'Backup mismatch' }
    $control = Enter-CdrControl -Root $state.RepoRoot
    try {
        Assert-CdrNoPendingRestart -Root $state.RepoRoot
        if (Test-Path -LiteralPath (Join-Path $state.RepoRoot '.codex_discord_runtime.cutover')) {
            throw 'Pending cutover operation preserved; deployment refused'
        }
        $running.Refresh()
        $current = Get-Process -Id $state.RuntimePid -ErrorAction Stop
        $lockText = [IO.File]::ReadAllText((Join-Path $state.RepoRoot '.codex_discord_rust.runtime.lock'))
        if ($running.HasExited -or $current.Path -cne $state.BinaryPath -or
            $current.StartTime.ToUniversalTime().Ticks.ToString() -cne [string]$state.RuntimeTicks -or
            $lockText -notmatch '(?m)^pid=(\d+)\r?$' -or [int]$Matches[1] -ne $state.RuntimePid) {
            throw 'Runtime changed before stop publication; no stop requested'
        }
        foreach ($name in @('.codex_discord_bot.disabled', '.codex_discord_rust.stop',
            '.codex_discord_rust.restart', '.codex_discord_rust.drain.prepare')) {
            if (Test-Path -LiteralPath (Join-Path $state.RepoRoot $name)) {
                throw 'Existing maintenance marker appeared before publication'
            }
        }
        Write-NewCdrMarker -Path $disabled -Text $state.Marker
        Write-NewCdrMarker -Path $stop -Text $state.Marker
    } finally { $control.Dispose() }
    Note 'normal_stop_requested'
    $deadline = (Get-Date).AddSeconds(45)
    while (-not $running.HasExited -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 500
        $running.Refresh()
    }
    if (-not $running.HasExited) { throw 'Normal shutdown timeout; no force kill' }
    # Retain owned maintenance markers throughout packaging; only dedicated
    # identity-bound recovery may consume them and start a replacement.
    Note 'old_runtime_stopped'
    & $state.CandidatePath --backup-store --env (Join-Path $state.RepoRoot '.env') >> $state.LogPath 2>&1
    if ($LASTEXITCODE -ne 0) { throw 'SQLite snapshot failed' }
    $env:CARGO_PROFILE_TEST_DEBUG = '0'
    $env:CARGO_PROFILE_DEV_DEBUG = '0'
    $env:PATH = $state.ToolPath + ';' + $env:PATH
    & $state.CargoPath test --locked --offline -p cdr-runtime --test windows_release_checkpoint_contract --test windows_release_checkpoint_evidence_integrity_contract --test windows_release_checkpoint_rollback_completeness_contract --no-fail-fast --quiet >> $state.TestLogPath 2>&1
    if ($LASTEXITCODE -ne 0) { throw 'Packaging gate failed; baseline retained' }
    Note 'packaging_gates_passed'
    if (Get-Process -Name cdr-runtime -ErrorAction SilentlyContinue) { throw 'Runtime appeared during maintenance' }
    if ((Hash $state.CandidatePath) -ne $state.CandidateHash) { throw 'Candidate changed during tests' }
    if (Test-Path -LiteralPath $state.NextPath) { throw 'Unexpected staging binary' }
    Copy-Item -LiteralPath $state.CandidatePath -Destination $state.NextPath
    if ((Hash $state.NextPath) -ne $state.CandidateHash) { throw 'Staged artifact mismatch' }
    [IO.File]::Replace($state.NextPath, $state.BinaryPath, $state.ReplaceBackupPath)
    if ((Hash $state.BinaryPath) -ne $state.CandidateHash) { throw 'Installed artifact mismatch' }
    Note "candidate_installed sha256=$($state.CandidateHash)"
} catch {
    $failed = $true
    Note "FAILED: $($_.Exception.Message)"
} finally {
    $lock.Dispose()
    # Recovery writes its own log. Redirecting it to that same file holds the
    # destination open and can block its Add-Content call after the bot starts.
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'Recover-CdrDeployment.ps1') -StatePath $StatePath
    if ($LASTEXITCODE -ne 0) { $failed = $true; Note 'recovery_failed; independent scheduled recovery remains armed' }
}
if ($failed) { exit 1 }
Note 'deployment_complete_pending_live_QA'
