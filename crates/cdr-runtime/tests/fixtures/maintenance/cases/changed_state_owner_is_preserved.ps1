param([string]$Variant)
$s=Read-CdrMaintenanceState $StatePath
$changed=Read-CdrMaintenanceState $StatePath;$changed.Operation='f'*32
Write-AtomicRestartMarker $StatePath ($changed|ConvertTo-Json -Depth 10)
try{Save-CdrMaintenanceState $s $StatePath;throw 'owner replaced'}catch{if($_.Exception.Message -notmatch 'owner_changed'){throw}}
if(([IO.File]::ReadAllText($StatePath)|ConvertFrom-Json).Operation -cne ('f'*32)){throw 'foreign overwritten'}
