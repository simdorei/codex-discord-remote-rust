# All Rust maintenance entry points share this process-lifetime OS file lock.
# A crashed owner releases its handle automatically. Never delete the lock file.
function Enter-CdrControl {
    param([string]$Root, [switch]$MaintenanceV2)
    $path = Join-Path ([IO.Path]::GetFullPath($Root)) '.codex_discord_rust.control.lock'
    try { $handle = [IO.File]::Open($path, 'OpenOrCreate', 'ReadWrite', 'None') }
    catch [IO.IOException] {
        if (($_.Exception.HResult -band 0xffff) -in @(32, 33)) {
            throw 'cdr_control_busy: another maintenance operation owns the control lock'
        }
        throw
    }
    try { if (-not $MaintenanceV2) { Assert-CdrNoMaintenanceV2 -Root $Root }; return $handle }
    catch { $handle.Dispose(); throw }
}

function Get-CdrArtifactHash {
    param([string]$Path)
    $sha = [Security.Cryptography.SHA256]::Create()
    $stream = [IO.File]::OpenRead($Path)
    try { return [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '') }
    finally { $stream.Dispose(); $sha.Dispose() }
}

# Caller holds the common control lock. Unrelated operations cannot adopt even
# a prepared journal; the original recovery entry owns all journal phases.
function Assert-CdrNoPendingRestart {
    param([string]$Root)
    Assert-CdrNoMaintenanceV2 -Root $Root
    foreach ($name in @('.codex_discord_rust.restart.launch',
        '.codex_discord_rust.restart', '.codex_discord_rust.drain.prepare',
        '.codex_discord_rust.drain.ack')) {
        if (Test-Path -LiteralPath (Join-Path $Root $name)) {
            throw 'Pending restart operation preserved (restart fence); unrelated maintenance refused'
        }
    }
    Assert-CdrNoOrphanRestartClaim -Root $Root
}

function Assert-CdrNoMaintenanceV2 {
    param([string]$Root)
    # The state IS the ownership marker. There is no state/marker publication gap.
    if ([IO.File]::Exists((Join-Path $Root '.codex_discord_rust.maintenance.v2'))) {
        throw 'maintenance_v2_pending: dedicated recovery owns this operation; legacy action refused'
    }
}

function Assert-CdrNoOrphanRestartClaim {
    param([string]$Root)
    if ([IO.Directory]::GetFiles($Root, '.codex_discord_rust.restart.claimed.*').Count) {
        throw 'Pending restart operation claim preserved; unrelated maintenance refused'
    }
}

function Write-NewCdrMarker {
    param([string]$Path, [string]$Text)
    # Publish a complete file, without replacing an existing marker.
    $temporary = $Path + '.new.' + [guid]::NewGuid().ToString('N')
    try {
        [IO.File]::WriteAllText($temporary, $Text, [Text.UTF8Encoding]::new($false))
        [IO.File]::Move($temporary, $Path)
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

function Assert-CdrMarkerOwner {
    param([string]$Path, [string]$Text)
    if ([IO.File]::Exists($Path) -and [IO.File]::ReadAllText($Path) -cne $Text) {
        throw 'Foreign maintenance marker preserved; recovery refused'
    }
}

function Wait-CdrReplacementReady {
    param([string]$ExpectedIdentity, [int]$TimeoutSeconds = 15)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        $process = Get-VerifiedRuntimeProcess
        if ($null -eq $process -or (Get-RustProcessIdentity $process) -cne $ExpectedIdentity) {
            throw 'Replacement runtime identity changed before completion'
        }
        $health = Get-HeartbeatHealth -Process $process
        if ($health.Healthy -and -not $health.Bootstrap) { return }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $deadline)
    throw 'Replacement runtime did not publish a fresh matching heartbeat'
}

function Write-CdrRestartCompletion {
    param($Fence, [string]$ExpectedChildIdentity)
    $identity = Get-VerifiedRuntimeIdentity
    if ($ExpectedChildIdentity -and $identity -cne $ExpectedChildIdentity) {
        throw 'Runtime lock belongs to a different child; completion refused'
    }
    if ([string]::IsNullOrWhiteSpace($identity) -or $identity -ceq $Fence.ProcessIdentity) {
        throw 'Restart did not produce a verified replacement identity'
    }
    Wait-CdrReplacementReady -ExpectedIdentity $identity
    $receipt = [ordered]@{
        RuntimeId=$Fence.RuntimeId; ProcessIdentity=$Fence.ProcessIdentity
        Nonce=$Fence.Nonce; ReplacementIdentity=$identity
    }
    Write-AtomicRestartMarker -Path (Join-Path $RepoRoot '.codex_discord_rust.restart.completed') `
        -Text ($receipt | ConvertTo-Json -Compress)
}

function Test-CdrRestartCompleted {
    param($Fence)
    $path = Join-Path $RepoRoot '.codex_discord_rust.restart.completed'
    if (-not [IO.File]::Exists($path)) { return $false }
    $receipt = [IO.File]::ReadAllText($path) | ConvertFrom-Json
    if (-not (Test-RestartDrainFenceMatch $receipt $Fence)) { return $false }
    Assert-CdrNoOrphanRestartClaim -Root $RepoRoot
    foreach ($name in @('restart', 'drain.prepare', 'drain.ack')) {
        if ([IO.File]::Exists((Join-Path $RepoRoot ('.codex_discord_rust.' + $name)))) {
            throw 'Another maintenance intent is active; restart completion cannot be certified'
        }
    }
    if ([IO.File]::Exists((Join-Path $RepoRoot '.codex_discord_rust.stop')) -or
        [IO.File]::Exists((Join-Path $RepoRoot '.codex_discord_bot.disabled'))) {
        throw 'Another maintenance intent is active; restart completion cannot be certified'
    }
    Wait-CdrReplacementReady -ExpectedIdentity $receipt.ReplacementIdentity
    return $true
}
