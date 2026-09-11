param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'CONNECTED.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$script:noticeGuard=$null
function Invoke-RestMethod {
 $script:posts++
 $script:noticeGuard=[IO.File]::Open((Get-CdrMaintenanceNoticePath $s),'Open','Read','Read')
 [pscustomobject]@{id='12345';channel_id=$s.NotifyChannel}
}
try{Invoke-CdrMaintenanceEngine $StatePath $op}finally{if($script:noticeGuard){$script:noticeGuard.Dispose()}}
if((Read-CdrMaintenanceNotice $s).Status -cne 'sending'){throw 'sending intent not preserved'}
$s=Get-Content ($StatePath+'.completed') -Raw|ConvertFrom-Json
Send-CdrCompletedNotice $s
if($script:posts -ne 1 -or (Test-Path $StatePath) -or (Test-Path $DisablePath)){throw 'ambiguous result replayed/relocked'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
