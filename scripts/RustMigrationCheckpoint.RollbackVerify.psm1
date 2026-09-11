Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.Common.psm1') -ErrorAction Stop
Set-StrictMode -Version Latest

function Invoke-CdrRollbackProbe {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)][string]$Label,
        [hashtable]$Environment = @{},
        [int]$TimeoutSeconds = 60
    )
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $FilePath
    $startInfo.Arguments = @($Arguments | ForEach-Object {
            if ([string]$_ -match '^-[A-Za-z0-9]+$') { [string]$_ }
            else { ConvertTo-CdrNativeArgument ([string]$_) }
        }) -join ' '
    $startInfo.WorkingDirectory = $WorkingDirectory
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $sensitiveKeys = @($startInfo.EnvironmentVariables.Keys | Where-Object {
            [string]$_ -match '(?i)(token|secret|password|cookie|api[_-]?key)'
        })
    foreach ($key in $sensitiveKeys) { $startInfo.EnvironmentVariables.Remove([string]$key) }
    foreach ($key in $Environment.Keys) {
        $startInfo.EnvironmentVariables[[string]$key] = [string]$Environment[$key]
    }

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) { throw "$Label did not start." }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $process.Kill()
            $null = $process.WaitForExit(5000)
            throw "$Label exceeded its $TimeoutSeconds-second deadline."
        }
        $process.WaitForExit()
        $stdout = [string]$stdoutTask.Result
        $stderr = [string]$stderrTask.Result
        if ($process.ExitCode -ne 0) {
            throw "$Label failed with exit $($process.ExitCode): $($stderr.Trim())"
        }
        return [pscustomobject]@{ Stdout = $stdout.Trim(); Stderr = $stderr.Trim() }
    } finally { $process.Dispose() }
}

function Test-CdrExtractedPowerShellSources {
    param(
        [Parameter(Mandatory = $true)][string]$PowerShellPath,
        [Parameter(Mandatory = $true)][string]$Operations
    )
    $probeCode = @'
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath($env:CDR_ROLLBACK_OPERATIONS_ROOT)
$files = @(Get-ChildItem -LiteralPath $root -Recurse -File | Where-Object {
    $_.Extension.ToLowerInvariant() -in @('.ps1', '.psm1')
})
foreach ($file in $files) {
    $tokens = $null
    $parseErrors = $null
    [void][Management.Automation.Language.Parser]::ParseFile(
        $file.FullName, [ref]$tokens, [ref]$parseErrors
    )
    if (@($parseErrors).Count -gt 0) {
        throw "PowerShell parse failed: $($file.FullName): $($parseErrors[0].Message)"
    }
}
foreach ($name in @(
    'codex-discord-memory-ab-common.ps1',
    'codex-discord-memory-ab-process.ps1',
    'codex-discord-memory-ab-report.ps1',
    'codex-discord-memory-ab-sample.ps1',
    'codex-discord-memory-ab-validation.ps1',
    'codex-discord-memory-ab-compare.ps1'
)) { . (Join-Path $root "scripts\$name") }
foreach ($name in @(
    'Resolve-MemoryAbFullPath', 'Get-MemoryAbVerifiedProcess',
    'Write-MemoryAbPhaseSummary', 'Invoke-MemoryAbPhaseSampling',
    'Get-MemoryAbNumericValue', 'Invoke-MemoryAbComparison'
)) {
    if ($null -eq (Get-Command $name -CommandType Function -ErrorAction SilentlyContinue)) {
        throw "Memory A/B rollback function was not loaded: $name"
    }
}
[Console]::Out.Write("passed|$($files.Count)")
'@
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($probeCode))
    $probeParameters = @{
        FilePath = $PowerShellPath
        Arguments = @('-NoProfile', '-NonInteractive', '-EncodedCommand', $encoded)
        WorkingDirectory = $Operations
        Label = 'Extracted PowerShell source preflight'
        Environment = @{ CDR_ROLLBACK_OPERATIONS_ROOT = $Operations }
    }
    $probe = Invoke-CdrRollbackProbe @probeParameters
    $parts = @($probe.Stdout -split '\|')
    if ($parts.Count -ne 2 -or $parts[0] -cne 'passed' -or
        $parts[1] -notmatch '^[1-9][0-9]*$') {
        throw "Extracted PowerShell source preflight returned unexpected output: $($probe.Stdout)"
    }
    return [int]$parts[1]
}

