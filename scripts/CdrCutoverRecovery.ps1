# Rust-only cutover support; loaded by codex-discord-runtime-cutover.ps1.
function Restore-TransactionSource {
    param($Transaction)
    Assert-RustCutoverPreflight
    Enter-CutoverMaintenance -Transaction $Transaction
    Set-CutoverPhase -Transaction $Transaction -Phase 'recovery_starting'
    Publish-Mode -Value 'rust'
    Set-CutoverPhase -Transaction $Transaction -Phase 'recovery_source_starting'
    Exit-CutoverMaintenance -Transaction $Transaction
    Start-Rust
    Bind-CutoverTarget $Transaction 'rust'
    Wait-RustHealthy
    Set-CutoverPhase -Transaction $Transaction -Phase 'recovery_complete'
    Complete-CutoverState -Transaction $Transaction
}

function Recover-InterruptedCutover {
    $transaction = Get-CutoverState
    if ($null -eq $transaction) { return }
    if ($DryRun) {
        if ($Recover) {
            Write-Output (
                "would_explicitly_recover_interrupted_cutover " +
                "transaction=$($transaction.TransactionId) " +
                "source=$($transaction.SourceRuntime) phase=$($transaction.Phase)"
            )
        } else {
            Write-Output (
                "would_require_explicit_cutover_recovery " +
                "transaction=$($transaction.TransactionId) " +
                "source=$($transaction.SourceRuntime) phase=$($transaction.Phase) " +
                "rerun_with=-Recover"
            )
        }
        $script:CutoverRecoveryPreviewed = $true
        return
    }
    if (
        $transaction.OwnerIdentity -ne $CutoverIdentity -and
        (Test-ProcessIdentityAlive -Identity $transaction.OwnerIdentity)
    ) {
        throw "Another cutover process is active: identity=$($transaction.OwnerIdentity)"
    }
    if (
        $transaction.Phase -eq 'target_healthy' -and
        (Test-TargetHealthy -Target $(if ($transaction.CompletionRuntime) { $transaction.CompletionRuntime } else { $transaction.TargetRuntime }))
    ) {
        Complete-CutoverState -Transaction $transaction
        $completedRuntime = if ($transaction.CompletionRuntime) { $transaction.CompletionRuntime } else { $transaction.TargetRuntime }
        Write-Output "interrupted_cutover_finalized runtime=$completedRuntime"
        $script:CutoverRecoveryHandled = $true
        return
    }
    if (-not $Recover) {
        $reason = (
            "Interrupted cutover requires explicit source recovery; " +
            "automatic rollback is disabled. Run: " +
            ".\codex-discord-runtime-cutover.ps1 -Runtime rust -Recover"
        )
        Throw-CutoverFailure -Transaction $transaction -Failure $reason
    }
    try {
        Restore-TransactionSource -Transaction $transaction
        Write-Output (
            "interrupted_cutover_recovered runtime=$($transaction.SourceRuntime) " +
            "recovery=explicit"
        )
        $script:CutoverRecoveryHandled = $true
    } catch {
        Throw-CutoverFailure -Transaction $transaction -Failure $_
    }
}
