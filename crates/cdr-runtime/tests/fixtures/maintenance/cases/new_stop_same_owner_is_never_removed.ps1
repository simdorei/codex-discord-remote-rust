param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PREPARED.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
[IO.File]::WriteAllText($StopPath,$op)
try{Complete-CdrMaintenance $s $StatePath;throw 'stop accepted'}catch{if($_.Exception.Message -notmatch 'post_launch_intent'){throw}}
if(-not (Test-Path $StopPath) -or -not (Test-Path $DisablePath)){throw 'stop/lock lost'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
