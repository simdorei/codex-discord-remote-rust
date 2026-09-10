[CmdletBinding()]
param(
    [string]$PythonExe = $env:PYTHON_EXE,
    [string]$CodexExe = $env:CODEX_EXE,
    [string]$CodexHome = $env:CODEX_HOME,
    [ValidateSet('rust', 'python')]
    [string]$Runtime = 'rust',
    [switch]$SkipDependencies,
    [switch]$SkipEnvFile,
    [switch]$SkipSteeringConfig,
    [switch]$SkipCodexPlugin,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
$CodexExeWasExplicit = $PSBoundParameters.ContainsKey('CodexExe')

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RequirementsPath = Join-Path $ScriptDir 'requirements.txt'
$RuntimeReleasePath = Join-Path $ScriptDir 'runtime-release.json'
$EnvExamplePath = Join-Path $ScriptDir '.env.example'
$EnvPath = Join-Path $ScriptDir '.env'
$PluginMarketplacePath = Join-Path $ScriptDir '.agents\plugins\marketplace.json'
$PluginManifestPath = Join-Path $ScriptDir 'plugins\codex-discord-remote\.codex-plugin\plugin.json'
$PluginInventoryVerifierPath = Join-Path $ScriptDir 'verify_codex_plugin_inventory.py'
$PluginMarketplaceName = 'codex-discord-remote'
$PluginRef = 'codex-discord-remote@codex-discord-remote'
$RuntimeModePath = Join-Path $ScriptDir '.codex_discord_runtime'
$RustRuntimePath = Join-Path $ScriptDir 'target\release\cdr-runtime.exe'
$RequiredPythonMajor = 3
$RequiredPythonMinor = 12
$PortablePythonVersion = '3.12.1'
$PortablePipVersion = '26.2.1'
$PortablePythonDir = Join-Path $ScriptDir '.python-portable'
$PortablePythonStageDir = Join-Path $ScriptDir '.python-portable.stage'
$PortablePythonPreviousDir = Join-Path $ScriptDir '.python-portable.previous'
$PortablePythonExe = Join-Path $PortablePythonDir 'python.exe'

if (-not (Test-Path -LiteralPath $RuntimeReleasePath)) {
    throw "runtime-release.json was not found: $RuntimeReleasePath"
}
$RuntimeRelease = Get-Content -Raw -LiteralPath $RuntimeReleasePath | ConvertFrom-Json
if ([string]$RuntimeRelease.python.version -ne $PortablePythonVersion) {
    throw "runtime-release.json Python version does not match installer: $PortablePythonVersion"
}
if ([string]$RuntimeRelease.pip.version -ne $PortablePipVersion) {
    throw "runtime-release.json pip version does not match installer: $PortablePipVersion"
}
$PortablePythonUrl = [string]$RuntimeRelease.python.url
$PortablePythonSha256 = [string]$RuntimeRelease.python.sha256
$GetPipUrl = [string]$RuntimeRelease.get_pip.url
$GetPipSha256 = [string]$RuntimeRelease.get_pip.sha256

function Test-PythonCommand {
    param([string[]]$Command)

    $exe = $Command[0]
    $baseArgs = @()
    if ($Command.Count -gt 1) {
        $baseArgs = $Command[1..($Command.Count - 1)]
    }

    & $exe @baseArgs -c "import sys; raise SystemExit(0 if sys.version_info[:2] == ($RequiredPythonMajor, $RequiredPythonMinor) else 1)" >$null 2>$null
    return $LASTEXITCODE -eq 0
}

function Get-PythonExecutablePath {
    param([string[]]$Command)

    $exe = $Command[0]
    $baseArgs = @()
    if ($Command.Count -gt 1) {
        $baseArgs = $Command[1..($Command.Count - 1)]
    }

    $output = & $exe @baseArgs -c "import sys; print(sys.executable)"
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($output)) {
        throw "Python command failed while resolving sys.executable: $exe $($baseArgs -join ' ')"
    }
    return ([string]$output).Trim()
}

