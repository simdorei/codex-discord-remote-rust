param([string]$Variant)
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'NATIVE.ps1'),[Text.Encoding]::UTF8)))
$EnvPath=Join-Path $RepoRoot '.env'
$env:CDR_DIAGNOSTIC_TEST_TOKEN='synthetic-process-secret-123456789'
$env:CDR_DIAGNOSTIC_OTHER_TOKEN='synthetic-process-secret-987654321'
[IO.File]::WriteAllText($EnvPath,'DISCORD_BOT_TOKEN=synthetic-env-secret-987654321',[Text.UTF8Encoding]::new($false))
$script:reason='app-server request initialize timed out after 8000 ms'
if($Variant -eq 'unicode'){$script:reason='연결 초기화 실패: 제한시간 초과'}
$payload=$script:reason
if($Variant -eq 'secrets'){$payload+=' synthetic-process-secret-123456789 synthetic-process-secret-987654321 synthetic-env-secret-987654321 Authorization: Bearer fake-bearer-credential-1234567890'}
if($Variant -eq 'bounded'){$payload+=' '+('x'*100000)}
$childCode="[Console]::OutputEncoding=[Text.UTF8Encoding]::new(`$false);[Console]::Error.WriteLine('$payload');exit 7"
$script:encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($childCode))
# Keep the command line bounded even when testing large diagnostic output.
if($Variant -eq 'bounded'){
 $childCode="[Console]::Error.WriteLine('$($script:reason) '+('x'*100000));exit 7"
 $script:encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($childCode))
}
function Invoke-CdrMaintenancePreflight {param($state) Invoke-CdrMaintenanceCommand $state $shell @('-NoProfile','-EncodedCommand',$script:encoded) 10}
$captured=[Collections.Generic.List[string]]::new()
try{
 Invoke-CdrMaintenanceEngine $StatePath $op 6>&1|ForEach-Object {$captured.Add([string]$_)}
 throw 'nonzero preflight was accepted'
}catch{if($_.Exception.Message -notmatch 'native_command_failed exit=7'){throw}}
$saved=Read-CdrMaintenanceState $StatePath
if(-not $saved.LastError.Contains($script:reason)){throw 'ERR-1: original native stderr reason was not persisted'}
if($null -ne $saved.ActiveCommand){throw 'definite exit left unknown child'}
if($script:calls -match ':(ACK|stop|install|launch)$'){throw 'preflight failure performed a deployment action'}
$text=([IO.File]::ReadAllText($StatePath))+($captured -join "`n")
foreach($secret in @('synthetic-process-secret-123456789','synthetic-process-secret-987654321','synthetic-env-secret-987654321','fake-bearer-credential-1234567890')){
 if($text.Contains($secret)){throw 'ERR-2: a synthetic credential leaked to state or console'}
}
if($saved.LastError.Length -gt 2500 -or (Get-Item -LiteralPath $StatePath).Length -gt 32768){throw 'ERR-2: saved native diagnostic is unbounded'}
if($Variant -eq 'bounded' -and -not $saved.LastError.Contains('[truncated]')){throw 'truncation was not disclosed'}
