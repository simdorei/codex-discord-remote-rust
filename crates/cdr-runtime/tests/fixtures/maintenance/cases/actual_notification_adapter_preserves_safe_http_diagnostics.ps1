param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'CONNECTED.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$caught=$null
try {Send-CdrMaintenanceMessage $s 'synthetic notice' 'fixture-nonce'} catch {$caught=$_}
if(-not $caught){throw 'rejection reported successful'}
if($caught.Exception.Data['HttpStatus'] -ne 403 -or $caught.Exception.Data['DiscordCode'] -ne 50013){throw 'M05: original HTTP/service error discarded'}
if($caught.Exception.Data['Outcome'] -cne 'rejected'){throw 'definite rejection confused with transport uncertainty'}
if($caught.Exception.Message -match 'fixture-secret'){throw 'raw credential-like text leaked'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
