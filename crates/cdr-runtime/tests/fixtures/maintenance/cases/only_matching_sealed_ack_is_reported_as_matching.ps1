param([string]$Variant)
switch ($Variant) {
'0' {
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceFailure.ps1')
. (Join-Path $env:V2_SOURCE 'codex-discord-rust-drain.ps1')
$s=Read-CdrMaintenanceState $StatePath;$s.LastError='fixture_failure'
$DrainAckPath=Join-Path $RepoRoot 'ack';$StopPath=Join-Path $RepoRoot 'stop'
function Get-Process {param($Id,$ErrorAction) $null}
$raw="version=1`nruntime_id=runtime1`nprocess_identity=42|99`nnonce=$op`n"
if('sealed' -ne 'missing'){$raw+="state=sealed`n"}
if('sealed' -eq 'foreign'){$raw=$raw.Replace('state=foreign','state=sealed').Replace($op,('f'*32))}
[IO.File]::WriteAllText($DrainAckPath,$raw)
$observation=Get-CdrMaintenanceFailureObservation $s
if('sealed' -eq 'sealed'){
 if($observation.Ack -cne 'matching'){throw 'valid ACK not observed'}
}elseif($observation.Ack -ceq 'matching'){throw 'unsealed ACK reported confirmed'}
}
'1' {
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceFailure.ps1')
. (Join-Path $env:V2_SOURCE 'codex-discord-rust-drain.ps1')
$s=Read-CdrMaintenanceState $StatePath;$s.LastError='fixture_failure'
$DrainAckPath=Join-Path $RepoRoot 'ack';$StopPath=Join-Path $RepoRoot 'stop'
function Get-Process {param($Id,$ErrorAction) $null}
$raw="version=1`nruntime_id=runtime1`nprocess_identity=42|99`nnonce=$op`n"
if('open' -ne 'missing'){$raw+="state=open`n"}
if('open' -eq 'foreign'){$raw=$raw.Replace('state=foreign','state=sealed').Replace($op,('f'*32))}
[IO.File]::WriteAllText($DrainAckPath,$raw)
$observation=Get-CdrMaintenanceFailureObservation $s
if('open' -eq 'sealed'){
 if($observation.Ack -cne 'matching'){throw 'valid ACK not observed'}
}elseif($observation.Ack -ceq 'matching'){throw 'unsealed ACK reported confirmed'}
}
'2' {
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceFailure.ps1')
. (Join-Path $env:V2_SOURCE 'codex-discord-rust-drain.ps1')
$s=Read-CdrMaintenanceState $StatePath;$s.LastError='fixture_failure'
$DrainAckPath=Join-Path $RepoRoot 'ack';$StopPath=Join-Path $RepoRoot 'stop'
function Get-Process {param($Id,$ErrorAction) $null}
$raw="version=1`nruntime_id=runtime1`nprocess_identity=42|99`nnonce=$op`n"
if('missing' -ne 'missing'){$raw+="state=missing`n"}
if('missing' -eq 'foreign'){$raw=$raw.Replace('state=foreign','state=sealed').Replace($op,('f'*32))}
[IO.File]::WriteAllText($DrainAckPath,$raw)
$observation=Get-CdrMaintenanceFailureObservation $s
if('missing' -eq 'sealed'){
 if($observation.Ack -cne 'matching'){throw 'valid ACK not observed'}
}elseif($observation.Ack -ceq 'matching'){throw 'unsealed ACK reported confirmed'}
}
'3' {
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceFailure.ps1')
. (Join-Path $env:V2_SOURCE 'codex-discord-rust-drain.ps1')
$s=Read-CdrMaintenanceState $StatePath;$s.LastError='fixture_failure'
$DrainAckPath=Join-Path $RepoRoot 'ack';$StopPath=Join-Path $RepoRoot 'stop'
function Get-Process {param($Id,$ErrorAction) $null}
$raw="version=1`nruntime_id=runtime1`nprocess_identity=42|99`nnonce=$op`n"
if('foreign' -ne 'missing'){$raw+="state=foreign`n"}
if('foreign' -eq 'foreign'){$raw=$raw.Replace('state=foreign','state=sealed').Replace($op,('f'*32))}
[IO.File]::WriteAllText($DrainAckPath,$raw)
$observation=Get-CdrMaintenanceFailureObservation $s
if('foreign' -eq 'sealed'){
 if($observation.Ack -cne 'matching'){throw 'valid ACK not observed'}
}elseif($observation.Ack -ceq 'matching'){throw 'unsealed ACK reported confirmed'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
