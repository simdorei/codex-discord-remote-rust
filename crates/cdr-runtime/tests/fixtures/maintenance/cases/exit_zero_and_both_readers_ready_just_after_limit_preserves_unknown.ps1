param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'NATIVE.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$script:commandBase=[DateTimeOffset]::UtcNow;$script:observedLate=$false
function Get-CdrCommandUtcNow {
 if($script:observedLate){$script:commandBase.AddSeconds(2)}else{$script:commandBase}
}
function Wait-CdrCommandExit($Process,$OutReader,$ErrReader) {
 if(-not $Process.WaitForExit(5000)){throw 'fixture child hung'}
 [void]$OutReader.GetAwaiter().GetResult();[void]$ErrReader.GetAwaiter().GetResult()
 $script:observedLate=$true
 return $true
}
try{Invoke-CdrMaintenanceCommand $s $shell @('-NoProfile','-Command','exit 0') 1;throw 'late completed child accepted'}
catch{if($_.Exception.Message -notmatch 'command_deadline_outcome_unknown'){throw}}
if((Read-CdrMaintenanceState $StatePath).ActiveCommand.Phase -ne 'child'){throw 'late child record lost'}
if((Read-CdrMaintenanceState $StatePath).Phase -ne 'planned'){throw 'advanced after expired observation'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
