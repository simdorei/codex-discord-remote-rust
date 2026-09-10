param([Parameter(Mandatory=$true)][string]$StatePath,
    [Parameter(Mandatory=$true)][ValidatePattern('^[a-f0-9]{32}$')][string]$ExpectedOperation)
$ErrorActionPreference='Stop'
$RepoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$expected = Join-Path $RepoRoot '.codex_discord_rust.maintenance.v2'
if ([IO.Path]::GetFullPath($StatePath) -cne $expected) { throw 'maintenance_state_path_not_owned' }
# Separate entry for BOTH initial worker and recurring recovery; same engine/budget.
$guard=$null
try {
    try { $guard=[IO.File]::Open(($expected+'.lock'),'OpenOrCreate','ReadWrite','None') }
    catch [IO.IOException] {
        if (($_.Exception.HResult -band 0xffff) -in @(32,33)) { Write-Output 'maintenance_busy'; exit 75 }
        throw
    }
    # Validate terminal receipts under the SAME common lock as active state.
    . (Join-Path $RepoRoot 'codex-discord-rust-control.ps1')
    $control=Enter-CdrControl $RepoRoot -MaintenanceV2
    try {
        if (-not [IO.File]::Exists($expected)) {
            if (-not [IO.File]::Exists($expected+'.completed')) { throw 'maintenance_state_missing; no fallback' }
            $receipt=[IO.File]::ReadAllText($expected+'.completed')|ConvertFrom-Json
            if ($receipt.Version -ne 2 -or $receipt.Operation -cne $ExpectedOperation -or $receipt.Phase -ne 'verified') {
                throw 'maintenance_expected_operation_mismatch; completed receipt preserved'
            }
            if ($receipt.ShutdownPolicy -cne 'live-handshake-v1') { throw 'maintenance_shutdown_policy_missing_or_unsupported' }
            Write-Output 'maintenance_previously_completed; no action or relaunch'
            exit 0
        }
        $pending=[IO.File]::ReadAllText($expected)|ConvertFrom-Json
        if ($pending.Operation -cne $ExpectedOperation) { throw 'maintenance_expected_operation_mismatch' }
    } finally { $control.Dispose() }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $RepoRoot 'codex-discord-rust-watchdog.ps1') `
        -RepoRoot $RepoRoot -MaintenanceStatePath $expected -ExpectedMaintenanceOperation $ExpectedOperation
    if ($LASTEXITCODE -ne 0) { throw "maintenance_worker_failed exit=$LASTEXITCODE; state/error preserved" }
} finally { if ($guard) { $guard.Dispose() } }
