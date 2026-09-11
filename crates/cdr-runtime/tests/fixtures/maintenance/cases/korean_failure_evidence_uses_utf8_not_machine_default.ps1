param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'PRODUCTION_FAILURE_FIELDS.ps1'), [Text.Encoding]::UTF8)))
$s.LastError='잠금 실패 — 한글 보존'
$PSDefaultParameterValues['Get-Content:Encoding']='Ascii'
Save-CdrCompletionFailureAudit $s
$saved=[IO.File]::ReadAllText((Join-Path $s.Bundle 'completion-failure.json'))|ConvertFrom-Json
if($saved.First.LastError -cne $s.LastError -or $saved.Latest.LastError -cne $s.LastError){throw 'Korean failure evidence corrupted'}
