param([string]$Variant)
switch ($Variant) {
'0' {
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceSchedule.ps1')
$s=Read-CdrMaintenanceState $StatePath
$identity=[Security.Principal.WindowsIdentity]::GetCurrent()
$s|Add-Member TaskUser $identity.Name
$s|Add-Member PowerShellPath (Get-Command powershell.exe).Source
$actual=switch('full'){
 'full'{$identity.Name}
 'short'{$identity.Name.Split('\')[-1]}
 'sid'{$identity.User.Value}
 'foreign'{'S-1-5-18'}
}
if('full' -eq 'foreign' -and $identity.User.Value -eq $actual){throw 'fixture unexpectedly runs as LocalSystem'}
function Get-ScheduledTask {param($TaskName,$ErrorAction)
 [pscustomobject]@{
  Settings=[pscustomobject]@{Enabled=$true}
  Actions=@([pscustomobject]@{Execute=$s.PowerShellPath;Arguments=(Get-CdrMaintenanceTaskArguments $StatePath $s.Operation);WorkingDirectory=$RepoRoot})
  Principal=[pscustomobject]@{UserId=$actual}
  Triggers=@([pscustomobject]@{Enabled=$true;Repetition=[pscustomobject]@{Interval='PT1M';Duration='PT30M'}})
 }
}
if('full' -eq 'foreign'){
 try{Assert-CdrMaintenanceRecoveryArmed $s;throw 'foreign task owner accepted'}
 catch{if($_.Exception.Message -notmatch 'independent_recovery_not_armed'){throw}}
}else{Assert-CdrMaintenanceRecoveryArmed $s}
}
'1' {
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceSchedule.ps1')
$s=Read-CdrMaintenanceState $StatePath
$identity=[Security.Principal.WindowsIdentity]::GetCurrent()
$s|Add-Member TaskUser $identity.Name
$s|Add-Member PowerShellPath (Get-Command powershell.exe).Source
$actual=switch('short'){
 'full'{$identity.Name}
 'short'{$identity.Name.Split('\')[-1]}
 'sid'{$identity.User.Value}
 'foreign'{'S-1-5-18'}
}
if('short' -eq 'foreign' -and $identity.User.Value -eq $actual){throw 'fixture unexpectedly runs as LocalSystem'}
function Get-ScheduledTask {param($TaskName,$ErrorAction)
 [pscustomobject]@{
  Settings=[pscustomobject]@{Enabled=$true}
  Actions=@([pscustomobject]@{Execute=$s.PowerShellPath;Arguments=(Get-CdrMaintenanceTaskArguments $StatePath $s.Operation);WorkingDirectory=$RepoRoot})
  Principal=[pscustomobject]@{UserId=$actual}
  Triggers=@([pscustomobject]@{Enabled=$true;Repetition=[pscustomobject]@{Interval='PT1M';Duration='PT30M'}})
 }
}
if('short' -eq 'foreign'){
 try{Assert-CdrMaintenanceRecoveryArmed $s;throw 'foreign task owner accepted'}
 catch{if($_.Exception.Message -notmatch 'independent_recovery_not_armed'){throw}}
}else{Assert-CdrMaintenanceRecoveryArmed $s}
}
'2' {
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceSchedule.ps1')
$s=Read-CdrMaintenanceState $StatePath
$identity=[Security.Principal.WindowsIdentity]::GetCurrent()
$s|Add-Member TaskUser $identity.Name
$s|Add-Member PowerShellPath (Get-Command powershell.exe).Source
$actual=switch('sid'){
 'full'{$identity.Name}
 'short'{$identity.Name.Split('\')[-1]}
 'sid'{$identity.User.Value}
 'foreign'{'S-1-5-18'}
}
if('sid' -eq 'foreign' -and $identity.User.Value -eq $actual){throw 'fixture unexpectedly runs as LocalSystem'}
function Get-ScheduledTask {param($TaskName,$ErrorAction)
 [pscustomobject]@{
  Settings=[pscustomobject]@{Enabled=$true}
  Actions=@([pscustomobject]@{Execute=$s.PowerShellPath;Arguments=(Get-CdrMaintenanceTaskArguments $StatePath $s.Operation);WorkingDirectory=$RepoRoot})
  Principal=[pscustomobject]@{UserId=$actual}
  Triggers=@([pscustomobject]@{Enabled=$true;Repetition=[pscustomobject]@{Interval='PT1M';Duration='PT30M'}})
 }
}
if('sid' -eq 'foreign'){
 try{Assert-CdrMaintenanceRecoveryArmed $s;throw 'foreign task owner accepted'}
 catch{if($_.Exception.Message -notmatch 'independent_recovery_not_armed'){throw}}
}else{Assert-CdrMaintenanceRecoveryArmed $s}
}
'3' {
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceSchedule.ps1')
$s=Read-CdrMaintenanceState $StatePath
$identity=[Security.Principal.WindowsIdentity]::GetCurrent()
$s|Add-Member TaskUser $identity.Name
$s|Add-Member PowerShellPath (Get-Command powershell.exe).Source
$actual=switch('foreign'){
 'full'{$identity.Name}
 'short'{$identity.Name.Split('\')[-1]}
 'sid'{$identity.User.Value}
 'foreign'{'S-1-5-18'}
}
if('foreign' -eq 'foreign' -and $identity.User.Value -eq $actual){throw 'fixture unexpectedly runs as LocalSystem'}
function Get-ScheduledTask {param($TaskName,$ErrorAction)
 [pscustomobject]@{
  Settings=[pscustomobject]@{Enabled=$true}
  Actions=@([pscustomobject]@{Execute=$s.PowerShellPath;Arguments=(Get-CdrMaintenanceTaskArguments $StatePath $s.Operation);WorkingDirectory=$RepoRoot})
  Principal=[pscustomobject]@{UserId=$actual}
  Triggers=@([pscustomobject]@{Enabled=$true;Repetition=[pscustomobject]@{Interval='PT1M';Duration='PT30M'}})
 }
}
if('foreign' -eq 'foreign'){
 try{Assert-CdrMaintenanceRecoveryArmed $s;throw 'foreign task owner accepted'}
 catch{if($_.Exception.Message -notmatch 'independent_recovery_not_armed'){throw}}
}else{Assert-CdrMaintenanceRecoveryArmed $s}
}
default { throw "Unknown native fixture variant: $Variant" }
}
