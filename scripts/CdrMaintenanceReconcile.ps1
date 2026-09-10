function Assert-CdrReconcileIntents($State, $Manifest, [string]$ManifestHash, [string]$StatePath) {
    Assert-CdrMarkerOwner $DisablePath $State.Operation
    foreach($path in @($StopPath,$DrainPreparePath,$DrainAckPath,$RestartPath,
        (Join-Path $RepoRoot '.codex_discord_rust.restart.launch'),(Join-Path $RepoRoot '.codex_discord_runtime.cutover'))){
        if([IO.File]::Exists($path)){throw 'reconcile_new_or_foreign_intent_preserved'}
    }
    Assert-CdrNoOrphanRestartClaim $RepoRoot
    $archive=Join-Path $State.Bundle 'completion.json'
    if(-not [IO.File]::Exists($DisablePath) -and -not [IO.File]::Exists($archive)){throw 'reconcile_seal_missing_without_completion'}
    foreach($path in @($archive,($StatePath+'.completed'))){
        if(-not [IO.File]::Exists($path)){continue}
        $r=Get-Content -LiteralPath $path -Raw -Encoding UTF8|ConvertFrom-Json
        if($path -ceq ($StatePath+'.completed') -and $r.Operation -cne $State.Operation){
            if($Manifest.PreviousCompletedHash -and (Get-CdrArtifactHash $path) -ceq $Manifest.PreviousCompletedHash){continue}
            throw 'maintenance_newer_completion_preserved'
        }
        if($r.Operation -cne $State.Operation -or $r.Phase -cne 'verified' -or
           $r.CandidateHash -cne $State.CandidateHash -or $r.RuntimeEvidence.ChildIdentity -cne $Manifest.ChildIdentity -or
           $r.Reconciliation.ManifestHash -cne $ManifestHash -or
           $r.Reconciliation.OriginalStateHash -cne $Manifest.OriginalStateHash){throw 'reconcile_receipt_authority_mismatch'}
        Assert-CdrPersistedRuntimeEvidence $r.RuntimeEvidence $State
    }
}

function Invoke-CdrCompletionReconcile($Manifest,[string]$ManifestHash,[string]$StatePath,[switch]$Apply) {
    $s=Assert-CdrReconcileBindings $Manifest $StatePath -AllowCompleted
    $archive=Join-Path $s.Bundle 'completion.json'
    if(-not [IO.File]::Exists($StatePath)){
        if(-not [IO.File]::Exists($archive)){throw 'reconcile_completion_missing'}
        $done=Get-Content -LiteralPath $archive -Raw -Encoding UTF8|ConvertFrom-Json
        if($done.Reconciliation.ManifestHash -cne $ManifestHash -or $done.Operation -cne $s.Operation -or
           $done.Phase -cne 'verified'){throw 'reconcile_completed_receipt_mismatch'}
        Write-Output 'reconcile_previously_completed; no action, no current-health claim';return
    }
    Assert-CdrReconcileIntents $s $Manifest $ManifestHash $StatePath
    $proof=Measure-CdrCompletionProof $s $Manifest.ChildIdentity
    $null=Assert-CdrReconcileBindings $Manifest $StatePath
    Assert-CdrReconcileIntents $s $Manifest $ManifestHash $StatePath
    if(-not $Apply){Write-Output "reconcile_preflight_ready identity=$($proof.ChildIdentity); no runtime or state mutation";return}
    # The source active state is never rewritten, including heartbeat observations.
    $receipt=Get-Content -LiteralPath (Join-Path $s.Bundle 'completion-original/state.json') -Raw -Encoding UTF8|ConvertFrom-Json
    $receipt.Phase='verified'
    $receipt|Add-Member CompletionPolicy 'runtime-proof-v1'
    $receipt|Add-Member RuntimeEvidence $proof
    $receipt|Add-Member PreviousCompletedHash $Manifest.PreviousCompletedHash
    $receipt|Add-Member Reconciliation ([pscustomobject]@{ManifestHash=$ManifestHash;OriginalStateHash=$Manifest.OriginalStateHash;
        OriginalStatePath=(Join-Path $s.Bundle 'completion-original/state.json');OriginalPhase=$s.Phase;
        PriorNotificationOutcome='unknown';Mode='completion-only'})
    Publish-CdrRuntimeReceipt $receipt $StatePath
    $null=Assert-CdrReconcileBindings $Manifest $StatePath
    Assert-CdrReconcileIntents $s $Manifest $ManifestHash $StatePath
    $null=Get-CdrCompletionObservation $s $Manifest.ChildIdentity
    if([IO.File]::Exists($DisablePath)){[IO.File]::Delete($DisablePath)}
    $null=Assert-CdrReconcileBindings $Manifest $StatePath
    Assert-CdrReconcileIntents $s $Manifest $ManifestHash $StatePath
    [IO.File]::Delete($StatePath)
    try {
        $notice=Initialize-CdrMaintenanceNotice $receipt 'unknown'
        if($notice.Status -cne 'unknown' -or $notice.Receipt){throw 'reconcile_legacy_notice_evidence_changed'}
    } catch {Write-CdrCompletedNoticeFailure $receipt $_}
    Write-Output "reconcile_runtime_verified identity=$($proof.ChildIdentity); owned maintenance markers removed; old notification remains unknown and unresent"
}