function Find-PythonCommand {
    if (-not [string]::IsNullOrWhiteSpace($PythonExe)) {
        $candidate = @($PythonExe)
        if (Test-PythonCommand -Command $candidate) {
            return $candidate
        }
        throw "PYTHON_EXE must point to Python 3.12.x: $PythonExe"
    }

    if (Test-Path -LiteralPath $PortablePythonExe) {
        $candidate = @($PortablePythonExe)
        if (Test-PythonCommand -Command $candidate) {
            return $candidate
        }
    }

    return @()
}

function Enable-PortablePythonSite {
    param([string]$PythonDir)

    $pthFiles = @(Get-ChildItem -LiteralPath $PythonDir -Filter 'python*._pth' -File)
    if ($pthFiles.Count -eq 0) {
        throw "Portable Python _pth file was not found in $PythonDir."
    }
    $pthPath = $pthFiles[0].FullName
    $lines = @(Get-Content -LiteralPath $pthPath)
    $updatedLines = [System.Collections.Generic.List[string]]::new()
    $hasRepoRoot = $false
    $hasImportSite = $false
    foreach ($lineValue in $lines) {
        $line = [string]$lineValue
        $trimmed = $line.Trim()
        if ($trimmed -eq '..') {
            $hasRepoRoot = $true
        }
        if ($trimmed -eq 'import site') {
            if (-not $hasRepoRoot) {
                $updatedLines.Add('..')
                $hasRepoRoot = $true
            }
            $hasImportSite = $true
            $updatedLines.Add($line)
            continue
        }
        if ($trimmed -eq '#import site') {
            if (-not $hasRepoRoot) {
                $updatedLines.Add('..')
                $hasRepoRoot = $true
            }
            $hasImportSite = $true
            $updatedLines.Add('import site')
            continue
        }
        $updatedLines.Add($line)
    }
    if (-not $hasRepoRoot) {
        $updatedLines.Add('..')
    }
    if (-not $hasImportSite) {
        $updatedLines.Add('import site')
    }
    Set-Content -LiteralPath $pthPath -Value $updatedLines -Encoding ASCII
}

function Assert-FileSha256 {
    param(
        [string]$Path,
        [string]$ExpectedSha256
    )

    if ($ExpectedSha256 -notmatch '^[0-9a-fA-F]{64}$') {
        throw "Invalid expected SHA-256 for ${Path}: $ExpectedSha256"
    }
    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $ExpectedSha256.ToLowerInvariant()) {
        throw "SHA-256 verification failed for ${Path}. Expected $ExpectedSha256, got $actual."
    }
}

