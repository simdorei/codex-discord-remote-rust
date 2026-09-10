[CmdletBinding()]
param([Parameter(Mandatory)][string]$StatePath,
    [ValidateSet('Worker','Recovery','Inspect')][string]$Mode = 'Inspect')
$ErrorActionPreference = 'Stop'
$state = Get-Content -LiteralPath $StatePath -Raw -Encoding UTF8 | ConvertFrom-Json
$root = [IO.Path]::GetFullPath($state.RepoRoot)
$folder = Split-Path -Parent ([IO.Path]::GetFullPath($StatePath))
$binary = Join-Path $root 'target/release/cdr-runtime.exe'
$watchdog = Join-Path $root 'codex-discord-rust-watchdog.ps1'
. (Join-Path $root 'codex-discord-rust-drain.ps1')
. (Join-Path $root 'codex-discord-rust-control.ps1')
$resultPath = Join-Path $folder 'result.json'
function Save-Result($value) {
    Write-AtomicRestartMarker $resultPath ($value | ConvertTo-Json -Depth 8)
}
function Identity($process) {
    $ticks = $process.StartTime.ToUniversalTime().Ticks
    $ticks -= $ticks % 10
    return "$($process.Id)|$ticks"
}
function Read-Result {
    if (Test-Path -LiteralPath $resultPath) {
        return Get-Content -LiteralPath $resultPath -Raw -Encoding UTF8 | ConvertFrom-Json
    }
    return [pscustomobject]@{Phase='scheduled'; Detail=''; CheckedAt=''; Replacement=''; ObservedAt=''; Notification='not_attempted'}
}
function Set-Phase($phase, $detail) {
    $script:result.Phase = $phase
    $script:result.Detail = [string]$detail
    $script:result.CheckedAt = [DateTime]::UtcNow.ToString('o')
    Save-Result $script:result
}
function Assert-Artifact {
    if ((Get-CdrArtifactHash $binary) -cne $state.BinaryHash) { throw 'Runtime artifact changed; QA refused' }
    foreach ($entry in $state.ScriptHashes) {
        if ((Get-CdrArtifactHash (Join-Path $root $entry.Path)) -cne $entry.Hash) {
            throw "Reviewed control script changed: $($entry.Path)"
        }
    }
}
function Assert-Fence($fence) {
    if ($null -eq $fence -or $fence.ProcessIdentity -cne $state.OriginalIdentity -or
        $fence.RuntimeId -cne $state.RuntimeId) { throw 'Foreign restart operation preserved' }
}
function Invoke-ExactWatchdog([string[]]$Arguments) {
    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $lines = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $watchdog -RepoRoot $root @Arguments 2>&1
        $code = $LASTEXITCODE
    } finally { $ErrorActionPreference = $previous }
    $details = ($lines | Out-String)
    if ($details.Length -gt 16000) { $details = $details.Substring($details.Length-16000) }
    Write-AtomicRestartMarker (Join-Path $folder 'recovery-output.txt') $details
    if ($code -ne 0) { throw "Exact watchdog exited ${code}: $details" }
}
function Publish-Result {
    if ($result.Notification -ne 'not_attempted') { return }
    # Persist before send: uncertain delivery is not retried automatically.
    $result.Notification = 'attempting'; Save-Result $result
    $caption = if ($result.Phase -eq 'passed') {
        "2번 재시작 시험 PASS: 기존 봇 종료, 새 봇 1개 실행, 정상 작동 신호를 두 번 확인했습니다. Discord에서 일반 메시지 하나를 보내 답변이 정상인지 확인해 주세요."
    } else {
        "2번 재시작 시험 미완료: $($result.Detail) 봇을 강제 종료하거나 안전 검사를 우회하지 않았습니다. 결과 파일을 첨부합니다."
    }
    if ($caption.Length -gt 1800) { $caption=$caption.Substring(0,1800) }
    $captionPath = Join-Path $folder 'notification.txt'
    Write-AtomicRestartMarker $captionPath $caption
    & $state.Python -X utf8 (Join-Path $root 'send_discord_attachment.py') `
        --thread-ref $state.ThreadId --content-file $captionPath $resultPath *> $null
    $result.Notification = if ($LASTEXITCODE -eq 0) { 'sent' } else { 'failed_or_unknown_no_retry' }
    Save-Result $result
}
if ($Mode -eq 'Inspect') {
    Assert-Artifact
    $p = Get-Process -Id $state.OriginalPid -ErrorAction Stop
    if ($p.Path -ine $binary -or (Identity $p) -cne $state.OriginalIdentity) { throw 'Original runtime changed' }
    Write-Output 'qa_preparation_verified_no_restart_requested'
    exit 0
}
$guard = $null
try {
    try { $guard = [IO.File]::Open((Join-Path $folder 'worker.lock'),'OpenOrCreate','ReadWrite','None') }
    catch [IO.IOException] {
        if (($_.Exception.HResult -band 0xffff) -in @(32,33)) { exit 0 }
        throw
    }
    $script:result = Read-Result
    if ($result.Phase -in @('passed','blocked')) { Publish-Result; exit 0 }
    Assert-Artifact
    if ($Mode -eq 'Worker') {
        if ($result.Phase -ne 'scheduled') { throw 'Worker is single-attempt; replay refused' }
        $p = Get-Process -Id $state.OriginalPid -ErrorAction Stop
        if ($p.Path -ine $binary -or (Identity $p) -cne $state.OriginalIdentity) { throw 'Original runtime changed' }
        Set-Phase 'working' 'Waiting for all active work to finish; full readiness remains enabled'
        $previous = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $output = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root 'codex-discord-rust-restart.ps1') `
                -RepoRoot $root -Immediate -ExpectedBotIdentity $state.OriginalIdentity `
                -DelaySeconds 15 -QuietSeconds 15 -WaitTimeoutSeconds 180 2>&1
            $code = $LASTEXITCODE
        } finally { $ErrorActionPreference = $previous }
        $details = $output | Out-String
        if ($details.Length -gt 16000) { $details=$details.Substring($details.Length-16000) }
        Write-AtomicRestartMarker (Join-Path $folder 'worker-output.txt') $details
        Set-Phase 'verify_pending' "Restart entry exited $code; independent verification required"
        exit 0
    }
    if ($result.Phase -eq 'scheduled') { return }
    # Only continue a restart belonging to the original runtime, never generic health recovery.
    $journalPath = Join-Path $root '.codex_discord_rust.restart.launch'
    $restartPath = Join-Path $root '.codex_discord_rust.restart'
    $preparePath = Join-Path $root '.codex_discord_rust.drain.prepare'
    $receiptPath = Join-Path $root '.codex_discord_rust.restart.completed'
    $fence = $null
    if (Test-Path -LiteralPath $journalPath) {
        $journal = Get-Content -LiteralPath $journalPath -Raw | ConvertFrom-Json
        $fence = $journal.Fence
    } elseif (Test-Path -LiteralPath $restartPath) { $fence = Get-RestartDrainFence $restartPath }
    elseif (Test-Path -LiteralPath $preparePath) {
        $fence = Get-RestartDrainFence $preparePath; Assert-Fence $fence
        Invoke-ExactWatchdog @('-PrepareRestart','-ExpectedRuntimeIdentity',$state.OriginalIdentity,
            '-RestartQuietSeconds','15','-RestartWaitTimeoutSeconds','30')
    } elseif (Test-Path -LiteralPath $receiptPath) {
        $fence = Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
    }
    if ($null -eq $fence) {
        $reason = 'No restart was published; original readiness did not complete'
        $workerOutput = Join-Path $folder 'worker-output.txt'
        if (Test-Path $workerOutput) { $reason += ': ' + [IO.File]::ReadAllText($workerOutput) }
        throw $reason
    }
    Assert-Fence $fence
    Invoke-ExactWatchdog @('-CompleteRestartFenceJson',($fence | ConvertTo-Json -Compress))
    $receipt = Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
    Assert-Fence $receipt
    $processes = @(Get-Process -Name cdr-runtime -ErrorAction SilentlyContinue | Where-Object {$_.Path -ieq $binary})
    if ($processes.Count -ne 1) { throw 'Expected exactly one runtime for this binary' }
    $newIdentity = Identity $processes[0]
    if ($newIdentity -ceq $state.OriginalIdentity -or $newIdentity -cne $receipt.ReplacementIdentity) {
        throw 'Replacement identity does not match completion receipt'
    }
    $old = Get-Process -Id $state.OriginalPid -ErrorAction SilentlyContinue
    if ($old -and (Identity $old) -ceq $state.OriginalIdentity) { throw 'Original runtime still exists' }
    if ($result.Replacement -and $result.Replacement -cne $newIdentity) { throw 'Replacement changed between observations' }
    if (-not $result.ObservedAt) {
        $result.Replacement=$newIdentity; $result.ObservedAt=[DateTime]::UtcNow.ToString('o')
        Set-Phase 'observing' 'First exact receipt and fresh heartbeat verified; awaiting second observation'
        return
    }
    if (([DateTime]::UtcNow-[DateTime]::Parse($result.ObservedAt).ToUniversalTime()).TotalSeconds -lt 15) { return }
    Set-Phase 'passed' 'Original exited; exactly one matching replacement healthy at two observations'
    Publish-Result
} catch {
    if ($null -ne $guard) {
        if (-not $script:result) { $script:result=Read-Result }
        Set-Phase 'blocked' $_.Exception.Message
        Write-AtomicRestartMarker (Join-Path $folder 'blocker-pending.json') (@{
            event_type='thread_blocked'; thread_id=$state.ThreadId; thread_title='Restart live QA 2';
            blocker_text=$_.Exception.Message; detected_by='explicit_status_or_keyword';
            recipient_key='kakao_business_chat:4969060002183711'; created_at=[DateTime]::UtcNow.ToString('o');
            local_delivery_status='pending_hub_configuration'
        } | ConvertTo-Json)
        Publish-Result
    }
    throw
} finally { if ($null -ne $guard) { $guard.Dispose() } }
