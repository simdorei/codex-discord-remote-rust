$source=[IO.File]::ReadAllText((Join-Path $env:V2_SOURCE 'scripts/Restart-CdrAfterQueueIdle.ps1'))
$root=$RepoRoot; $CheckOnly=$false
function Assert-OriginalRuntime {[pscustomobject]@{Id=42}}
try {& ([scriptblock]::Create($source.Substring($source.IndexOf('Set-Location -LiteralPath $root')))); throw 'unsafe helper accepted'}
catch {if($_.Exception.Message -notmatch 'Queue-only restart is retired'){throw}}
if(Get-ChildItem -LiteralPath $root -Force){throw 'helper wrote a marker'}
