# Runtime proof and owned cleanup, deliberately independent of notice delivery.
. (Join-Path $PSScriptRoot 'CdrMaintenanceCompletionAudit.ps1')
function Get-CdrRuntimeCompletionEvidence($State) {
    $child=Get-CdrMaintenanceChild $State -RequireFresh
    $journal=Read-CdrLaunchJournal (Join-Path $State.Bundle 'launch.json')
    if(-not (Test-RestartDrainFenceMatch $journal.Fence $State.Fence)){throw 'maintenance_completion_fence_mismatch'}
    if(Get-Process -Id ([int]$State.Fence.ProcessIdentity.Split('|')[0]) -ErrorAction SilentlyContinue){throw 'maintenance_original_PID_still_present'}
    $now=[DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    if($State.Heartbeats.Count -ne 2 -or $State.Heartbeats[1] -le $State.Heartbeats[0] -or
       $State.Heartbeats[0] -lt ($now-45) -or $State.Heartbeats[1] -gt $now){throw 'maintenance_fresh_runtime_proof_missing'}
    [pscustomobject]@{Operation=$State.Operation;ChildIdentity=$journal.ChildIdentity;
        CandidateHash=$State.CandidateHash;Heartbeats=@($State.Heartbeats);ObservedAt=[DateTimeOffset]::UtcNow.ToString('o')}
}

function Assert-CdrPersistedRuntimeEvidence($Evidence, $State) {
    if(-not $Evidence -or $Evidence.Operation -cne $State.Operation -or
       $Evidence.CandidateHash -cne $State.CandidateHash -or $Evidence.ChildIdentity -notmatch '^\d+\|\d+$'){
        throw 'maintenance_runtime_evidence_mismatch'
    }
    try {
        $observed=[DateTimeOffset]::Parse($Evidence.ObservedAt).ToUnixTimeSeconds()
        if($Evidence.Heartbeats.Count -ne 2 -or [string]$Evidence.Heartbeats[0] -notmatch '^\d+$' -or
           [string]$Evidence.Heartbeats[1] -notmatch '^\d+$' -or
           [long]$Evidence.Heartbeats[1] -le [long]$Evidence.Heartbeats[0] -or
           [long]$Evidence.Heartbeats[0] -lt ($observed-45) -or
           [long]$Evidence.Heartbeats[1] -gt $observed -or $observed -gt [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()){
            throw 'invalid witness'
        }
    } catch {throw 'maintenance_evidence_heartbeat_invalid'}
}

function Assert-CdrRuntimeReceiptBinding($Receipt, $State) {
    if($Receipt.Operation -cne $State.Operation -or $Receipt.Phase -cne 'verified' -or
       $Receipt.CompletionPolicy -cne 'runtime-proof-v1' -or
       $Receipt.CandidateHash -cne $State.CandidateHash -or
       $Receipt.RuntimeEvidence.ChildIdentity -cne $State.RuntimeEvidence.ChildIdentity -or
       $Receipt.BinaryPath -cne $State.BinaryPath -or $Receipt.RepoRoot -cne $State.RepoRoot -or
       -not (Test-RestartDrainFenceMatch $Receipt.Fence $State.Fence)){throw 'maintenance_completion_receipt_mismatch'}
    Assert-CdrPersistedRuntimeEvidence $Receipt.RuntimeEvidence $State
    if(($Receipt.Reconciliation -or $State.Reconciliation) -and
       ($Receipt.Reconciliation.ManifestHash -cne $State.Reconciliation.ManifestHash -or
        $Receipt.Reconciliation.OriginalStateHash -cne $State.Reconciliation.OriginalStateHash)){
        throw 'maintenance_completion_review_authority_mismatch'
    }
}

function Publish-CdrRuntimeReceipt($State, [string]$StatePath) {
    Assert-CdrPersistedRuntimeEvidence $State.RuntimeEvidence $State
    $archive=Join-Path $State.Bundle 'completion.json'
    $text=$State|ConvertTo-Json -Depth 12 -Compress
    if([IO.File]::Exists($archive)) {
        Assert-CdrRuntimeReceiptBinding (Get-Content -LiteralPath $archive -Raw -Encoding UTF8|ConvertFrom-Json) $State
    } else { Write-NewCdrMarker $archive $text }
    $latest=$StatePath+'.completed'
    if([IO.File]::Exists($latest)) {
        $existing=Get-Content -LiteralPath $latest -Raw -Encoding UTF8|ConvertFrom-Json
        if($existing.Operation -ceq $State.Operation) { Assert-CdrRuntimeReceiptBinding $existing $State; return }
        if(-not $State.PreviousCompletedHash -or (Get-CdrArtifactHash $latest) -cne $State.PreviousCompletedHash){throw 'maintenance_newer_completion_preserved'}
    } elseif($State.PreviousCompletedHash){throw 'maintenance_previous_completion_missing'}
    Write-AtomicRestartMarker $latest $text
}

function Complete-CdrRuntimeMaintenance($State, [string]$StatePath) {
    Assert-CdrMaintenanceArtifacts $State
    Assert-CdrMaintenanceMarkers $State
    $null=Get-CdrMaintenanceChild $State -RequireFresh
    Assert-CdrPersistedRuntimeEvidence $State.RuntimeEvidence $State
    if(Get-Process -Id ([int]$State.Fence.ProcessIdentity.Split('|')[0]) -ErrorAction SilentlyContinue){throw 'maintenance_original_PID_still_present'}
    $journal=Read-CdrLaunchJournal (Join-Path $State.Bundle 'launch.json')
    if(-not $State.RuntimeEvidence -or $State.RuntimeEvidence.Operation -cne $State.Operation -or
       $State.RuntimeEvidence.CandidateHash -cne $State.CandidateHash -or
       $State.RuntimeEvidence.ChildIdentity -cne $journal.ChildIdentity -or
       -not (Test-RestartDrainFenceMatch $journal.Fence $State.Fence)){throw 'maintenance_runtime_evidence_mismatch'}
    $owned=Read-CdrMaintenanceState $StatePath
    if($owned.Operation -cne $State.Operation -or $owned.Phase -cne 'verified'){throw 'maintenance_completion_owner_changed'}
    # Missing seal is resumable ONLY after our durable completion receipt exists.
    if(-not [IO.File]::Exists($DisablePath)) {
        if(-not [IO.File]::Exists($StatePath+'.completed')){throw 'maintenance_seal_missing_without_completion'}
        Assert-CdrRuntimeReceiptBinding (Get-Content -LiteralPath ($StatePath+'.completed') -Raw -Encoding UTF8|ConvertFrom-Json) $State
    }
    Publish-CdrRuntimeReceipt $State $StatePath
    # Preserve failures added AFTER the immutable completion receipt was published.
    # If that proof cannot be saved, do not delete its sole remaining active copy.
    Save-CdrCompletionFailureAudit $owned -FromActiveState
    Assert-CdrMaintenanceMarkers $State
    $null=Get-CdrMaintenanceChild $State -RequireFresh
    Assert-CdrMarkerOwner $DisablePath $State.Operation
    if([IO.File]::Exists($DisablePath)){[IO.File]::Delete($DisablePath)}
    Assert-CdrMaintenanceMarkers $State
    if((Read-CdrMaintenanceState $StatePath).Operation -cne $State.Operation){throw 'maintenance_completion_owner_changed'}
    [IO.File]::Delete($StatePath)
    Write-Output 'maintenance_runtime_verified; automatic recovery unsealed; notification tracked separately'
}
