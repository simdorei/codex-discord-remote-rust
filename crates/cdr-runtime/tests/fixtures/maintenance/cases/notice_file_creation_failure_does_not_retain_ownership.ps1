param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'), [Text.Encoding]::UTF8)))
[void][IO.Directory]::CreateDirectory((Get-CdrMaintenanceNoticePath $s))
$visible=@(Invoke-CdrMaintenanceEngine $StatePath $op 3>&1)
if((Test-Path $StatePath) -or (Test-Path $DisablePath) -or $script:posts -ne 0){throw 'notice creation failure affected runtime'}
if(($visible -join ' ') -notmatch 'notification_journal'){throw 'notice creation failure hidden'}
if(-not (Test-Path (Join-Path $s.Bundle 'notification-journal-error.json'))){throw 'notice creation failure not recorded'}