function Install-PortablePython312 {
    if ($DryRun) {
        Write-Host "Would verify and stage portable Python $PortablePythonVersion in $PortablePythonStageDir"
        return
    }
    Write-Host "Portable Python $PortablePythonVersion was not found. Building a verified staged runtime."
    if (Test-Path -LiteralPath $PortablePythonStageDir) {
        Remove-Item -LiteralPath $PortablePythonStageDir -Recurse -Force
    }
    New-Item -ItemType Directory -Path $PortablePythonStageDir | Out-Null
    $stagePythonExe = Join-Path $PortablePythonStageDir 'python.exe'
    $stagePythonZip = Join-Path $PortablePythonStageDir "python-${PortablePythonVersion}-embed-amd64.zip"
    $stageGetPipPath = Join-Path $PortablePythonStageDir 'get-pip.py'
    $stageGetPipLogPath = Join-Path $PortablePythonStageDir 'get-pip.log'
    try {
        Invoke-WebRequest -Uri $PortablePythonUrl -OutFile $stagePythonZip
        Assert-FileSha256 -Path $stagePythonZip -ExpectedSha256 $PortablePythonSha256
        Expand-Archive -LiteralPath $stagePythonZip -DestinationPath $PortablePythonStageDir -Force
        Remove-Item -LiteralPath $stagePythonZip -Force
        Enable-PortablePythonSite -PythonDir $PortablePythonStageDir

        Invoke-WebRequest -Uri $GetPipUrl -OutFile $stageGetPipPath
        Assert-FileSha256 -Path $stageGetPipPath -ExpectedSha256 $GetPipSha256
        & $stagePythonExe $stageGetPipPath "pip==$PortablePipVersion" --no-warn-script-location *> $stageGetPipLogPath
        if ($LASTEXITCODE -ne 0) {
            throw "get-pip.py failed for staged portable Python with exit code ${LASTEXITCODE}. See $stageGetPipLogPath."
        }
        Remove-Item -LiteralPath $stageGetPipPath -Force
        Remove-Item -LiteralPath $stageGetPipLogPath -Force
        if (-not (Test-PythonCommand -Command @($stagePythonExe))) {
            throw "Staged portable Python failed the Python 3.12 runtime check: $stagePythonExe"
        }

        if (Test-Path -LiteralPath $PortablePythonPreviousDir) {
            Remove-Item -LiteralPath $PortablePythonPreviousDir -Recurse -Force
        }
        if (Test-Path -LiteralPath $PortablePythonDir) {
            Move-Item -LiteralPath $PortablePythonDir -Destination $PortablePythonPreviousDir
        }
        try {
            Move-Item -LiteralPath $PortablePythonStageDir -Destination $PortablePythonDir
        } catch {
            if (Test-Path -LiteralPath $PortablePythonPreviousDir) {
                Move-Item -LiteralPath $PortablePythonPreviousDir -Destination $PortablePythonDir
            }
            throw
        }
        if (Test-Path -LiteralPath $PortablePythonPreviousDir) {
            Remove-Item -LiteralPath $PortablePythonPreviousDir -Recurse -Force
        }
    } catch {
        if (Test-Path -LiteralPath $PortablePythonStageDir) {
            Remove-Item -LiteralPath $PortablePythonStageDir -Recurse -Force
        }
        throw
    }
}

function Resolve-PythonCommand {
    $command = @(Find-PythonCommand)
    if ($command.Count -gt 0) {
        return $command
    }

    Install-PortablePython312
    if ($DryRun) {
        return @($PortablePythonExe)
    }
    $command = @(Find-PythonCommand)
    if ($command.Count -gt 0) {
        return $command
    }

    throw "Portable Python 3.12.x was not found after bootstrap: $PortablePythonExe"
}

function Invoke-Python {
    param([string[]]$Arguments)

    $command = @(Resolve-PythonCommand)
    $exe = $command[0]
    $baseArgs = @()
    if ($command.Count -gt 1) {
        $baseArgs = $command[1..($command.Count - 1)]
    }

    if ($DryRun) {
        Write-Output "Would run: $exe $($baseArgs + $Arguments -join ' ')"
        return
    }

    & $exe @baseArgs @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Python command failed with exit code ${LASTEXITCODE}: $exe $($baseArgs + $Arguments -join ' ')"
    }
}

function Get-EnvFileValue {
    param([string]$Name)

    if (-not (Test-Path -LiteralPath $EnvPath)) {
        return ''
    }
    foreach ($line in Get-Content -LiteralPath $EnvPath) {
        $trimmed = ([string]$line).Trim()
        if (-not $trimmed -or $trimmed.StartsWith('#')) {
            continue
        }
        $key, $value = $trimmed.Split('=', 2)
        if ($key -eq $Name) {
            return $value.Trim().Trim('"')
        }
    }
    return ''
}

function Set-EnvFileValue {
    param(
        [string]$Name,
        [string]$Value
    )

    $lines = @()
    if (Test-Path -LiteralPath $EnvPath) {
        $lines = @(Get-Content -LiteralPath $EnvPath)
    }

    $found = $false
    for ($i = 0; $i -lt $lines.Count; $i++) {
        $line = [string]$lines[$i]
        $trimmed = $line.Trim()
        if (-not $trimmed -or $trimmed.StartsWith('#') -or -not $line.Contains('=')) {
            continue
        }
        $key, $oldValue = $line.Split('=', 2)
        $null = $oldValue
        if ($key.Trim() -eq $Name) {
            $lines[$i] = "$Name=$Value"
            $found = $true
            break
        }
    }

    if (-not $found) {
        $lines += "$Name=$Value"
    }

    Set-Content -LiteralPath $EnvPath -Value $lines -Encoding UTF8
}

