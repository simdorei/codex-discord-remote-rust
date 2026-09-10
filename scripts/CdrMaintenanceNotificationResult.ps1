# Public-safe diagnostics only. Never persist arbitrary error/request/response text.
function New-CdrNotificationException($Record) {
    $status=$null; $code=$null
    try { if($Record.Exception.Response.StatusCode){$status=[int]$Record.Exception.Response.StatusCode} } catch {}
    try {
        $detail=$Record.ErrorDetails.Message | ConvertFrom-Json
        if([string]$detail.code -match '^\d{1,10}$'){$code=[long]$detail.code}
    } catch {}
    $outcome=if($status -ge 400 -and $status -lt 500 -and $status -ne 408){'rejected'}else{'unknown'}
    $kind=$Record.Exception.GetType().Name
    $publicCode=if($outcome -eq 'unknown'){'maintenance_notification_outcome_unknown'}else{'maintenance_notification_rejected'}
    $noticeException=[Exception]::new("$publicCode; http_status=$status; discord_code=$code; exception_type=$kind; no automatic resend")
    $noticeException.Data['HttpStatus']=$status; $noticeException.Data['DiscordCode']=$code
    $noticeException.Data['Outcome']=$outcome; $noticeException.Data['ExceptionType']=$kind
    return $noticeException
}

function Get-CdrNotificationDiagnostic($Record) {
    $exception=$Record.Exception
    $outcome=[string]$exception.Data['Outcome']
    if($outcome -notin @('rejected','unknown')){$exception=New-CdrNotificationException $Record;$outcome='unknown'}
    [pscustomobject]@{Outcome=$outcome;HttpStatus=$exception.Data['HttpStatus'];DiscordCode=$exception.Data['DiscordCode'];ExceptionType=$exception.Data['ExceptionType']}
}
