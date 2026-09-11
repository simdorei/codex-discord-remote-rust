param([string]$Variant)
try{Write-NewCdrMarker $StopPath 'replacement';throw 'overwrite accepted'}
catch{if($_.Exception.Message -eq 'overwrite accepted'){throw}}
if([IO.File]::ReadAllText($StopPath) -cne 'owned'){throw 'marker overwritten'}
