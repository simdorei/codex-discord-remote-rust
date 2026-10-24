[CmdletBinding()]
param([Parameter(Mandatory)][string]$Archive, [string]$CodexExe)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'CdrNativeProcess.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'CdrWindowsBundle.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'CdrWindowsExecutable.psm1') -Force
Add-Type -AssemblyName System.IO.Compression.FileSystem
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
$work = Join-Path $tempRoot ('cdr-release-' + [guid]::NewGuid().ToString('N'))
$extract = Join-Path $work ('install space ' + [char]0xd55c + [char]0xae00)
$null = [IO.Directory]::CreateDirectory($extract)
$utf8 = [Text.UTF8Encoding]::new($false)
$previousRoot = $env:CDR_RELEASE_TEST_ROOT
$checks = 0
function Assert-Release([bool]$Condition, [string]$Reason) {
    if (-not $Condition) { throw $Reason }
    $script:checks++
}
function Assert-Refused([scriptblock]$Action, [string]$Expected) {
    try { & $Action | Out-Null } catch {
        if ($_.Exception.Message -notmatch [regex]::Escape($Expected)) { throw }
        $script:checks++
        return
    }
    throw "Expected failure was not reported: $Expected"
}
try {
    $Archive = [IO.Path]::GetFullPath($Archive)
    $expectedHash = ([IO.File]::ReadAllText($Archive + '.sha256').Trim() -split '\s+')[0]
    Assert-Release ((Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash -ieq $expectedHash) 'Archive checksum mismatch'
    [IO.Compression.ZipFile]::ExtractToDirectory($Archive, $extract)
    $roots = @(Get-ChildItem -LiteralPath $extract -Directory -Force)
    Assert-Release ($roots.Count -eq 1) 'Expected exactly one directory in the ZIP'
    $root = $roots[0].FullName
    $manifest = Test-CdrWindowsBundle -Root $root
    $checks++
    foreach ($file in $manifest.files) {
        Assert-Release ($file.path -notmatch '(^|/)(\.env|\.git|auth\.json|target)(/|$)|\.(sqlite|sqlite3|db|log)$') 'Release contains local state or build output'
    }
    foreach ($name in @('cdr-runtime.exe', 'cdr-pro-helper.exe')) {
        Test-CdrWindowsExecutable -Path (Join-Path $root ('bin\' + $name))
        $checks++
    }
    $profile = Join-Path $work 'codex-profile'
    $null = [IO.Directory]::CreateDirectory($profile)
    [IO.File]::WriteAllText((Join-Path $profile 'config.toml'), "[notice]`nhide_world_writable_warning = true`nhide_full_access_warning = true`n", $utf8)
    if (-not $CodexExe) {
        $CodexExe = Join-Path $work 'codex.cmd'
        $fakeScript = Join-Path $work 'fake-codex.ps1'
        [IO.File]::Copy((Join-Path $PSScriptRoot 'tests\Fake-ReleaseCodex.ps1'), $fakeScript)
        [IO.File]::WriteAllText($CodexExe, "@echo off`r`npowershell.exe -NoProfile -ExecutionPolicy Bypass -File `"%~dp0fake-codex.ps1`" %*`r`nexit /b %errorlevel%`r`n", $utf8)
    }
    $env:CDR_RELEASE_TEST_ROOT = $root
    $hostScript = Join-Path $PSScriptRoot 'tests\Invoke-ReleaseInstallFixture.ps1'
    $arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $hostScript,
        '-Root', $root, '-CodexExe', $CodexExe, '-CodexHome', $profile)
    $shell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $dry = [string](Invoke-CdrNative -Executable $shell -Arguments ($arguments + '-DryRun'))
    Assert-Release ($dry.Contains('Dry run complete.')) 'Dry-run did not finish'
    Assert-Release (-not (Test-Path -LiteralPath (Join-Path $root '.env'))) 'Dry-run created settings'
    Assert-Release (-not (Test-Path -LiteralPath (Join-Path $root 'target'))) 'Dry-run installed files'
    $entry = [string](Invoke-CdrNative -Executable (Join-Path $root 'install.cmd') `
        -Arguments @('-DryRun', '-CodexExe', $CodexExe, '-CodexHome', $profile) -InputText "`r`n" `
        -EnvironmentOverrides @{PATH="$env:SystemRoot\System32;$env:SystemRoot;$env:SystemRoot\System32\WindowsPowerShell\v1.0";CARGO_TARGET_DIR=''})
    Assert-Release ($entry.Contains('Dry run complete.')) 'Double-click entry failed its dry-run'
    $envPath = Join-Path $root '.env'
    [IO.File]::WriteAllText($envPath, "KEEP=preserve-me`nDISCORD_BOT_TOKEN=test-not-a-secret`n", $utf8)
    $statePath = Join-Path $root 'existing-state.bin'
    [IO.File]::WriteAllText($statePath, 'existing user data', $utf8)
    $installed = [string](Invoke-CdrNative -Executable $shell -Arguments $arguments)
    Assert-Release ($installed.Contains('Install complete.')) 'Installation did not finish'
    Assert-Release ($installed.Contains('Verified Codex plugin inventory')) 'Plugin installation was not verified'
    Assert-Release ([IO.File]::ReadAllText($envPath).Contains('KEEP=preserve-me')) 'Existing setting changed'
    Assert-Release ([IO.File]::ReadAllText($envPath).Contains('DISCORD_BOT_TOKEN=test-not-a-secret')) 'Existing bot token changed'
    Assert-Release ([IO.File]::ReadAllText($statePath) -ceq 'existing user data') 'Existing data changed'
    $installedRuntime = Join-Path $root 'target\release\cdr-runtime.exe'
    Assert-Release ((Get-FileHash -LiteralPath $installedRuntime).Hash -ceq (Get-FileHash -LiteralPath (Join-Path $root 'bin\cdr-runtime.exe')).Hash) 'Wrong runtime installed'
    $help = [string](Invoke-CdrNative -Executable $installedRuntime -Arguments @('--help'))
    Assert-Release ($help.Length -gt 20) 'Installed runtime cannot print help'
    foreach ($name in @('.codex_discord_rust.runtime.lock', '.codex_discord_rust.heartbeat', '.codex_discord_bot.disabled')) {
        Assert-Release (-not (Test-Path -LiteralPath (Join-Path $root $name))) 'Installation started or changed bot operation'
    }
    # Reinstalling the same release must preserve state and remain an install-only action.
    $repeat = [string](Invoke-CdrNative -Executable $shell -Arguments $arguments)
    Assert-Release ($repeat.Contains('Install complete.')) 'Same-version reinstall failed'
    $setupInput = @{token=('synthetic-' + [char]0xd1a0 + [char]0xd070);channel_id='invalid-channel'} | ConvertTo-Json -Compress
    Assert-Refused { Invoke-CdrNative -Executable $installedRuntime -Arguments @('--admin', 'setup-discord',
        '--input-stdin', '--repo-root', $root) -InputText $setupInput } 'Discord ID must contain digits only'
    $helper = Join-Path $root 'bin\cdr-pro-helper.exe'
    $helperBytes = [IO.File]::ReadAllBytes($helper)
    [IO.File]::Delete($helper)
    Assert-Refused { Invoke-CdrNative -Executable $shell -Arguments $arguments } 'RELEASE_INCOMPLETE: missing file: bin/cdr-pro-helper.exe'
    [IO.File]::WriteAllBytes($helper, $helperBytes)
    [IO.File]::AppendAllText($helper, 'damaged', $utf8)
    Assert-Refused { Invoke-CdrNative -Executable $shell -Arguments $arguments } 'RELEASE_DAMAGED: checksum mismatch: bin/cdr-pro-helper.exe'
    [IO.File]::WriteAllBytes($helper, $helperBytes)
    $manifestPath = Join-Path $root 'release-manifest.json'
    $manifestText = [IO.File]::ReadAllText($manifestPath)
    $manifest.files = @($manifest.files | Where-Object path -ne 'bin/cdr-pro-helper.exe')
    [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 5), $utf8)
    Assert-Refused { Test-CdrWindowsBundle -Root $root } 'manifest omits required file: bin/cdr-pro-helper.exe'
    [IO.File]::WriteAllText($manifestPath, $manifestText, $utf8)
    [IO.File]::Delete($manifestPath)
    Assert-Refused { Invoke-CdrNative -Executable $shell -Arguments $arguments } 'RELEASE_INCOMPLETE: release-manifest.json is missing'
    [IO.File]::WriteAllText($manifestPath, $manifestText, $utf8)
    $environmentHash = (Get-FileHash -LiteralPath $envPath).Hash
    [IO.File]::WriteAllText($installedRuntime, 'old runtime must be preserved', $utf8)
    Assert-Refused { Invoke-CdrNative -Executable $shell -Arguments $arguments } 'Existing runtime differs'
    Assert-Release ([IO.File]::ReadAllText($installedRuntime) -ceq 'old runtime must be preserved') 'Existing runtime was overwritten'
    Assert-Release ((Get-FileHash -LiteralPath $envPath).Hash -ceq $environmentHash) 'Refused installation changed settings'
    Write-Output "Windows release checks passed: $checks (no developer tools; no bot startup)."
} finally {
    $env:CDR_RELEASE_TEST_ROOT = $previousRoot
    $resolvedWork = [IO.Path]::GetFullPath($work)
    if (-not $resolvedWork.StartsWith($tempRoot + '\', [StringComparison]::OrdinalIgnoreCase) -or
        ((Get-Item -LiteralPath $resolvedWork).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Refusing to clean a test directory outside the temporary root.'
    }
    Remove-Item -LiteralPath $resolvedWork -Recurse -Force
}
