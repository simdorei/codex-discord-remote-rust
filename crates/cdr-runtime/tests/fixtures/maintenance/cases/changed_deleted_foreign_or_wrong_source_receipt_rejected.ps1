param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'BACKUP.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
Invoke-CdrMaintenancePreStopBackup $s $StatePath
switch('changed'){
 'changed'{[IO.File]::WriteAllText($snapshot,'corrupted')}
 'deleted'{[IO.File]::Delete($snapshot)}
 'foreign'{$s.PreStopBackup.Operation='f'*32}
 'source'{$s.PreStopBackup.SourceDb=Join-Path $RepoRoot 'wrong.sqlite'}
}
$rejected=$false
try{Assert-CdrMaintenancePreStopBackup $s}catch{$rejected=$true}
if(-not $rejected){throw 'bad backup accepted'}
}
'1' {
Invoke-CdrMaintenancePreStopBackup $s $StatePath
switch('deleted'){
 'changed'{[IO.File]::WriteAllText($snapshot,'corrupted')}
 'deleted'{[IO.File]::Delete($snapshot)}
 'foreign'{$s.PreStopBackup.Operation='f'*32}
 'source'{$s.PreStopBackup.SourceDb=Join-Path $RepoRoot 'wrong.sqlite'}
}
$rejected=$false
try{Assert-CdrMaintenancePreStopBackup $s}catch{$rejected=$true}
if(-not $rejected){throw 'bad backup accepted'}
}
'2' {
Invoke-CdrMaintenancePreStopBackup $s $StatePath
switch('foreign'){
 'changed'{[IO.File]::WriteAllText($snapshot,'corrupted')}
 'deleted'{[IO.File]::Delete($snapshot)}
 'foreign'{$s.PreStopBackup.Operation='f'*32}
 'source'{$s.PreStopBackup.SourceDb=Join-Path $RepoRoot 'wrong.sqlite'}
}
$rejected=$false
try{Assert-CdrMaintenancePreStopBackup $s}catch{$rejected=$true}
if(-not $rejected){throw 'bad backup accepted'}
}
'3' {
Invoke-CdrMaintenancePreStopBackup $s $StatePath
switch('source'){
 'changed'{[IO.File]::WriteAllText($snapshot,'corrupted')}
 'deleted'{[IO.File]::Delete($snapshot)}
 'foreign'{$s.PreStopBackup.Operation='f'*32}
 'source'{$s.PreStopBackup.SourceDb=Join-Path $RepoRoot 'wrong.sqlite'}
}
$rejected=$false
try{Assert-CdrMaintenancePreStopBackup $s}catch{$rejected=$true}
if(-not $rejected){throw 'bad backup accepted'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
