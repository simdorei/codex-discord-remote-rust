# Bounded native command execution. An uncertain child is retained, NEVER killed/replayed.
. (Join-Path $PSScriptRoot 'CdrMaintenanceDiagnostics.ps1')
function Get-CdrCommandUtcNow { [DateTimeOffset]::UtcNow }
function Wait-CdrCommandExit($Process, $OutReader, $ErrReader) { $Process.WaitForExit(100) }

function ConvertTo-CdrMaintenanceArgument([string]$Value) {
    if ($Value.Contains('"') -or $Value.Contains("`n") -or $Value.Contains("`r")) {
        throw 'maintenance_native_argument_unsupported'
    }
    $trimmed=$Value.TrimEnd('\')
    return '"'+$trimmed+('\'*(2*($Value.Length-$trimmed.Length)))+'"'
}

function Invoke-CdrMaintenanceCommand($State, [string]$File, [string[]]$Arguments, [int]$LimitSeconds=0, [switch]$PassThru) {
    $statePath=Get-CdrMaintenancePath $RepoRoot
    if ($null -ne $State.ActiveCommand) { throw 'maintenance_command_outcome_unknown; recorded helper cannot be replayed' }
    $seconds=Get-CdrMaintenanceRemainingSeconds $State $LimitSeconds
    $deadline=(Get-CdrCommandUtcNow).AddSeconds($seconds)
    $info=[Diagnostics.ProcessStartInfo]::new()
    $info.FileName=$File
    $info.Arguments=(@($Arguments|ForEach-Object {ConvertTo-CdrMaintenanceArgument $_}) -join ' ')
    $info.WorkingDirectory=$RepoRoot; $info.UseShellExecute=$false
    $info.CreateNoWindow=$true; $info.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $info.RedirectStandardOutput=$true; $info.RedirectStandardError=$true
    $info.StandardOutputEncoding=[Text.UTF8Encoding]::new($false)
    $info.StandardErrorEncoding=[Text.UTF8Encoding]::new($false)
    $process=[Diagnostics.Process]::new(); $process.StartInfo=$info
    $State.ActiveCommand=[pscustomobject]@{Phase='launching';File=$File;Identity=''}
    Save-CdrMaintenanceState $State $statePath
    try {
        Assert-CdrMaintenanceDeadline $State
        if (-not $process.Start()) { throw 'maintenance_command_outcome_unknown; child creation not confirmed' }
        $ticks=$process.StartTime.ToUniversalTime().Ticks; $ticks-=($ticks%10)
        $State.ActiveCommand.Phase='child';$State.ActiveCommand.Identity="$($process.Id)|$ticks"
        Save-CdrMaintenanceState $State $statePath
        $outTask=$process.StandardOutput.ReadToEndAsync();$errTask=$process.StandardError.ReadToEndAsync()
        while ($true) {
            $exited=Wait-CdrCommandExit $process $outTask $errTask
            $observedAt=Get-CdrCommandUtcNow
            # Expiration wins over a simultaneous exit/readers-ready observation.
            # Keep the recorded child even if it already exited when observed late.
            if ($observedAt -ge $deadline -or $observedAt -ge [DateTimeOffset]::Parse($State.Deadline)) {
                throw 'maintenance_command_deadline_outcome_unknown; helper retained; no automatic retry'
            }
            if ($exited -and $outTask.IsCompleted -and $errTask.IsCompleted) { break }
            if ($exited) { Start-Sleep -Milliseconds 100 }
        }
        $code=$process.ExitCode
        # Both readers are started together, avoiding stdout/stderr pipe deadlock.
        $output=$outTask.GetAwaiter().GetResult(); $errorText=$errTask.GetAwaiter().GetResult()
        $safeOut=Get-CdrMaintenanceDiagnostic $output $EnvPath
        $safeErr=Get-CdrMaintenanceDiagnostic $errorText $EnvPath
        foreach ($text in @($safeOut,$safeErr)) {
            if ($text) { Write-Host $text }
        }
        $State.ActiveCommand=$null
        Save-CdrMaintenanceState $State $statePath
        Assert-CdrMaintenanceDeadline $State
        if ($code -ne 0) { throw "maintenance_native_command_failed exit=$code; stderr=$safeErr; stdout=$safeOut" }
        if ($PassThru) { return [pscustomobject]@{ExitCode=$code;Stdout=$output;Stderr=$errorText} }
    } finally { $process.Dispose() }
}
