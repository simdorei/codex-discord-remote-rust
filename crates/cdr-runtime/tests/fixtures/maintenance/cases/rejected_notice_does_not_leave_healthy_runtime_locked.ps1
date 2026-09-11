param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'CONNECTED.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
try {Invoke-CdrMaintenanceEngine $StatePath $op} catch {}
if(Test-Path $DisablePath){throw 'M01: healthy runtime remains disabled by notification failure'}
if(Test-Path $StatePath){throw 'M01: healthy runtime ownership remains active'}
$receipt=Get-Content ($StatePath+'.completed') -Raw | ConvertFrom-Json
if($receipt.Phase -cne 'verified' -or $receipt.CompletionPolicy -cne 'runtime-proof-v1'){throw 'runtime completion missing'}
if($script:starts -ne 1 -or $script:posts -ne 1){throw 'wrong side-effect count'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
