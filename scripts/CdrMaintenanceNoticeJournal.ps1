# Per-operation notice results never overwrite the shared latest completion receipt.
function Get-CdrMaintenanceNoticePath($State) { Join-Path $State.Bundle 'notification.json' }

function Initialize-CdrMaintenanceNotice($State, [string]$InitialStatus='pending') {
    $path=Get-CdrMaintenanceNoticePath $State
    if([IO.File]::Exists($path)) { return Read-CdrMaintenanceNotice $State }
    $notice=[pscustomobject]@{Version=1;Operation=$State.Operation;CandidateHash=$State.CandidateHash;
        Status=$InitialStatus;Receipt='';Diagnostic=$null}
    Write-NewCdrMarker $path ($notice|ConvertTo-Json -Depth 6 -Compress)
    return $notice
}

function Read-CdrMaintenanceNotice($State) {
    $notice=Get-Content -LiteralPath (Get-CdrMaintenanceNoticePath $State) -Raw -Encoding UTF8 | ConvertFrom-Json
    if($notice.Version -ne 1 -or $notice.Operation -cne $State.Operation -or
       $notice.CandidateHash -cne $State.CandidateHash -or
       $notice.Status -notin @('pending','sending','sent','rejected','unknown')) { throw 'maintenance_notice_identity_invalid' }
    return $notice
}

function Save-CdrMaintenanceNotice($State, $Notice) {
    $null=Read-CdrMaintenanceNotice $State
    if($Notice.Operation -cne $State.Operation -or $Notice.CandidateHash -cne $State.CandidateHash){throw 'maintenance_notice_owner_changed'}
    Write-AtomicRestartMarker (Get-CdrMaintenanceNoticePath $State) ($Notice|ConvertTo-Json -Depth 6 -Compress)
}

function Send-CdrCompletedNotice($State) {
    # Called only after successful ownership cleanup, once in the initial entry.
    if([IO.File]::Exists((Join-Path $State.Bundle 'notification-journal-error.json'))){
        Write-Warning 'Runtime complete; notification_journal previously unconfirmed; no resend';return
    }
    $notice=Initialize-CdrMaintenanceNotice $State
    if($notice.Status -ne 'pending') { Write-Warning "Runtime complete; notification=$($notice.Status); no resend"; return }
    $notice.Status='sending'
    Save-CdrMaintenanceNotice $State $notice
    try {
        $content="Deployment runtime verified: one new runtime identity=$($State.RuntimeEvidence.ChildIdentity); candidate_sha256=$($State.CandidateHash); operation=$($State.Operation)"
        $notice.Receipt=Send-CdrMaintenanceMessage $State $content ('ok'+$State.Operation.Substring(0,22)) 20
        $notice.Status='sent'
    } catch {
        $notice.Diagnostic=Get-CdrNotificationDiagnostic $_
        $notice.Status=$notice.Diagnostic.Outcome
    }
    Save-CdrMaintenanceNotice $State $notice
    if($notice.Status -ne 'sent') { Write-Warning ("Runtime complete; notification="+$notice.Status+'; '+($notice.Diagnostic|ConvertTo-Json -Compress)) }
    Write-Output "maintenance_notification=$($notice.Status)"
}

function Write-CdrCompletedNoticeFailure($State, $Record) {
    # A notice-only I/O fault must not recreate runtime ownership after completion.
    # Preserve a safe, independent failure record; never change transports or POST.
    $diagnostic=Get-CdrNotificationDiagnostic $Record
    $base=$Record.Exception.GetBaseException()
    $failure=[pscustomobject]@{Version=1;Operation=$State.Operation;CandidateHash=$State.CandidateHash;
        Status='unknown';Diagnostic=$diagnostic;JournalExceptionType=$base.GetType().Name;HResult=$base.HResult}
    $path=Join-Path $State.Bundle 'notification-journal-error.json'
    try {
        if([IO.File]::Exists($path)){
            $existing=Get-Content -LiteralPath $path -Raw -Encoding UTF8|ConvertFrom-Json
            if($existing.Version -ne 1 -or $existing.Operation -cne $State.Operation -or
               $existing.CandidateHash -cne $State.CandidateHash -or $existing.Status -cne 'unknown'){
                throw 'maintenance_notification_journal_error_owner_changed'
            }
        } else {Write-NewCdrMarker $path ($failure|ConvertTo-Json -Depth 6 -Compress)}
    } catch {
        Write-Warning ("Runtime complete; notification_journal failure evidence unavailable; original_type="+
            $failure.JournalExceptionType+'; evidence_error_type='+$_.Exception.GetBaseException().GetType().Name+'; no automatic resend')
        return
    }
    Write-Warning ('Runtime complete; notification_journal unavailable; status=unknown; exception_type='+
        $failure.JournalExceptionType+'; hresult='+$failure.HResult+'; no automatic resend')
}