function Get-DefaultCodexHomePath {
    return (Join-Path ([Environment]::GetFolderPath('UserProfile')) '.codex')
}

function Test-CodexRuntimeBinPath {
    param([string]$Path)

    $normalized = $Path.TrimEnd('\', '/').ToLowerInvariant()
    return $normalized.Contains('\.sandbox-bin') `
        -or $normalized.Contains('\plugins\.plugin-appserver') `
        -or $normalized.EndsWith('\appdata\local\openai\codex\bin') `
        -or $normalized.EndsWith('\app\resources')
}

function Resolve-CodexHomePath {
    $defaultCodexHome = Get-DefaultCodexHomePath
    if (-not [string]::IsNullOrWhiteSpace($CodexHome)) {
        $expanded = [Environment]::ExpandEnvironmentVariables($CodexHome.Trim().Trim('"').Trim("'"))
        $homePath = [Environment]::GetFolderPath('UserProfile')
        if ($expanded -eq '~') {
            return $homePath
        }
        if ($expanded.StartsWith('~/') -or $expanded.StartsWith('~\')) {
            $expanded = Join-Path $homePath $expanded.Substring(2)
        }
        $resolved = [System.IO.Path]::GetFullPath($expanded)
        if (Test-CodexRuntimeBinPath -Path $resolved) {
            return $defaultCodexHome
        }
        return $resolved
    }

    return $defaultCodexHome
}

function Find-CodexCommand {
    if (-not [string]::IsNullOrWhiteSpace($CodexExe)) {
        return $CodexExe.Trim().Trim('"').Trim("'")
    }

    $envCodexExe = Get-EnvFileValue -Name 'CODEX_EXE'
    if (-not [string]::IsNullOrWhiteSpace($envCodexExe)) {
        return $envCodexExe
    }

    $codex = Get-Command codex -ErrorAction SilentlyContinue
    if ($codex -ne $null) {
        return $codex.Source
    }

    return ''
}

function Resolve-CodexCommand {
    $command = Find-CodexCommand
    if (-not [string]::IsNullOrWhiteSpace($command)) {
        return $command
    }

    throw 'Codex CLI was not found. Set CODEX_EXE or install/enable the codex command.'
}

function ConvertTo-WindowsNativeArgument {
    param(
        [AllowEmptyString()]
        [string]$Argument
    )

    if ($Argument.Length -gt 0 -and $Argument -notmatch '[\s"]') {
        return $Argument
    }

    $builder = [System.Text.StringBuilder]::new()
    $null = $builder.Append('"')
    $backslashCount = 0
    foreach ($character in $Argument.ToCharArray()) {
        if ($character -eq [char]92) {
            $backslashCount++
            continue
        }
        if ($character -eq [char]34) {
            if ($backslashCount -gt 0) {
                $null = $builder.Append(
                    [string]::new([char]92, ($backslashCount * 2) + 1)
                )
            } else {
                $null = $builder.Append([char]92)
            }
            $null = $builder.Append([char]34)
            $backslashCount = 0
            continue
        }
        if ($backslashCount -gt 0) {
            $null = $builder.Append([string]::new([char]92, $backslashCount))
            $backslashCount = 0
        }
        $null = $builder.Append($character)
    }
    if ($backslashCount -gt 0) {
        $null = $builder.Append(
            [string]::new([char]92, $backslashCount * 2)
        )
    }
    $null = $builder.Append('"')
    return $builder.ToString()
}

function Invoke-Codex {
    param([string[]]$Arguments)

    $exe = Resolve-CodexCommand
    if ($DryRun) {
        Write-Output "Would run: $exe $($Arguments -join ' ')"
        return
    }

    $resolvedCommand = Get-Command -Name $exe -ErrorAction SilentlyContinue
    if ($null -ne $resolvedCommand -and
            -not [string]::IsNullOrWhiteSpace($resolvedCommand.Source)) {
        $exe = $resolvedCommand.Source
    }

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $utf8 = [System.Text.UTF8Encoding]::new($false)
    $startInfo.StandardOutputEncoding = $utf8
    $startInfo.StandardErrorEncoding = $utf8

    $extension = [System.IO.Path]::GetExtension($exe).ToLowerInvariant()
    if ($extension -eq '.ps1') {
        $cmdCandidate = [System.IO.Path]::ChangeExtension($exe, '.cmd')
        if (-not (Test-Path -LiteralPath $cmdCandidate -PathType Leaf)) {
            throw "Codex resolved to a PowerShell shim without a sibling .cmd launcher: $exe. Pass -CodexExe with the Codex .exe or .cmd launcher."
        }
        $exe = $cmdCandidate
        $extension = '.cmd'
    }
    if ($extension -eq '.cmd' -or $extension -eq '.bat') {
        $commandShell = $env:ComSpec
        if ([string]::IsNullOrWhiteSpace($commandShell)) {
            $commandShell = Join-Path $env:SystemRoot 'System32\cmd.exe'
        }
        $variablePrefix = 'CODEX_INSTALL_' + [guid]::NewGuid().ToString('N')
        $exeVariable = $variablePrefix + '_EXE'
        $startInfo.EnvironmentVariables[$exeVariable] = $exe
        $commandParts = [System.Collections.Generic.List[string]]::new()
        $commandParts.Add('"%' + $exeVariable + '%"')
        for ($index = 0; $index -lt $Arguments.Count; $index++) {
            $argument = [string]$Arguments[$index]
            if ($argument.Contains('"')) {
                throw 'Codex .cmd arguments cannot contain a double quote.'
            }
            $argumentVariable = $variablePrefix + '_ARG_' + $index
            $startInfo.EnvironmentVariables[$argumentVariable] = $argument
            $commandParts.Add('"%' + $argumentVariable + '%"')
        }
        $startInfo.FileName = $commandShell
        $startInfo.Arguments = '/d /s /v:off /c "' + ($commandParts -join ' ') + '"'
    } else {
        $startInfo.FileName = $exe
        $nativeArguments = @(
            $Arguments | ForEach-Object {
                ConvertTo-WindowsNativeArgument -Argument ([string]$_)
            }
        )
        $startInfo.Arguments = $nativeArguments -join ' '
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) {
            throw "Codex CLI process did not start: $exe"
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        $stdoutText = $stdoutTask.GetAwaiter().GetResult()
        $stderrText = $stderrTask.GetAwaiter().GetResult()
        $exitCode = $process.ExitCode

        if ($exitCode -ne 0) {
            $details = @()
            if (-not [string]::IsNullOrWhiteSpace($stdoutText)) {
                $details += "stdout:`n$stdoutText"
            }
            if (-not [string]::IsNullOrWhiteSpace($stderrText)) {
                $details += "stderr:`n$stderrText"
            }
            $detailText = if ($details.Count -gt 0) {
                "`n" + ($details -join "`n")
            }
            else {
                ''
            }
            throw "Codex CLI failed with exit code ${exitCode}: $exe $($Arguments -join ' ')${detailText}`nUpdate Codex or pass -CodexExe with the correct Codex CLI executable."
        }

        if (-not [string]::IsNullOrWhiteSpace($stderrText)) {
            Write-Warning $stderrText
        }
        if ($stdoutText.Length -gt 0) {
            Write-Output $stdoutText
        }
    }
    finally {
        $process.Dispose()
    }
}

