param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'RECONCILE.ps1'), [Text.Encoding]::UTF8)))
$null=Initialize-CdrMaintenanceNotice $s 'unknown'
$guard=[IO.File]::Open((Get-CdrMaintenanceNoticePath $s),'Open','ReadWrite','None')
try{try{. $entry -ManifestPath $manifestPath -ManifestHash $manifestHash -ExpectedOperation $op -Apply}catch{}}
finally{$guard.Dispose()}
if((Test-Path $StatePath) -or (Test-Path $DisablePath)){throw 'R1 legacy: notice-only fault retained ownership'}
$receipt=Get-Content ($StatePath+'.completed') -Raw|ConvertFrom-Json
if($receipt.Reconciliation.PriorNotificationOutcome -cne 'unknown'){throw 'legacy uncertainty lost'}
if(-not (Test-Path (Join-Path $s.Bundle 'notification-journal-error.json'))){throw 'legacy notice file failure lost'}
if($script:posts -ne 0 -or $script:starts -ne 1){throw 'legacy completion replayed external work'}