function Test-CdrCheckpointRollbackPayload {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$ExtractRoot,
        [Parameter(Mandatory = $true)][int]$SourceFileCount
    )
    $operations = Join-Path $ExtractRoot 'operations'
    foreach ($required in @(
            'codex-discord-watchdog.ps1',
            'codex-discord-atomic-file-runtime.ps1',
            'codex-discord-bot-headless.vbs',
            'codex-discord-memory-ab.ps1',
            'scripts\codex-discord-memory-ab-common.ps1',
            'scripts\codex-discord-memory-ab-process.ps1',
            'scripts\codex-discord-memory-ab-report.ps1',
            'scripts\codex-discord-memory-ab-sample.ps1',
            'scripts\codex-discord-memory-ab-validation.ps1',
            'scripts\codex-discord-memory-ab-compare.ps1'
        )) {
        Assert-CdrCheckpointLeaf (Join-Path $operations $required) 'Required source rollback payload'
    }

    $utf8 = [Text.UTF8Encoding]::new($false, $true)
    [IO.File]::WriteAllText(
        (Join-Path $operations '.codex_discord_bot.disabled'), "checkpoint_verify`n", $utf8
    )
    $powerShell = Get-Command 'powershell.exe' -ErrorAction Stop
    $watchdog = Invoke-CdrRollbackProbe `
        -FilePath $powerShell.Source `
        -Arguments @(
            '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File',
            (Join-Path $operations 'codex-discord-watchdog.ps1'), '-DryRun'
        ) `
        -WorkingDirectory $operations `
        -Label 'Extracted Rust watchdog dry run' `
        -Environment @{ CODEX_DISCORD_RUNTIME = 'rust' }
    if ($watchdog.Stdout -cne 'disabled') {
        throw "Extracted Rust watchdog dry run returned unexpected output: $($watchdog.Stdout)"
    }
    $powerShellProbe = @{
        PowerShellPath = $powerShell.Source
        Operations = $operations
    }
    $powerShellFileCount = Test-CdrExtractedPowerShellSources @powerShellProbe

    $probeRoot = Join-Path (Split-Path -Parent $ExtractRoot) 'native-rollback-probe'
    $null = New-Item -ItemType Directory -Path $probeRoot
    $runtime = Join-Path $ExtractRoot 'artifacts\cdr-runtime.exe'
    $setup = Invoke-CdrRollbackProbe -FilePath $runtime -Arguments @(
        '--admin', 'setup-discord', '--repo-root', $probeRoot, '--dry-run', '--bot-id', '42'
    ) -WorkingDirectory $probeRoot -Label 'Extracted Rust setup preflight'
    if ($setup.Stdout -notmatch 'client_id=42' -or
        (Test-Path -LiteralPath (Join-Path $probeRoot '.env'))) {
        throw 'Extracted Rust setup preflight was invalid or unexpectedly wrote configuration.'
    }
    $pro = Invoke-CdrRollbackProbe `
        -FilePath (Join-Path $ExtractRoot 'artifacts\cdr-pro-helper.exe') `
        -Arguments @('conversation', 'status', '--scope', 'codex-pro-000000000000000000000000') `
        -WorkingDirectory $probeRoot -Label 'Extracted Rust Pro helper preflight' `
        -Environment @{ SIMDOREI_PRO_CONVERSATION_DB = (Join-Path $probeRoot 'pro.sqlite') }
    $proState = $pro.Stdout | ConvertFrom-Json
    if ($proState.status -cne 'missing') {
        throw 'Extracted Rust Pro helper returned an unexpected conversation state.'
    }
    if (@(Get-ChildItem -LiteralPath $operations -Recurse -File -Filter '*.py').Count -ne 0) {
        throw 'Python sources are not part of a Rust-only rollback payload.'
    }

    return [pscustomobject][ordered]@{
        Status = 'passed'
        PowerShellWatchdogDryRun = 'passed_disabled'
        PowerShellSourceParse = 'passed'
        PowerShellSourceFileCount = $powerShellFileCount
        MemoryAbModuleImportPreflight = 'passed'
        RustSetupPreflight = 'passed'
        RustProHelperPreflight = 'passed'
        SourceFileCount = $SourceFileCount
        RequiresPython = $false
    }
}

Export-ModuleMember -Function 'Test-CdrCheckpointRollbackPayload'
