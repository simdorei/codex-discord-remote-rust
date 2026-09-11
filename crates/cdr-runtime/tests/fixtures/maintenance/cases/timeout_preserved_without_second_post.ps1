param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'CONNECTED.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
function Invoke-RestMethod {$script:posts++;throw [TimeoutException]::new('fixture timeout')}
Invoke-CdrMaintenanceEngine $StatePath $op
$notice=Read-CdrMaintenanceNotice $s
if($notice.Status -cne 'unknown' -or (Test-Path $DisablePath) -or (Test-Path $StatePath)){throw 'timeout affected runtime completion'}
$s=Get-Content ($StatePath+'.completed') -Raw|ConvertFrom-Json
Send-CdrCompletedNotice $s
if($script:posts -ne 1 -or $script:starts -ne 1){throw 'unknown notice replayed'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
