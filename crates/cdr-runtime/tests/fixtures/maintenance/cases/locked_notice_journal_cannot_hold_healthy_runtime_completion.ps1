param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'), [Text.Encoding]::UTF8)))
$null=Initialize-CdrMaintenanceNotice $s
$path=Get-CdrMaintenanceNoticePath $s
$guard=[IO.File]::Open($path,'Open','ReadWrite','None')
try{try{$visible=@(Invoke-CdrMaintenanceEngine $StatePath $op 3>&1)}catch{}}
finally{$guard.Dispose()}
if((Test-Path $StatePath) -or (Test-Path $DisablePath)){throw 'R1: notice-only file fault retained runtime ownership'}
$receipt=Get-Content ($StatePath+'.completed') -Raw|ConvertFrom-Json
if($receipt.Phase -cne 'verified' -or $receipt.InitialNotificationOutcome -cne 'pending'){throw 'runtime proof/pending notice missing'}
$errorPath=Join-Path $s.Bundle 'notification-journal-error.json'
if(-not (Test-Path $errorPath)){throw 'notice journal error evidence missing'}
$errorRecord=Get-Content $errorPath -Raw|ConvertFrom-Json
if($errorRecord.Operation -cne $op -or $errorRecord.Status -cne 'unknown'){throw 'notice fault identity/outcome lost'}
if(($visible -join ' ') -notmatch 'notification_journal'){throw 'notice file failure was silent'}
Send-CdrCompletedNotice $receipt
if($script:posts -ne 0 -or $script:starts -ne 1){throw 'notice file fault caused POST or relaunch'}
if(@($script:calls|Where-Object{$_ -match ':(stop|install|cleanup)$'}).Count){throw 'runtime work replayed'}
