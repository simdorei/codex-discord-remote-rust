param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PREPARED.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
[IO.File]::Delete($DisablePath)
try{Complete-CdrMaintenance $s $StatePath;throw 'unproven accepted'}catch{if($_.Exception.Message -notmatch 'without_completion'){throw}}
if(-not (Test-Path $StatePath)){throw 'unproven active ownership removed'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
