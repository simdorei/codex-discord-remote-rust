# Completion is a single control-lock decision. Observation outside the lock is
# useful evidence, but never authority to consume state after a competing intent.
function Get-CutoverTargetIdentity {
    param([string]$Target)
    if ($Target -eq 'python') { return Get-PythonIdentity }
    return Get-RustIdentity
}

function Throw-CutoverFailure {
    param($Transaction, $Failure)
    $primary = if ($Failure -is [Management.Automation.ErrorRecord]) { $Failure.Exception.Message } else { [string]$Failure }
    try { Ensure-CutoverRecoveryDisabled -Transaction $Transaction }
    catch {
        throw "Cutover failed: $primary; recovery seal not published: $($_.Exception.Message)"
    }
    throw "Cutover failed: $primary; automatic rollback disabled; recovery state preserved"
}

function Bind-CutoverTarget {
    param($Transaction, [string]$Target)
    $identity = Get-CutoverTargetIdentity $Target
    if ([string]::IsNullOrWhiteSpace($identity)) { throw 'Cutover target identity is unavailable' }
    $Transaction | Add-Member -NotePropertyName TargetIdentity -NotePropertyValue $identity -Force
    $Transaction | Add-Member -NotePropertyName CompletionRuntime -NotePropertyValue $Target -Force
}

function Complete-CutoverState {
    param($Transaction)
    $control = Enter-CdrControl -Root $RepoRoot
    try {
        Assert-CdrNoPendingRestart -Root $RepoRoot
        foreach ($name in @('.codex_discord_rust.stop', '.codex_discord_bot.stop')) {
            if (Test-Path -LiteralPath (Join-Path $RepoRoot $name)) {
                throw 'New maintenance intent prevents cutover completion'
            }
        }
        $current = Get-CutoverState
        if ($null -eq $current -or $current.TransactionId -cne $Transaction.TransactionId) {
            throw 'Cutover recovery state ownership changed'
        }
        $target = if ($Transaction.CompletionRuntime) { $Transaction.CompletionRuntime } else { $Transaction.TargetRuntime }
        if ([string]::IsNullOrWhiteSpace($Transaction.TargetIdentity) -or
            $current.TargetIdentity -cne $Transaction.TargetIdentity -or
            (Get-CutoverTargetIdentity $target) -cne $Transaction.TargetIdentity -or
            -not (Test-TargetHealthy $target)) {
            throw 'Cutover target identity or health changed before completion'
        }
        if (-not [IO.File]::Exists($ModePath) -or [IO.File]::ReadAllText($ModePath).Trim() -cne $target) {
            throw 'Cutover mode changed before completion'
        }
        $owner = Get-DisableOwner
        if ($null -ne $owner -and ($owner.kind -notin @('cutover','cutover_recovery') -or
            $owner.transaction_id -cne $Transaction.TransactionId)) {
            throw 'New maintenance intent prevents cutover completion'
        }
        # Persist exact target identity before consuming the recoverable state.
        Set-CutoverPhase $Transaction 'target_healthy'
        if ($null -ne $owner) { [IO.File]::Delete($DisablePath) }
        [IO.File]::Delete($CutoverStatePath)
    } finally { $control.Dispose() }
}
