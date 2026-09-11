param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'),[Text.Encoding]::UTF8)))
switch ($Variant) {
'0' {
$s.LastError='older warning without first-completion evidence'
Save-CdrCompletionFailureAudit $s -FromActiveState
$path=Join-Path $s.Bundle 'completion-active-failure.json'
$copy=Get-Content $path -Raw -Encoding UTF8|ConvertFrom-Json
if(-not $copy.PSObject.Properties['First'] -or $null -ne $copy.First){throw 'missing first was invented'}
$s.LastError='first actual completion error'
$s|Add-Member FirstCompletionFailure (Get-CdrCompletionFailureSnapshot $s)
Save-CdrCompletionFailureAudit $s -FromActiveState
$copy=Get-Content $path -Raw -Encoding UTF8|ConvertFrom-Json
if($copy.First.LastError -cne $s.LastError -or $copy.Snapshot.LastError -cne $s.LastError){throw 'first was not enriched'}
}
default { throw "Unknown native fixture variant: $Variant" }
}
