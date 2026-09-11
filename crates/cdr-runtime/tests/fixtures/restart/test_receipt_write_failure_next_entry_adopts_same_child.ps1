$realAtomic=(Get-Command Write-AtomicRestartMarker).ScriptBlock
$script:failReceipt=$true
function Write-AtomicRestartMarker {param($Path,$Text)
 if($Path.EndsWith('.restart.completed') -and $script:failReceipt){$script:failReceipt=$false;throw 'injected receipt failure'}
 & $realAtomic -Path $Path -Text $Text
}
try{& $entry;throw 'receipt error hidden'}catch{if($_.Exception.Message -notmatch 'injected receipt failure'){throw}}
if($script:starts -ne 1){throw 'first launch missing'}
$script:CdrLaunchJournalPath=$null;$script:RustRestartStartedProcess=$null
& $entry
if($script:starts -ne 1){throw 'duplicate launch'}
if(Test-Path (Join-Path $root '.codex_discord_rust.restart.launch')){throw 'journal not completed'}
if(-not (Test-Path (Join-Path $root '.codex_discord_rust.restart.completed'))){throw 'receipt missing'}
