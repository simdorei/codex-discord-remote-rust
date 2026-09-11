param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'BOUNDARY.ps1'), [Text.Encoding]::UTF8)))
. (Join-Path $env:V2_SOURCE 'codex-discord-rust-drain.ps1')
$s.Phase='stop_requested'
function Get-Process {param($Id,$Name,$ErrorAction) if($Id -eq 42){[pscustomobject]@{Id=42}}}
function Get-VerifiedRuntimeIdentity {'42|99'}
try{Invoke-CdrMaintenanceStop $s;throw 'missing ACK accepted'}catch{if($_.Exception.Message -notmatch 'fence changed'){throw}}
if(Test-Path $StopPath){throw 'stop published without ACK'}
