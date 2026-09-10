function Get-CodexEnvFileValue {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$Name
    )

    $envPath = Join-Path $RepoRoot '.env'
    if (-not (Test-Path -LiteralPath $envPath -PathType Leaf)) {
        return ''
    }
    $utf8NoBom = [System.Text.UTF8Encoding]::new($false)
    foreach ($line in [System.IO.File]::ReadAllLines($envPath, $utf8NoBom)) {
        $trimmed = $line.Trim()
        if ([string]::IsNullOrWhiteSpace($trimmed) -or $trimmed.StartsWith('#')) {
            continue
        }
        $separator = $trimmed.IndexOf('=')
        if ($separator -lt 1) {
            continue
        }
        $key = $trimmed.Substring(0, $separator).Trim()
        if (-not $key.Equals($Name, [StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        $value = $trimmed.Substring($separator + 1).Trim()
        if (
            $value.Length -ge 2 -and
            (($value[0] -eq '"' -and $value[$value.Length - 1] -eq '"') -or
            ($value[0] -eq "'" -and $value[$value.Length - 1] -eq "'"))
        ) {
            $value = $value.Substring(1, $value.Length - 2)
        }
        return $value
    }
    return ''
}

function Resolve-CodexPythonPath {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$Value,
        [Parameter(Mandatory = $true)][string]$Source
    )

    $expanded = [Environment]::ExpandEnvironmentVariables($Value.Trim())
    $resolved = if ([IO.Path]::IsPathRooted($expanded)) {
        [IO.Path]::GetFullPath($expanded)
    } else {
        [IO.Path]::GetFullPath((Join-Path $RepoRoot $expanded))
    }
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw "$Source executable not found: $resolved"
    }
    return $resolved
}

function Resolve-CodexRuntimePythonExecutable {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)

    $explicitOverride = $env:CODEX_DISCORD_PYTHON
    if (-not [string]::IsNullOrWhiteSpace($explicitOverride)) {
        return Resolve-CodexPythonPath `
            -RepoRoot $RepoRoot `
            -Value $explicitOverride `
            -Source 'CODEX_DISCORD_PYTHON'
    }
    $installedPython = $env:PYTHON_EXE
    if ([string]::IsNullOrWhiteSpace($installedPython)) {
        $installedPython = Get-CodexEnvFileValue -RepoRoot $RepoRoot -Name 'PYTHON_EXE'
    }
    if (-not [string]::IsNullOrWhiteSpace($installedPython)) {
        return Resolve-CodexPythonPath `
            -RepoRoot $RepoRoot `
            -Value $installedPython `
            -Source 'PYTHON_EXE'
    }
    $portable = Join-Path $RepoRoot '.python-portable\python.exe'
    if (-not (Test-Path -LiteralPath $portable -PathType Leaf)) {
        throw "Bot Python executable not found; configure PYTHON_EXE in $RepoRoot\.env or install $portable"
    }
    return $portable
}
