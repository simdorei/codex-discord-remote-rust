# Bounded first/latest failure evidence, separate from immutable runtime receipts.
function Get-CdrCompletionFailureSnapshot($State) {
    [pscustomobject]@{LastError=[string]$State.LastError;FailureObservation=$State.FailureObservation;
        FailureNoticePhase=[string]$State.FailureNoticePhase;FailureReceipt=[string]$State.FailureReceipt;
        FailureNoticeError=[string]$State.FailureNoticeError}
}

function Assert-CdrCompletionFailureSnapshot($Snapshot) {
    if(-not $Snapshot){throw 'maintenance_completion_failure_snapshot_invalid'}
    foreach($name in @('LastError','FailureNoticePhase','FailureReceipt','FailureNoticeError')){
        if(-not $Snapshot.PSObject.Properties[$name] -or $Snapshot.$name -isnot [string]){
            throw 'maintenance_completion_failure_snapshot_invalid'
        }
    }
    if(-not $Snapshot.PSObject.Properties['FailureObservation'] -or
       $Snapshot.FailureNoticePhase -cnotin @('','none','sending','sent','unknown') -or
       ($Snapshot.FailureReceipt -and $Snapshot.FailureReceipt -notmatch '^\d+$') -or
       ($Snapshot.FailureNoticePhase -ceq 'sent' -and -not $Snapshot.FailureReceipt)){
        throw 'maintenance_completion_failure_snapshot_invalid'
    }
}

function Merge-CdrFailureNoticeEvidence($Latest, $Existing) {
    $prior=[string]$Existing.FailureNoticePhase;$incoming=[string]$Latest.FailureNoticePhase
    if($prior -in @('sent','unknown')){
        if($incoming -in @('sent','unknown') -and
           ($incoming -cne $prior -or $Latest.FailureReceipt -cne $Existing.FailureReceipt)){
            throw 'maintenance_completion_failure_notice_conflict'
        }
    } elseif($prior -ne 'sending' -or $incoming -notin @('','none')){return}
    # One failure POST per operation: its later result is stronger than stale active
    # sending/none. Preserve all result fields, while normal error Latest can advance.
    foreach($name in @('FailureNoticePhase','FailureReceipt','FailureNoticeError')){$Latest.$name=$Existing.$name}
}

function Save-CdrActiveFailureSnapshot($State, $Snapshot, [bool]$HasFailure) {
    $path=Join-Path $State.Bundle 'completion-active-failure.json'
    # Do not invent a first completion failure from an older warning. The explicit
    # first record, when present, must survive separately from the current snapshot.
    $first=$State.FirstCompletionFailure
    if($null -ne $first){Assert-CdrCompletionFailureSnapshot $first}
    $copy=[pscustomobject]@{Version=1;Operation=$State.Operation;CandidateHash=$State.CandidateHash;
        Source='active-state';First=$first;Snapshot=$Snapshot}
    if([IO.File]::Exists($path)){
        $existing=Get-Content -LiteralPath $path -Raw -Encoding UTF8|ConvertFrom-Json
        if($existing.Version -ne 1 -or $existing.Operation -cne $State.Operation -or
           $existing.CandidateHash -cne $State.CandidateHash -or $existing.Source -cne 'active-state'){
            throw 'maintenance_completion_failure_owner_changed'
        }
        Assert-CdrCompletionFailureSnapshot $existing.Snapshot
        if(-not $existing.PSObject.Properties['First']){throw 'maintenance_completion_failure_snapshot_invalid'}
        if($null -ne $existing.First){
            Assert-CdrCompletionFailureSnapshot $existing.First
            if($null -ne $first -and
               ($first|ConvertTo-Json -Depth 10 -Compress) -cne ($existing.First|ConvertTo-Json -Depth 10 -Compress)){
                throw 'maintenance_completion_failure_first_conflict'
            }
            $copy.First=$existing.First
        }
        if(-not $HasFailure){return}
        Write-AtomicRestartMarker $path ($copy|ConvertTo-Json -Depth 10 -Compress)
    } else {
        if(-not $HasFailure){return}
        Write-NewCdrMarker $path ($copy|ConvertTo-Json -Depth 10 -Compress)
    }
    $saved=Get-Content -LiteralPath $path -Raw -Encoding UTF8|ConvertFrom-Json
    if(($saved|ConvertTo-Json -Depth 10 -Compress) -cne ($copy|ConvertTo-Json -Depth 10 -Compress)){
        throw 'maintenance_completion_failure_evidence_unconfirmed'
    }
}

