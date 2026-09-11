function Enter-RestartDrain {throw 'DRAIN MUST NOT BE CALLED'}
try{& $entry;throw 'foreign marker accepted'}catch{if($_.Exception.Message -notmatch 'explicit preparation refused before drain'){throw}}
if([IO.File]::ReadAllText($RestartPath) -cne 'foreign-invalid'){throw 'foreign marker changed'}
