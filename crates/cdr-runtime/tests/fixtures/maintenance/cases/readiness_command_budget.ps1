param([string]$Variant)
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceActions.ps1')
$s=Read-CdrMaintenanceState $StatePath
$script:available=120;$script:nativeCall=$null
function Get-CdrArtifactHash {param($path) return $s.CandidateHash}
function Get-CdrMaintenanceRemainingSeconds {param($state,$limit) return [Math]::Min($limit,$script:available)}
function Invoke-CdrMaintenanceCommand {param($state,$file,$arguments,$limit) $script:nativeCall=@{Arguments=$arguments;Limit=$limit}}
Invoke-CdrMaintenanceFullReadiness $s
if($script:nativeCall.Limit -ne 120){throw 'INIT-3: native deadline does not allow startup and close budgets'}
$arguments=$script:nativeCall.Arguments
$index=[Array]::IndexOf($arguments,'--restart-wait-timeout-seconds')
if($index -lt 0 -or [int]$arguments[$index+1] -gt 60){throw 'readiness wait exceeds the available command budget'}
$script:available=50;$script:nativeCall=$null
try{Invoke-CdrMaintenanceFullReadiness $s;throw 'insufficient startup budget was accepted'}
catch{if($_.Exception.Message -notmatch 'readiness_budget_insufficient'){throw}}
if($null -ne $script:nativeCall){throw 'insufficient budget still started native child'}