function Save-CdrCompletionFailureAudit($State, [switch]$ActiveStateUnpersisted, [switch]$FromActiveState) {
    $expected=Join-Path $RepoRoot ('.codex-discord-backups/maintenance-v2-'+$State.Operation)
    if($State.Operation -notmatch '^[a-f0-9]{32}$' -or $State.CandidateHash -notmatch '^[A-F0-9]{64}$' -or
       $State.Bundle -cne $expected){throw 'maintenance_completion_failure_identity_invalid'}
    $path=Join-Path $State.Bundle 'completion-failure.json'
    $hasFailure=$null -ne $State.FirstCompletionFailure -or $State.LastError -or
        [string]$State.FailureNoticePhase -notin @('','none')
    $latest=Get-CdrCompletionFailureSnapshot $State
    $first=if($State.FirstCompletionFailure){$State.FirstCompletionFailure}else{$latest}
    $audit=[pscustomobject]@{Version=1;Operation=$State.Operation;CandidateHash=$State.CandidateHash;
        First=$first;Latest=$latest;ActiveStateUnpersisted=[bool]$ActiveStateUnpersisted}
    $existing=$null
    if([IO.File]::Exists($path)){
        $existing=Get-Content -LiteralPath $path -Raw -Encoding UTF8|ConvertFrom-Json
        if($existing.Version -ne 1 -or $existing.Operation -cne $State.Operation -or
           $existing.CandidateHash -cne $State.CandidateHash){
            throw 'maintenance_completion_failure_owner_changed'
        }
        Assert-CdrCompletionFailureSnapshot $existing.First
        Assert-CdrCompletionFailureSnapshot $existing.Latest
        if($existing.PSObject.Properties['ActiveStateUnpersisted'] -and $existing.ActiveStateUnpersisted -isnot [bool]){
            throw 'maintenance_completion_failure_snapshot_invalid'
        }
    }
    if($FromActiveState){
        # A disk snapshot is not a new failure event. Preserve it separately so it
        # cannot replace a newer in-memory failure or POST result already archived.
        Assert-CdrCompletionFailureSnapshot $latest
        if($existing){
            $probe=Get-CdrCompletionFailureSnapshot $State
            Merge-CdrFailureNoticeEvidence $probe $existing.Latest
        }
        Save-CdrActiveFailureSnapshot $State $latest ([bool]$hasFailure)
        return
    }
    if(-not $hasFailure){return}
    if($existing){
        Assert-CdrCompletionFailureSnapshot $latest
        Merge-CdrFailureNoticeEvidence $latest $existing.Latest
        $audit.First=$existing.First
        Write-AtomicRestartMarker $path ($audit|ConvertTo-Json -Depth 10 -Compress)
    } else {
        if(-not $hasFailure){return}
        Assert-CdrCompletionFailureSnapshot $first
        Assert-CdrCompletionFailureSnapshot $latest
        Write-NewCdrMarker $path ($audit|ConvertTo-Json -Depth 10 -Compress)
    }
    $saved=Get-Content -LiteralPath $path -Raw -Encoding UTF8|ConvertFrom-Json
    if(($saved|ConvertTo-Json -Depth 10 -Compress) -cne ($audit|ConvertTo-Json -Depth 10 -Compress)){
        throw 'maintenance_completion_failure_evidence_unconfirmed'
    }
}
