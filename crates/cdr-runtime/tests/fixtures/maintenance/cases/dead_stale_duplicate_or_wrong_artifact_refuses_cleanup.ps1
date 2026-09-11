param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PREPARED.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$script:childAlive=$false
try{Complete-CdrMaintenance $s $StatePath;throw 'invalid proof accepted'}catch{if($_.Exception.Message -eq 'invalid proof accepted'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'invalid proof unsealed'}
}
'1' {
function Get-HeartbeatHealth {[pscustomobject]@{Healthy=$false;Bootstrap=$false}}
try{Complete-CdrMaintenance $s $StatePath;throw 'invalid proof accepted'}catch{if($_.Exception.Message -eq 'invalid proof accepted'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'invalid proof unsealed'}
}
'2' {
function Get-Process {param($Id,$Name,$ErrorAction) if($Name){@([pscustomobject]@{Id=77;Path=$BinaryPath},[pscustomobject]@{Id=88;Path=$BinaryPath})}elseif($Id -eq 77){[pscustomobject]@{Id=77;Path=$BinaryPath}}}
try{Complete-CdrMaintenance $s $StatePath;throw 'invalid proof accepted'}catch{if($_.Exception.Message -eq 'invalid proof accepted'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'invalid proof unsealed'}
}
'3' {
[IO.File]::WriteAllText($BinaryPath,'changed')
try{Complete-CdrMaintenance $s $StatePath;throw 'invalid proof accepted'}catch{if($_.Exception.Message -eq 'invalid proof accepted'){throw}}
if(-not (Test-Path $StatePath) -or -not (Test-Path $DisablePath)){throw 'invalid proof unsealed'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
