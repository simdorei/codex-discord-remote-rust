param([Parameter(Mandatory=$true)][string]$StatePath)
$ErrorActionPreference = 'Stop'
$state = Get-Content -LiteralPath $StatePath -Raw -Encoding UTF8 | ConvertFrom-Json
if ($null -ne $state.Version -and $state.Version -ne 1) {
    throw 'maintenance_v2_or_unknown: use dedicated maintenance entry'
}
$guard = $null
try {
    try { $guard = [IO.File]::Open($state.LockPath, 'OpenOrCreate', 'ReadWrite', 'None') }
    catch [IO.IOException] {
        $win32 = $_.Exception.HResult -band 0xffff
        if ($win32 -in @(32, 33)) { Write-Output 'deployment_busy'; exit 75 }
        throw
    }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $state.Watchdog `
        -RepoRoot $state.RepoRoot -BinaryPath $state.BinaryPath `
        -RecoverDeploymentStatePath $StatePath
    if ($LASTEXITCODE -ne 0) { throw "Recovery watchdog failed: $LASTEXITCODE" }
} finally {
    if ($null -ne $guard) { $guard.Dispose() }
}
