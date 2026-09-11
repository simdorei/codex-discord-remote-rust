param([string]$Variant)
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceState.ps1')
$RepoRoot=$env:V2_SOURCE
try{Assert-CdrCertifiedBaseline '75F713070CA47D014A99E3FF2A73B7D336E2B7529B4A28064E6393FE8EB08E8A';throw 'unproven accepted'}
catch{if($_.Exception.Message -notmatch 'T1_installed_drain_contract_unproven'){throw}}
