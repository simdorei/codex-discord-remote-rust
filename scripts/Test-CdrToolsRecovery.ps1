$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'CdrDesktopRecovery.ps1')
. (Join-Path $PSScriptRoot 'CdrToolsRecovery.ps1')
function Assert($Condition,$Message) { if (-not $Condition) { throw $Message } }
$script:effectiveMcpReader=(Get-Item Function:\Get-CdrToolsEffectiveMcpServers).ScriptBlock
function Get-CdrToolsEffectiveMcpServers { return ,@() }
# Host process operations are in-memory fakes. The later reader tests start only
# their isolated compiled fixture EXE; no installed bot or desktop is touched.
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
    DesktopIdentity=(Get-CdrRecoveryIdentity $script:fixtureDesktop);DesktopPath='fixture-desktop.exe';RecoveryIdentity='exact-hosts'
    DesktopPackageFullName='OpenAI.Codex_1.0.0.0_x64__publisher';DesktopAppUserModelId='OpenAI.Codex_publisher!App'}
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
function Start-CdrDetachedProcess { throw 'Raw desktop launch loses package identity.' }
function Assert-CdrPackagedDesktopActivationAvailable {}
function Start-CdrPackagedDesktop {
    param($Plan)
    Assert ($Plan.DesktopAppUserModelId -ceq 'OpenAI.Codex_publisher!App') 'Unexpected registered app target.'
    $script:events.Add('start:desktop');return $script:fixtureReplacement
}
Invoke-CdrToolsRestart $plan 'memory-receipt'
Assert ($script:receipt.Phase -ceq 'restarted') 'Full restart did not complete.'
Assert (-not $script:receipt.ToolProbeVerified) 'Restart must not claim a tool probe succeeded.'
Assert (($script:events -join ',') -ceq 'stop:20,stop:21,start:desktop,start:bot-controller') 'The desktop host must be ready before the bot reads tool configuration.'
$stale=$plan | ConvertTo-Json | ConvertFrom-Json;$stale.BotIdentity='90|91'
$refused=$false;try { Invoke-CdrToolsRestart $stale 'memory-receipt' } catch { $refused=$true }
Assert $refused 'Changed bot identity was not refused.'
Assert ($script:events.Count -eq 4) 'Stale identity caused a process action.'
$plan.DesktopIdentity='stopped';$script:events.Clear()
Invoke-CdrToolsRestart $plan 'memory-receipt'
Assert (($script:events -join ',') -ceq 'start:desktop,start:bot-controller') 'Closed desktop must start before the bot.'
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
# Exercise the real bounded native-reader wrapper with an isolated EXE. It emits
# the CLI's normalized schema; it never starts an MCP server or uses real config.
Set-Item Function:\Get-CdrToolsEffectiveMcpServers -Value $script:effectiveMcpReader
$readerDesktop=Join-Path $testRoot 'package\app\ChatGPT.exe'
$readerExe=Join-Path $testRoot 'package\app\resources\codex.exe'
[void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($readerExe))
Add-Type -OutputAssembly $readerExe -OutputType ConsoleApplication -TypeDefinition @'
using System;
using System.Diagnostics;
using System.IO;
using System.Threading;
class CdrToolsFixtureReader {
    static int Main(string[] args) {
        if (args.Length != 3 || args[0] != "mcp" || args[1] != "list" || args[2] != "--json") return 12;
        string home = Environment.GetEnvironmentVariable("CODEX_HOME");
        if (home == null || Path.GetFullPath(Environment.CurrentDirectory) != Path.GetFullPath(home)) return 13;
        string mode = File.ReadAllText(Path.Combine(home, "reader-mode.txt"));
        if (mode == "timeout") {
            File.WriteAllText(Path.Combine(home, "reader-pid.txt"), Process.GetCurrentProcess().Id.ToString());
            Thread.Sleep(30000);
        }
        if (mode == "failure") {
            Console.Error.WriteLine("fixture-secret-must-not-be-logged");
            return 14;
        }
        Console.Write(File.ReadAllText(Path.Combine(home, "reader-output.json")));
        return 0;
    }
}
'@
$readerMode=Join-Path $testRoot 'reader-mode.txt'
$readerOutput=Join-Path $testRoot 'reader-output.json'
[IO.File]::WriteAllText($readerMode,'success')
function Set-ReaderServers([object[]]$Servers) {
    [IO.File]::WriteAllText($readerOutput,(ConvertTo-Json -InputObject @($Servers) -Depth 8))
}
function Test-FixtureNativeReady {
    param([int]$Milliseconds=5000)
    return Test-CdrToolsNativeReady $testRoot $testRoot $readerDesktop ([DateTime]::UtcNow.AddMilliseconds($Milliseconds))
}
function Native-Server([string]$Name,[bool]$Enabled,[string]$PipePath) {
    return @{name=$Name;enabled=$Enabled;transport=@{type='stdio';env=@{
        SKY_CUA_NATIVE_PIPE='1';SKY_CUA_NATIVE_PIPE_DIRECTORY=$PipePath};env_vars=@()}}
}
$pipeName='codex-computer-use-'+[guid]::NewGuid().ToString()
$livePipePath='\\.\pipe\'+$pipeName
$deadPipePath='\\.\pipe\codex-computer-use-'+[guid]::NewGuid().ToString()
$pipe=$null
$configPath=Join-Path $testRoot 'config.toml'
try {
    Set-ReaderServers @()
    Assert (Test-FixtureNativeReady) 'Empty effective configuration must not create a legacy registration.'
    # Saved TOML may still contain the old value; only the normalized active
    # configuration and the runtime's standalone override decide readiness.
    [IO.File]::WriteAllText($configPath,"[mcp_servers.node_repl]`nenabled = false`n[mcp_servers.node_repl.env]`nSKY_CUA_NATIVE_PIPE = '1'`nSKY_CUA_NATIVE_PIPE_DIRECTORY = '$deadPipePath'`n")
    Set-ReaderServers @((Native-Server 'node_repl' $false $deadPipePath))
    Assert (Test-FixtureNativeReady) 'Disabled legacy registration blocked recovery.'
    Set-ReaderServers @((Native-Server 'node_repl' $true $deadPipePath))
    Assert (Test-FixtureNativeReady) 'The runtime standalone override was ignored.'
    Set-ReaderServers @((Native-Server 'cdr_computer' $true $deadPipePath))
    Assert (Test-FixtureNativeReady) 'The bundled provider replacement retained a legacy native-pipe requirement.'
    Set-ReaderServers @((Native-Server 'disabled_native' $false $deadPipePath))
    Assert (Test-FixtureNativeReady) 'A disabled non-legacy server blocked recovery.'
    Set-ReaderServers @((Native-Server 'active_native' $true $livePipePath))
    Assert (-not (Test-FixtureNativeReady)) 'An active provider with a dead pipe was reported ready.'
    $pipe=[IO.Pipes.NamedPipeServerStream]::new($pipeName)
    Assert (Test-FixtureNativeReady) 'An active provider with a live pipe was not observed.'
    Set-ReaderServers @((Native-Server 'active_native' $true $deadPipePath))
    Assert (-not (Test-FixtureNativeReady)) 'An unrelated live pipe hid a stale active address.'
    $pluginVersion=Join-Path $testRoot 'plugins\cache\openai-bundled\unified-computer-use\fixture'
    [void][IO.Directory]::CreateDirectory($pluginVersion)
    $plugin=@{mcpServers=@{cua_repl=@{env=@{SKY_CUA_NATIVE_PIPE='1';SKY_CUA_NATIVE_PIPE_DIRECTORY=$livePipePath}}}}
    [IO.File]::WriteAllText((Join-Path $pluginVersion '.mcp.json'),($plugin | ConvertTo-Json -Depth 5))
    $unified=Native-Server 'cua_repl' $true $livePipePath
    Set-ReaderServers @((Native-Server 'active_native' $true $deadPipePath),$unified)
    Assert (-not (Test-FixtureNativeReady)) 'A live unified pipe hid another active provider''s dead pipe.'
    Set-ReaderServers @((Native-Server 'node_repl' $true $deadPipePath),$unified)
    Assert (Test-FixtureNativeReady) 'A live unified pipe was blocked by overridden legacy configuration.'
    Set-ReaderServers @((Native-Server 'node_repl' $false $deadPipePath),$unified)
    Assert (Test-FixtureNativeReady) 'A live unified pipe was blocked by disabled legacy configuration.'
    Set-ReaderServers @($unified)
    Assert (Test-FixtureNativeReady) 'Unified CUA native pipe was not detected.'
    $pipe.Dispose();$pipe=$null
    Assert (-not (Test-FixtureNativeReady)) 'A dead unified CUA pipe was reported ready.'
    # Cached files are not evidence of active installation or enablement. The
    # CLI resolves plugin selection and returns only the effective transports.
    $unified.enabled=$false
    Set-ReaderServers @($unified)
    Assert (Test-FixtureNativeReady) 'A disabled plugin with cached files blocked readiness.'
    Set-ReaderServers @()
    Assert (Test-FixtureNativeReady) 'An absent/uninstalled plugin with cached files blocked readiness.'
    $unified.enabled=$true
    Set-ReaderServers @($unified)
    $pipe=[IO.Pipes.NamedPipeServerStream]::new($pipeName)
    $newerVersion=Join-Path ([IO.Path]::GetDirectoryName($pluginVersion)) 'not-installed'
    [void][IO.Directory]::CreateDirectory($newerVersion)
    $plugin.mcpServers.cua_repl.env.SKY_CUA_NATIVE_PIPE_DIRECTORY=$deadPipePath
    $newerManifest=Join-Path $newerVersion '.mcp.json'
    [IO.File]::WriteAllText($newerManifest,($plugin | ConvertTo-Json -Depth 5))
    [IO.File]::SetLastWriteTimeUtc($newerManifest,[DateTime]::UtcNow.AddMinutes(1))
    Assert (Test-FixtureNativeReady) 'A newer unused cache manifest replaced the active live endpoint.'
    $pipe.Dispose();$pipe=$null
    Set-ReaderServers @((Native-Server 'active_native' $true ''))
    Assert (-not (Test-FixtureNativeReady)) 'An active native provider without an address was reported ready.'
    Set-ReaderServers @(@{name='active_native';enabled=$true;transport=@{type='stdio';env=@{};env_vars=@('SKY_CUA_NATIVE_PIPE')}})
    Assert (-not (Test-FixtureNativeReady)) 'An unresolved inherited native setting was reported ready.'
    foreach ($json in @('not json','{}','[{}]','[{"name":"bad","enabled":"false"}]','[{"name":"bad","enabled":true}]')) {
        [IO.File]::WriteAllText($readerOutput,$json)
        Assert (-not (Test-FixtureNativeReady)) 'Malformed effective configuration was reported ready.'
    }
    [IO.File]::WriteAllText($readerMode,'failure')
    Assert (-not (Test-FixtureNativeReady)) 'A failed configuration reader was reported ready.'
    $diagnostic=''
    try { Get-CdrToolsEffectiveMcpServers $testRoot $testRoot $readerDesktop ([DateTime]::UtcNow.AddSeconds(5)) }
    catch { $diagnostic=$_.Exception.Message }
    Assert ($diagnostic -notlike '*fixture-secret*') 'Reader failure disclosed raw configuration diagnostics.'
    [IO.File]::WriteAllText($readerMode,'timeout')
    $elapsed=[Diagnostics.Stopwatch]::StartNew()
    Assert (-not (Test-FixtureNativeReady -Milliseconds 1000)) 'A timed-out reader was reported ready.'
    $elapsed.Stop()
    Assert ($elapsed.Elapsed.TotalSeconds -lt 5) 'Configuration observation exceeded its bounded wait.'
    $readerPid=[int][IO.File]::ReadAllText((Join-Path $testRoot 'reader-pid.txt'))
    $stillRunning=$null
    try { $stillRunning=[Diagnostics.Process]::GetProcessById($readerPid) } catch [ArgumentException] {}
    try { Assert ($null -eq $stillRunning -or $stillRunning.HasExited) 'Timed-out configuration reader was left running.' }
    finally { if ($null -ne $stillRunning) { $stillRunning.Dispose() } }
} finally {
    if ($null -ne $pipe) { $pipe.Dispose() }
}
Write-Output 'full_recovery_tests_passed'
