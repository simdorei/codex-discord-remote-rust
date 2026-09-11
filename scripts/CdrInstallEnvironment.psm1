function Get-EnvFileValue {
    param([string]$EnvPath, [string]$Name)

    if (-not (Test-Path -LiteralPath $EnvPath)) {
        return ''
    }
    foreach ($line in Get-Content -LiteralPath $EnvPath -Encoding UTF8) {
        $trimmed = ([string]$line).Trim()
        if (-not $trimmed -or $trimmed.StartsWith('#')) {
            continue
        }
        if (-not $trimmed.Contains('=')) { continue }
        $key, $value = $trimmed.Split('=', 2)
        if ($key.Trim() -ceq $Name) {
            return $value.Trim().Trim('"').Trim("'")
        }
    }
    return ''
}

function Get-DefaultCodexHomePath {
    return (Join-Path ([Environment]::GetFolderPath('UserProfile')) '.codex')
}

function Test-CodexRuntimeBinPath {
    param([string]$Path)

    $normalized = $Path.Replace('/', '\').TrimEnd('\').ToLowerInvariant()
    foreach ($directory in @(
        '\.sandbox-bin', '\plugins\.plugin-appserver',
        '\appdata\local\openai\codex\bin', '\app\resources'
    )) {
        if ($normalized.EndsWith($directory) -or $normalized.Contains($directory + '\')) {
            return $true
        }
    }
    return $false
}

function Resolve-CodexHomePath {
    param([string]$CodexHome)
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
        $provider = $null
        $drive = $null
        $resolved = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath(
            $expanded, [ref]$provider, [ref]$drive)
        if ($provider.Name -cne 'FileSystem') {
            throw 'CODEX_HOME must identify a FileSystem directory, not another PowerShell provider.'
        }
        if (Test-CodexRuntimeBinPath -Path $resolved) {
            throw 'CODEX_HOME points at a runtime executable directory, not a Codex profile.'
        }
        return $resolved
    }

    return $defaultCodexHome
}


Export-ModuleMember -Function Get-EnvFileValue, Resolve-CodexHomePath
