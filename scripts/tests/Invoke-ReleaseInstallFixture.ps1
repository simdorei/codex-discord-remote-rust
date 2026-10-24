param([string]$Root, [string]$CodexExe, [string]$CodexHome, [switch]$DryRun)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$OutputEncoding = [Console]::OutputEncoding
$env:PATH = "$env:SystemRoot\System32;$env:SystemRoot;$env:SystemRoot\System32\WindowsPowerShell\v1.0"
$env:CARGO_TARGET_DIR = $null
foreach ($command in @('git', 'cargo', 'rustc', 'cl', 'python', 'node')) {
    if (Get-Command $command -ErrorAction SilentlyContinue) { throw "Test environment exposes a developer tool: $command" }
}
# Any attempt to turn an install into a startup fails this executable contract.
function global:Register-ScheduledTask { throw 'Installation tried to register automatic startup' }
function global:Start-ScheduledTask { throw 'Installation tried to start a scheduled task' }
function global:Enable-ScheduledTask { throw 'Installation tried to enable automatic startup' }
function global:Start-Process { throw 'Installation tried to launch a background process' }
& (Join-Path $Root 'install.ps1') -Prebuilt -CodexExe $CodexExe -CodexHome $CodexHome -DryRun:$DryRun
