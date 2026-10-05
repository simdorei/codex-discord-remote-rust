$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'CdrDesktopRecovery.ps1')
. (Join-Path $PSScriptRoot 'CdrToolsRecovery.ps1')
function Assert($Condition,$Message) { if (-not $Condition) { throw $Message } }
# All process operations below are in-memory fakes. No process is started/stopped.
$script:events=[Collections.Generic.List[string]]::new()
function Fake-Process([int]$Number) {
    $p=[pscustomobject]@{Id=$Number;StartTime=[DateTime]::UtcNow;HasExited=$false;Handle=1;ExitCode=0}
    $p | Add-Member ScriptMethod Dispose {}
    $p | Add-Member ScriptMethod Refresh {}
    $p | Add-Member ScriptMethod CloseMainWindow { return $false }
    $p | Add-Member ScriptMethod WaitForExit { param($Timeout) return $true }
    $p | Add-Member ScriptMethod Kill { $script:events.Add("stop:$($this.Id)");$this.HasExited=$true }
    return $p
}
$script:fixtureDesktop=Fake-Process 20;$script:fixtureBackend=Fake-Process 21
$script:fixtureReplacement=Fake-Process 30;$script:fixtureWorker=Fake-Process 40
$testRoot=Join-Path ([IO.Path]::GetTempPath()) ('cdr-tools-fixture-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($testRoot)
$script:fixturePlan=[pscustomobject]@{State='tools_recovery';ThreadId='aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee';RepoRoot=$testRoot;
    CodexHome=$testRoot;LockPath=(Join-Path $testRoot 'unused.lock');BotIdentity='10|11';
    DesktopIdentity=(Get-CdrRecoveryIdentity $script:fixtureDesktop);DesktopPath='fixture-desktop.exe';RecoveryIdentity='exact-hosts'}
$plan=$script:fixturePlan
$script:receipt=$null
function Get-CdrToolsRecoveryPlan { return $script:fixturePlan }
function Get-CdrOwnedProcessHandles { return ,@($script:fixtureDesktop,$script:fixtureBackend) }
function Get-Process { return $script:fixtureDesktop }
function Get-CdrWriterOwners { return @() }
function Get-CimInstance { return [pscustomobject]@{ProcessId=31} }
function Save-CdrDesktopRecoveryReceipt { param($Path,$Receipt) $script:receipt=$Receipt }
function Start-Process {
    param($FilePath,$WindowStyle,[switch]$PassThru,$ArgumentList)
    Assert ($WindowStyle -ceq 'Hidden') 'Helper must be hidden.'
    if ($FilePath -ceq $script:fixturePlan.DesktopPath) { $script:events.Add('start:desktop');return $script:fixtureReplacement }
    Assert ($ArgumentList -contains '-ForceWorker') 'Bot restart must use the independent force worker.'
    $script:events.Add('start:bot-controller')
    [IO.File]::WriteAllText((Join-Path $testRoot '.codex_discord_rust.force.completed'),
        (@{InterruptedIdentity='10|11';ReplacementIdentity='50|51'} | ConvertTo-Json))
    return $script:fixtureWorker
}
Invoke-CdrToolsRestart $plan 'memory-receipt'
Assert ($script:receipt.Phase -ceq 'restarted') 'Full restart did not complete.'
Assert (-not $script:receipt.ToolProbeVerified) 'Restart must not claim a tool probe succeeded.'
Assert (($script:events -join ',') -ceq 'stop:20,stop:21,start:bot-controller,start:desktop') 'Wrong stop/start order or duplicate launch.'
$stale=$plan | ConvertTo-Json | ConvertFrom-Json;$stale.BotIdentity='90|91'
$refused=$false;try { Invoke-CdrToolsRestart $stale 'memory-receipt' } catch { $refused=$true }
Assert $refused 'Changed bot identity was not refused.'
Assert ($script:events.Count -eq 4) 'Stale identity caused a process action.'
$plan.DesktopIdentity='stopped';$script:events.Clear()
Invoke-CdrToolsRestart $plan 'memory-receipt'
Assert (($script:events -join ',') -ceq 'start:bot-controller,start:desktop') 'Closed desktop must still start.'
$script:fixtureWorker.ExitCode=1;$script:events.Clear()
Invoke-CdrToolsRestart $plan 'memory-receipt'
Assert ($script:receipt.Phase -ceq 'failed') 'Bot failure was reported as success.'
Assert ($script:events.Contains('start:desktop')) 'Desktop must be restored after bot failure.'
$script:fixtureWorker.ExitCode=0
function Get-CdrWriterOwners { return @([pscustomobject]@{ReplacementWriter=$true}) }
Invoke-CdrToolsRestart $plan 'memory-receipt'
Assert ($script:receipt.Phase -ceq 'restarted') 'Replacement writer must not hide a verified host restart.'
Assert (-not $script:receipt.WriterReleased) 'Replacement writer observation must remain explicit.'
$script:fixtureReplacement.HasExited=$true;$script:events.Clear()
$refused=$false;try { Invoke-CdrToolsRestart $plan 'memory-receipt' } catch { $refused=$true }
Assert $refused 'Exited replacement desktop was reported as ready.'
Assert ($script:receipt.Phase -ceq 'restarting_desktop') 'Exited replacement was marked as successfully restarted.'
Assert ($script:events.Contains('start:desktop')) 'Replacement startup was not attempted.'
$script:fixtureReplacement.HasExited=$false;$script:events.Clear()
$script:fixtureWorker | Add-Member ScriptMethod WaitForExit { param($Timeout) return $false } -Force
Invoke-CdrToolsRestart $plan 'memory-receipt'
Assert ($script:receipt.Phase -ceq 'failed') 'Unconfirmed bot restart was reported as success.'
Assert ($script:receipt.Error -like '*unverified*') 'Unconfirmed bot restart lost its diagnostic.'
Assert ($script:events.Contains('start:desktop')) 'Desktop restoration was skipped after an unconfirmed bot restart.'
Assert (-not $script:receipt.ToolProbeVerified) 'Unconfirmed host restart claimed tool-probe success.'
Write-Output 'full_recovery_tests_passed'
