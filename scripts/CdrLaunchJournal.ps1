# Durable launch ownership. Completion receipts are derived from this record,
# never from whichever process happens to own the runtime lock at a later time.
function Read-CdrLaunchJournal {
    param([string]$Path)
    if (-not [IO.File]::Exists($Path)) { return $null }
    $record = [IO.File]::ReadAllText($Path) | ConvertFrom-Json
    if ($record.BinaryPath -cne $BinaryPath -or $record.Version -ne 1 -or
        $record.Phase -notin @('prepared', 'launching', 'child')) {
        throw 'Invalid launch journal preserved'
    }
    if ($record.ClaimPath) {
        $claim = [IO.Path]::GetFullPath($record.ClaimPath)
        if ([IO.Path]::GetDirectoryName($claim) -cne $RepoRoot -or
            [IO.Path]::GetFileName($claim) -notmatch '^\.codex_discord_rust\.restart\.claimed\.\d+\.\d+$') {
            throw 'Launch claim path is outside the owned restart namespace'
        }
    }
    return $record
}

function Save-CdrLaunchJournal {
    param([string]$Path, $Record)
    Write-AtomicRestartMarker -Path $Path -Text ($Record | ConvertTo-Json -Depth 8 -Compress)
}

function New-CdrLaunchJournal {
    param([string]$Path, [string]$Operation, $Fence = $null, [string]$ArtifactHash)
    $record = Read-CdrLaunchJournal $Path
    if ($null -ne $record) {
        if ($record.Operation -cne $Operation -or
            ($ArtifactHash -and $record.ArtifactHash -cne $ArtifactHash) -or
            ($null -ne $Fence -and -not (Test-RestartDrainFenceMatch $record.Fence $Fence))) {
            throw 'Foreign launch journal preserved'
        }
        return $record
    }
    $record = [pscustomobject]@{
        Version=1; BinaryPath=$BinaryPath; Operation=$Operation; Fence=$Fence
        Phase='prepared'; ChildIdentity=''; ClaimPath=''
        ArtifactHash=$ArtifactHash
    }
    Save-CdrLaunchJournal $Path $record
    return $record
}

function Set-CdrLaunchStarting {
    if (-not $script:CdrLaunchJournalPath) { return }
    $record = Read-CdrLaunchJournal $script:CdrLaunchJournalPath
    if ($record.Phase -ne 'prepared') { throw 'Launch already attempted; refusing duplicate start' }
    $record.Phase = 'launching'
    Save-CdrLaunchJournal $script:CdrLaunchJournalPath $record
}

function Set-CdrLaunchChild {
    param($Process)
    if (-not $script:CdrLaunchJournalPath) { return }
    $record = Read-CdrLaunchJournal $script:CdrLaunchJournalPath
    $identity = Get-RustProcessIdentity $Process
    if ($record.Phase -ne 'launching' -or $identity -notmatch '^\d+\|\d+$') {
        throw 'Launch child identity cannot be durably bound'
    }
    $record.ChildIdentity = $identity
    $record.Phase = 'child'
    Save-CdrLaunchJournal $script:CdrLaunchJournalPath $record
}

function Get-CdrRecordedChild {
    param($Record)
    if ($Record.Phase -eq 'launching') {
        throw 'launch_outcome_unknown: child creation was not durably recorded; no automatic duplicate launch'
    }
    if ($Record.Phase -eq 'prepared') { return $null }
    if ($Record.ChildIdentity -notmatch '^(\d+)\|\d+$') { throw 'Invalid recorded child identity' }
    $childPid = [int]$Matches[1]
    $process = Get-Process -Id $childPid -ErrorAction SilentlyContinue
    if ($null -eq $process) { return $null }
    if ($process.Path -cne $BinaryPath -or
        (Get-RustProcessIdentity $process) -cne $Record.ChildIdentity) {
        throw 'Recorded child PID now belongs to another instance; preserved'
    }
    return $process
}
