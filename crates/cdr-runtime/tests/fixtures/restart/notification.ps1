param([string]$Variant)
$ErrorActionPreference='Stop'
$path=Join-Path $env:CDR_SOURCE 'scripts/Invoke-CdrRestartLiveQa.ps1'
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseInput([IO.File]::ReadAllText($path,[Text.Encoding]::UTF8),[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'QA script does not parse'}
$node=$ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Publish-Result'},$true)
. ([scriptblock]::Create($node.Extent.Text))
$root=$env:CDR_FIXTURE_ROOT;$folder=$root;$binary=Join-Path $root 'cdr-runtime.exe';$resultPath=Join-Path $root 'result.json'
$state=[pscustomobject]@{ThreadId='00000000-0000-0000-0000-000000000001'}
$result=[pscustomobject]@{Notification='not_attempted';Phase='passed';Detail=''}
$script:sends=0;$script:checks=0;$script:saves=@()
function Save-Result($value){$script:saves+=@([string]$value.Notification)}
function Assert-Artifact {$script:checks++;if($Variant -eq 'changed'){throw 'Runtime artifact changed; QA refused'}}
function Write-AtomicRestartMarker($path,$text){[IO.File]::WriteAllText($path,$text,[Text.UTF8Encoding]::new($false))}
function Invoke-CdrNative {param($Executable,$Arguments,$TimeoutSeconds)
 if($Executable -cne $binary -or $Arguments[0] -cne '--admin' -or $Arguments[1] -cne 'send-attachment'){throw 'wrong native entry'}
 if($Arguments -notcontains '--thread-ref' -or $Arguments -notcontains $state.ThreadId -or $Arguments -notcontains $resultPath){throw 'wrong target or attachment'}
 if($result.Notification -cne 'attempting'){throw 'send intent not saved'}
 $script:sends++
 if($Variant -eq 'failure'){throw 'Discord send failed: HTTP 503; not retried'}
 'DISCORD_ATTACHMENT_SENT'
}
$observed=''
try {Publish-Result}catch{$observed=$_.Exception.Message}
switch($Variant){
 'success' {
   if($observed -or $result.Notification -cne 'sent' -or $script:sends -ne 1 -or $script:checks -ne 1){throw "native delivery failed: $observed"}
   if(($script:saves -join ',') -cne 'attempting,sent'){throw 'receipt ordering changed'}
   Publish-Result
   if($script:sends -ne 1){throw 'duplicate notification'}
   if(-not [IO.File]::ReadAllText((Join-Path $root 'notification.txt'),[Text.Encoding]::UTF8).Contains('재시작 시험 PASS')){throw 'Korean caption corrupted'}
 }
 'failure' {
   if($observed -notmatch 'HTTP 503' -or $result.NotificationError -notmatch 'HTTP 503'){throw "actual upload error lost: $observed"}
   if($result.Notification -cne 'failed_or_unknown_no_retry'){throw 'failure receipt not retained'}
   Publish-Result
   if($script:sends -ne 1){throw 'failed send retried'}
 }
 'changed' {
   if($observed -notmatch 'artifact changed' -or $script:sends -ne 0 -or $result.Notification -cne 'not_attempted'){throw 'unverified executable invoked'}
 }
 default {throw 'unknown test variant'}
}