function Test-CodexPluginInventory {
    if ($DryRun) {
        Write-Output "Would verify Codex marketplace and plugin inventory for $PluginRef"
        return
    }

    $marketplaceInventoryPath = [System.IO.Path]::GetTempFileName()
    $pluginInventoryPath = [System.IO.Path]::GetTempFileName()
    $utf8 = [System.Text.UTF8Encoding]::new($false)
    try {
        $marketplaceOutput = @(Invoke-Codex -Arguments @('plugin', 'marketplace', 'list', '--json'))
        $pluginOutput = @(Invoke-Codex -Arguments @('plugin', 'list', '--json'))
        [System.IO.File]::WriteAllText(
            $marketplaceInventoryPath,
            ($marketplaceOutput -join [Environment]::NewLine),
            $utf8
        )
        [System.IO.File]::WriteAllText(
            $pluginInventoryPath,
            ($pluginOutput -join [Environment]::NewLine),
            $utf8
        )
        Invoke-Python -Arguments @(
            $PluginInventoryVerifierPath,
            '--marketplace-inventory', $marketplaceInventoryPath,
            '--plugin-inventory', $pluginInventoryPath,
            '--plugin-manifest', $PluginManifestPath,
            '--expected-root', $ScriptDir,
            '--marketplace-name', $PluginMarketplaceName,
            '--plugin-id', $PluginRef
        )
    } catch {
        throw "INSTALL_INCOMPLETE: Codex plugin inventory verification failed. $($_.Exception.Message)"
    } finally {
        Remove-Item -LiteralPath $marketplaceInventoryPath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $pluginInventoryPath -Force -ErrorAction SilentlyContinue
    }
}

