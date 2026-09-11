param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'), [Text.Encoding]::UTF8)))
[void][IO.Directory]::CreateDirectory((Get-CdrMaintenanceNoticePath $s))
[void][IO.Directory]::CreateDirectory((Join-Path $s.Bundle 'notification-journal-error.json'))
$visible=@(Invoke-CdrMaintenanceEngine $StatePath $op 3>&1)
if((Test-Path $StatePath) -or (Test-Path $DisablePath) -or $script:posts -ne 0){throw 'notice diagnostic failure relocked runtime'}
if(($visible -join ' ') -notmatch 'failure evidence unavailable'){throw 'diagnostic storage failure was silent'}
$receipt=Get-Content ($StatePath+'.completed') -Raw|ConvertFrom-Json
if($receipt.InitialNotificationOutcome -cne 'pending'){throw 'unconfirmed notice advertised as sent'}
