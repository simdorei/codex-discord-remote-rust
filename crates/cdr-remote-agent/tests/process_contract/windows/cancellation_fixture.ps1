$ErrorActionPreference = 'Stop'
function Trace([string]$message) {
    [IO.File]::AppendAllText($env:CDR_PROCESS_CONTRACT_TRACE_PATH, $message + [Environment]::NewLine)
}
Trace ('parent-entry pid=' + $PID)
try {
    if ($env:CDR_PROCESS_CONTRACT_DIAG_CASE -eq 'early-error') { throw 'intentional diagnostic early error' }
    Trace 'child-launch-attempt'
    $childCode = '[IO.File]::WriteAllText($env:CDR_PROCESS_CONTRACT_CHILD_TRACE_PATH, [string]$PID); Start-Sleep -Seconds 30'
    $startInfo = [Diagnostics.ProcessStartInfo]::new('powershell.exe')
    $startInfo.Arguments = '-NoProfile -NonInteractive -Command ' + $childCode
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $p = [Diagnostics.Process]::Start($startInfo)
    Trace ('child-launch-return pid=' + $p.Id + ' alive=' + (-not $p.HasExited))
    [Console]::Out.Write($p.Id)
    [Console]::Out.Flush()
    [Console]::Error.Write('cancel-ready')
    [Console]::Error.Flush()
    if ($env:CDR_PROCESS_CONTRACT_DIAG_CASE -eq 'missing-pid') {
        Trace 'intentional missing PID publication'
    } elseif ($env:CDR_PROCESS_CONTRACT_DIAG_CASE -eq 'malformed-pid') {
        [IO.File]::WriteAllText($env:CDR_PROCESS_CONTRACT_PID_PATH, 'not-a-pid')
    } else {
        [IO.File]::WriteAllText($env:CDR_PROCESS_CONTRACT_PID_PATH, [string]$p.Id)
    }
    Trace 'parent-publication-step-returned'
    Start-Sleep -Seconds 30
} catch {
    $detail = $_.Exception.Message
    Trace ('launcher-error: ' + $detail.Substring(0, [Math]::Min(512, $detail.Length)))
    throw
}
