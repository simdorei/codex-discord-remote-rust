# No credentials in state, command arguments, transcripts or output.
. (Join-Path $PSScriptRoot 'CdrMaintenanceNotificationResult.ps1')
. (Join-Path $PSScriptRoot 'CdrMaintenanceNoticeJournal.ps1')
function Send-CdrMaintenanceResult($State, [string]$StatePath) {
    Assert-CdrMaintenanceArtifacts $State
    Assert-CdrMaintenanceMarkers $State
    $child = Get-CdrMaintenanceChild $State -RequireFresh
    $content = "Deployment verified: old runtime exited; one new runtime PID=$($child.Id); two fresh heartbeats; candidate_sha256=$($State.CandidateHash); operation=$($State.Operation)"
    Send-CdrMaintenanceMessage $State $content ('ok'+$State.Operation.Substring(0,22)) (Get-CdrMaintenanceRemainingSeconds $State 20)
}

function Send-CdrMaintenanceMessage($State, [string]$Content, [string]$Nonce, [int]$TimeoutSeconds=20) {
    $token = [Environment]::GetEnvironmentVariable('DISCORD_BOT_TOKEN','Process')
    if (-not $token) {
        foreach ($line in [IO.File]::ReadAllLines($EnvPath)) {
            if ($line -match '^\s*DISCORD_BOT_TOKEN\s*=\s*(.+?)\s*$') {
                $token = $Matches[1].Trim().Trim('"').Trim("'")
            }
        }
    }
    if (-not $token) { throw 'maintenance_notification_token_missing' }
    if ($State.NotifyChannel -cne '900000000000000001') { throw 'maintenance_notification_channel_not_approved' }
    $body = @{content=$Content; nonce=$Nonce; enforce_nonce=$true;
        allowed_mentions=@{parse=@()}} | ConvertTo-Json -Depth 4 -Compress
    try {
        $reply = Invoke-RestMethod -Method Post -Uri ('https://discord.com/api/v10/channels/'+$State.NotifyChannel+'/messages') `
            -Headers @{Authorization=('Bot '+$token)} -ContentType 'application/json; charset=utf-8' `
            -Body ([Text.Encoding]::UTF8.GetBytes($body)) -TimeoutSec $TimeoutSeconds
    } catch {
        throw (New-CdrNotificationException $_)
    } finally { $token = $null }
    if ([string]$reply.id -notmatch '^\d+$' -or [string]$reply.channel_id -cne $State.NotifyChannel) {
        throw 'maintenance_notification_receipt_invalid'
    }
    return [string]$reply.id
}

function Publish-CdrMaintenanceFailure($State, [string]$StatePath) {
    # At most one failure notification, separate from success receipt and execution.
    if ($State.FailureNoticePhase -ne 'none') { return }
    $State.FailureNoticePhase='sending'
    Save-CdrMaintenanceState $State $StatePath
    try {
        $observation=$State.FailureObservation
        $content = "Deployment not completed. phase=$($State.Phase); attempt=$($State.Attempts)/3; halted=$($State.Halted). failure_code=$($observation.Code); original=$($observation.Original); observed_at=$($observation.ObservedAt); ack=$($observation.Ack); stop=$($observation.Stop). Protective state preserved; no forced stop, duplicate launch or automatic database rollback. operation=$($State.Operation)"
        $State.FailureReceipt = Send-CdrMaintenanceMessage $State $content ('err'+$State.Operation.Substring(0,21))
        $State.FailureNoticePhase='sent'
    } catch {
        $State.FailureNoticePhase='unknown'
        # Preserve a local pending event; notification trouble never replays deployment.
        $State.FailureNoticeError='Discord alert unconfirmed; inspect notification outcome manually'
    }
    Save-CdrMaintenanceState $State $StatePath
}
