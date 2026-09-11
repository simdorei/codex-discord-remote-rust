param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'LOCAL.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$s=Read-CdrMaintenanceState $StatePath
if('missing' -eq 'missing'){$s.PSObject.Properties.Remove('ShutdownPolicy')}else{$s.ShutdownPolicy='unknown'}
Write-AtomicRestartMarker $StatePath ($s|ConvertTo-Json -Depth 10)
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'bad policy accepted'}
catch{if($_.Exception.Message -notmatch 'shutdown_policy'){throw}}
if($script:calls.Count){throw 'action before policy check'}
}
'1' {
$s=Read-CdrMaintenanceState $StatePath
if('unknown' -eq 'missing'){$s.PSObject.Properties.Remove('ShutdownPolicy')}else{$s.ShutdownPolicy='unknown'}
Write-AtomicRestartMarker $StatePath ($s|ConvertTo-Json -Depth 10)
try{Invoke-CdrMaintenanceEngine $StatePath $op;throw 'bad policy accepted'}
catch{if($_.Exception.Message -notmatch 'shutdown_policy'){throw}}
if($script:calls.Count){throw 'action before policy check'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
