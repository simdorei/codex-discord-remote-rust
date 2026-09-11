param([string]$Variant)
switch ($Variant) {
'0' {
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceFailure.ps1')
$s=Read-CdrMaintenanceState $StatePath;$s.LastError='fixture'
$DrainAckPath=Join-Path $RepoRoot 'ack';$StopPath=Join-Path $RepoRoot 'stop'
function Get-Process {param($Id,$ErrorAction) $null}
$text=$op
if('exact' -eq 'newline'){$text+="`n"}
if('exact' -eq 'space'){$text=' '+$text}
[IO.File]::WriteAllText($StopPath,$text)
$guard=$null
try {
 if('exact' -eq 'unreadable'){$guard=[IO.File]::Open($StopPath,'Open','ReadWrite','None')}
 $observation=Get-CdrMaintenanceFailureObservation $s
 $expected=if('exact' -eq 'exact'){'owned'}elseif('exact' -eq 'unreadable'){'unknown'}else{'foreign'}
 if($observation.Stop -cne $expected){throw 'stop observation differs from owner guard'}
}finally{if($guard){$guard.Dispose()}}
}
'1' {
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceFailure.ps1')
$s=Read-CdrMaintenanceState $StatePath;$s.LastError='fixture'
$DrainAckPath=Join-Path $RepoRoot 'ack';$StopPath=Join-Path $RepoRoot 'stop'
function Get-Process {param($Id,$ErrorAction) $null}
$text=$op
if('newline' -eq 'newline'){$text+="`n"}
if('newline' -eq 'space'){$text=' '+$text}
[IO.File]::WriteAllText($StopPath,$text)
$guard=$null
try {
 if('newline' -eq 'unreadable'){$guard=[IO.File]::Open($StopPath,'Open','ReadWrite','None')}
 $observation=Get-CdrMaintenanceFailureObservation $s
 $expected=if('newline' -eq 'exact'){'owned'}elseif('newline' -eq 'unreadable'){'unknown'}else{'foreign'}
 if($observation.Stop -cne $expected){throw 'stop observation differs from owner guard'}
}finally{if($guard){$guard.Dispose()}}
}
'2' {
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceFailure.ps1')
$s=Read-CdrMaintenanceState $StatePath;$s.LastError='fixture'
$DrainAckPath=Join-Path $RepoRoot 'ack';$StopPath=Join-Path $RepoRoot 'stop'
function Get-Process {param($Id,$ErrorAction) $null}
$text=$op
if('space' -eq 'newline'){$text+="`n"}
if('space' -eq 'space'){$text=' '+$text}
[IO.File]::WriteAllText($StopPath,$text)
$guard=$null
try {
 if('space' -eq 'unreadable'){$guard=[IO.File]::Open($StopPath,'Open','ReadWrite','None')}
 $observation=Get-CdrMaintenanceFailureObservation $s
 $expected=if('space' -eq 'exact'){'owned'}elseif('space' -eq 'unreadable'){'unknown'}else{'foreign'}
 if($observation.Stop -cne $expected){throw 'stop observation differs from owner guard'}
}finally{if($guard){$guard.Dispose()}}
}
'3' {
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceFailure.ps1')
$s=Read-CdrMaintenanceState $StatePath;$s.LastError='fixture'
$DrainAckPath=Join-Path $RepoRoot 'ack';$StopPath=Join-Path $RepoRoot 'stop'
function Get-Process {param($Id,$ErrorAction) $null}
$text=$op
if('unreadable' -eq 'newline'){$text+="`n"}
if('unreadable' -eq 'space'){$text=' '+$text}
[IO.File]::WriteAllText($StopPath,$text)
$guard=$null
try {
 if('unreadable' -eq 'unreadable'){$guard=[IO.File]::Open($StopPath,'Open','ReadWrite','None')}
 $observation=Get-CdrMaintenanceFailureObservation $s
 $expected=if('unreadable' -eq 'exact'){'owned'}elseif('unreadable' -eq 'unreadable'){'unknown'}else{'foreign'}
 if($observation.Stop -cne $expected){throw 'stop observation differs from owner guard'}
}finally{if($guard){$guard.Dispose()}}
}
default { throw "Unknown native fixture variant: $Variant" }
}