function Install-RustRuntime {
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if ($null -eq $cargo) {
        throw 'Rust cargo was not found. Install the rust-toolchain.toml toolchain before selecting -Runtime rust.'
    }
    if ($DryRun) {
        Write-Output "Would build the pinned Rust runtime: $RustRuntimePath"
        return
    }
    Push-Location $ScriptDir
    try {
        & $cargo.Source build --release --locked -p cdr-runtime
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
    if (-not (Test-Path -LiteralPath $RustRuntimePath -PathType Leaf)) {
        throw "Rust build completed without the expected binary: $RustRuntimePath"
    }
    Write-Output "Built Rust runtime: $RustRuntimePath"
}

if ($Runtime -eq 'rust') {
    Install-RustRuntime
}

if (-not $SkipDependencies) {
    if (-not (Test-Path -LiteralPath $RequirementsPath)) {
        throw "requirements.txt was not found: $RequirementsPath"
    }
    Write-Output "Installing Python dependencies from requirements.txt"
    Invoke-Python -Arguments @('-m', 'pip', 'install', '--require-hashes', '-r', $RequirementsPath)
}

if (-not $SkipEnvFile) {
    if (Test-Path -LiteralPath $EnvPath) {
        Write-Output ".env already exists: $EnvPath"
    } elseif (Test-Path -LiteralPath $EnvExamplePath) {
        if ($DryRun) {
            Write-Output "Would create: $EnvPath from .env.example"
        } else {
            Copy-Item -LiteralPath $EnvExamplePath -Destination $EnvPath
            Write-Output "Created: $EnvPath"
        }
    } else {
        Write-Output ".env.example was not found; skipping .env creation."
    }

    if ($DryRun) {
        Write-Output "Would set PYTHON_EXE to the resolved Python 3.12 executable in .env"
        $codexHomePath = Resolve-CodexHomePath
        Write-Output "Would set CODEX_HOME=$codexHomePath in .env"
        $existingCodexCommand = Get-EnvFileValue -Name 'CODEX_EXE'
        if ($CodexExeWasExplicit) {
            Write-Output "Would set explicitly configured CODEX_EXE in .env"
        } elseif (-not [string]::IsNullOrWhiteSpace($existingCodexCommand)) {
            Write-Output "Would preserve existing CODEX_EXE in .env"
        } elseif (-not [string]::IsNullOrWhiteSpace($CodexExe)) {
            Write-Output "Inherited CODEX_EXE would not be saved to .env"
        } else {
            Write-Output "Would leave CODEX_EXE unchanged; PATH-discovered Codex commands are not saved to .env"
        }
    } else {
        $pythonCommand = @(Resolve-PythonCommand)
        $pythonExePath = Get-PythonExecutablePath -Command $pythonCommand
        Set-EnvFileValue -Name 'PYTHON_EXE' -Value $pythonExePath
        Write-Output "Configured PYTHON_EXE=$pythonExePath"

        $codexHomePath = Resolve-CodexHomePath
        Set-EnvFileValue -Name 'CODEX_HOME' -Value $codexHomePath
        Write-Output "Configured CODEX_HOME=$codexHomePath"

        if ($CodexExeWasExplicit) {
            $explicitCodexCommand = $CodexExe.Trim().Trim('"').Trim("'")
            Set-EnvFileValue -Name 'CODEX_EXE' -Value $explicitCodexCommand
            Write-Output "Configured explicit CODEX_EXE=$explicitCodexCommand"
        } else {
            $existingCodexCommand = Get-EnvFileValue -Name 'CODEX_EXE'
            if (-not [string]::IsNullOrWhiteSpace($existingCodexCommand)) {
                Write-Output 'Preserved existing CODEX_EXE in .env.'
            } elseif (-not [string]::IsNullOrWhiteSpace($CodexExe)) {
                Write-Output 'Inherited CODEX_EXE was not saved to .env.'
            } elseif ($null -ne (Get-Command codex -ErrorAction SilentlyContinue)) {
                Write-Output 'PATH-discovered Codex command was not saved to .env.'
            } else {
                Write-Output 'CODEX_EXE remains unset; no Codex command was found on PATH.'
            }
        }
    }
}

Write-Output 'Discovering Codex Desktop executable.'
Invoke-Python -Arguments @('codex_desktop_bridge.py', 'discover_codex')

if ($SkipSteeringConfig) {
    Write-Output 'Skipping steering config: installer no longer changes Codex Desktop follow-up mode.'
}

if ($SkipCodexPlugin) {
    Write-Output 'Skipping Codex plugin install.'
} else {
    if (-not (Test-Path -LiteralPath $PluginMarketplacePath)) {
        throw "Codex plugin marketplace was not found: $PluginMarketplacePath"
    }
    if (-not (Test-Path -LiteralPath $PluginManifestPath)) {
        throw "INSTALL_INCOMPLETE: Codex plugin manifest was not found: $PluginManifestPath"
    }
    if (-not (Test-Path -LiteralPath $PluginInventoryVerifierPath)) {
        throw "INSTALL_INCOMPLETE: Codex plugin inventory verifier was not found: $PluginInventoryVerifierPath"
    }
    try {
        Write-Output 'Installing Codex plugin marketplace from this repository.'
        Invoke-Codex -Arguments @('plugin', 'marketplace', 'add', $ScriptDir)
        Write-Output "Installing Codex plugin: $PluginRef"
        Invoke-Codex -Arguments @('plugin', 'add', $PluginRef)
    } catch {
        throw "INSTALL_INCOMPLETE: Codex plugin installation failed; !pro is not ready. $($_.Exception.Message)"
    }
    Test-CodexPluginInventory
}

if ($DryRun) {
    Write-Output 'Dry run complete.'
    Write-Output 'Plugin inventory was not verified.'
} else {
    $utf8NoBom = [Text.UTF8Encoding]::new($false)
    [IO.File]::WriteAllText($RuntimeModePath, "$Runtime`n", $utf8NoBom)
    Write-Output "Configured runtime mode: $Runtime"
    Write-Output 'Install complete.'
    Write-Output 'Setup required: run .\setup-discord-bot.ps1 and paste the Discord bot token when prompted.'
    Write-Output 'After setup, restart Codex so bundled skills reload, then run .\codex-discord-bot.cmd'
    Write-Output 'Rollback remains available with: .\install.ps1 -Runtime python -SkipDependencies'
}
