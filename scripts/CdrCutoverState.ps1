# Rust-only cutover support; loaded by codex-discord-runtime-cutover.ps1.
function Write-AtomicUtf8 {
    param([string]$Path, [string]$Content)
    $temporary = "$Path.tmp.$PID"
    $utf8 = [Text.UTF8Encoding]::new($false)
    [IO.File]::WriteAllText($temporary, $Content, $utf8)
    Move-Item -LiteralPath $temporary -Destination $Path -Force
}

function Read-KeyValueFile {
    param([string]$Path)
    $values = @{}
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $values }
    foreach ($line in (Get-Content -LiteralPath $Path -ErrorAction Stop)) {
        if ([string]$line -match '^([^=]+)=(.*)$') {
            $values[[string]$Matches[1]] = [string]$Matches[2]
        }
    }
    return $values
}

function Get-CutoverState {
    if (-not (Test-Path -LiteralPath $CutoverStatePath -PathType Leaf)) { return $null }
    $values = Read-KeyValueFile -Path $CutoverStatePath
    foreach ($key in @('transaction_id', 'owner_identity', 'source_runtime', 'target_runtime', 'phase')) {
        if ([string]::IsNullOrWhiteSpace([string]$values[$key])) {
            throw "Cutover recovery state is invalid: missing $key"
        }
    }
    if ($values.source_runtime -notin @('rust')) {
        throw 'Cutover recovery state uses an unsupported source runtime; recovery state preserved.'
    }
    if ($values.target_runtime -notin @('rust')) {
        throw 'Cutover recovery state uses an unsupported target runtime; recovery state preserved.'
    }
    return [pscustomobject]@{
        TransactionId = [string]$values.transaction_id
        OwnerIdentity = [string]$values.owner_identity
        SourceRuntime = [string]$values.source_runtime
        TargetRuntime = [string]$values.target_runtime
        Phase = [string]$values.phase
        TargetIdentity = [string]$values.target_identity
        CompletionRuntime = [string]$values.completion_runtime
    }
}

function Set-CutoverPhase {
    param($Transaction, [string]$Phase)
    $Transaction.OwnerIdentity = $CutoverIdentity
    $Transaction.Phase = $Phase
    $content = (
        "transaction_id=$($Transaction.TransactionId)`n" +
        "owner_identity=$($Transaction.OwnerIdentity)`n" +
        "source_runtime=$($Transaction.SourceRuntime)`n" +
        "target_runtime=$($Transaction.TargetRuntime)`n" +
        "phase=$Phase`n" +
        "target_identity=$($Transaction.TargetIdentity)`n" +
        "completion_runtime=$($Transaction.CompletionRuntime)`n" +
        "updated_at=$((Get-Date).ToUniversalTime().ToString('o'))`n"
    )
    Write-AtomicUtf8 -Path $CutoverStatePath -Content $content
}

function Get-DisableOwner {
    if (-not (Test-Path -LiteralPath $DisablePath -PathType Leaf)) { return $null }
    return Read-KeyValueFile -Path $DisablePath
}

function Enter-CutoverMaintenance {
    param($Transaction)
    $control = Enter-CdrControl -Root $RepoRoot
    try {
    Assert-CdrNoPendingRestart -Root $RepoRoot
    $owner = Get-DisableOwner
    if ($null -ne $owner) {
        if (
            $owner.kind -notin @('cutover', 'cutover_recovery') -or
            $owner.transaction_id -ne $Transaction.TransactionId
        ) {
            throw "Runtime is disabled by an operator-owned marker: $DisablePath"
        }
        return
    }
    Write-NewCdrMarker -Path $DisablePath -Text (
        "kind=cutover`n" +
        "transaction_id=$($Transaction.TransactionId)`n" +
        "cutover_identity=$CutoverIdentity`n"
    )
    } finally { $control.Dispose() }
}

function Exit-CutoverMaintenance {
    param($Transaction)
    $control = Enter-CdrControl -Root $RepoRoot
    try {
    $owner = Get-DisableOwner
    if ($null -eq $owner) { return }
    if (
        $owner.kind -notin @('cutover', 'cutover_recovery') -or
        $owner.transaction_id -ne $Transaction.TransactionId
    ) {
        throw 'Cutover maintenance marker ownership changed.'
    }
    Remove-Item -LiteralPath $DisablePath -Force
    } finally { $control.Dispose() }
}

function Ensure-CutoverRecoveryDisabled {
    param($Transaction)
    $control = Enter-CdrControl -Root $RepoRoot
    try {
    Assert-CdrNoPendingRestart -Root $RepoRoot
    if (Test-Path -LiteralPath $DisablePath -PathType Leaf) { return }
    Write-NewCdrMarker -Path $DisablePath -Text (
        "kind=cutover_recovery`n" +
        "transaction_id=$($Transaction.TransactionId)`n" +
        "recovery_required=true`n" +
        "phase=$($Transaction.Phase)`n"
    )
    } finally { $control.Dispose() }
}

function New-CutoverTransaction {
    param([string]$Source, [string]$Target)
    $control = Enter-CdrControl -Root $RepoRoot
    try {
    Assert-CdrNoPendingRestart -Root $RepoRoot
    if (Test-Path -LiteralPath $CutoverStatePath) { throw 'Existing cutover state preserved' }
    if (Test-Path -LiteralPath $DisablePath) { throw 'Existing maintenance intent preserved' }
    $transaction = [pscustomobject]@{
        TransactionId = [guid]::NewGuid().ToString('N')
        OwnerIdentity = $CutoverIdentity
        SourceRuntime = $Source
        TargetRuntime = $Target
        Phase = 'created'
    }
    Set-CutoverPhase -Transaction $transaction -Phase 'created'
    return $transaction
    } finally { $control.Dispose() }
}
