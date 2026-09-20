function Request-BotRestart {
    if ($RuntimeMode -ne 'rust') { throw 'Tray restart only supports the Rust runtime.' }
    $expectedIdentity = Get-RustTrayProcessIdentity -Process (Get-RustTrayProcess)
    if ([string]::IsNullOrWhiteSpace($expectedIdentity)) {
        throw 'Running Rust bot process identity could not be verified.'
    }
    $restartScript = Join-Path $ScriptDir 'codex-discord-rust-restart.ps1'
    if (-not (Test-Path -LiteralPath $restartScript -PathType Leaf)) {
        throw "Rust restart script not found: $restartScript"
    }
    # A normally returning PowerShell script does not update LASTEXITCODE.
    # Isolate this invocation from earlier native-command results.
    $priorExitCode = $global:LASTEXITCODE
    try {
        $global:LASTEXITCODE = 0
        $result = & $restartScript -RepoRoot $ScriptDir -ExpectedBotIdentity $expectedIdentity
        if ($global:LASTEXITCODE -ne 0) {
            throw "Rust restart request failed with exit code $global:LASTEXITCODE"
        }
    } finally {
        $global:LASTEXITCODE = $priorExitCode
    }
    Write-LauncherLog "tray_restart_requested runtime=rust result=$result"

}

function Request-BotForceRestart {
    if ($RuntimeMode -ne 'rust') { throw 'Tray force restart only supports the Rust runtime.' }
    $identity = Get-RustTrayProcessIdentity -Process (Get-RustTrayProcess)
    $restartScript = Join-Path $ScriptDir 'codex-discord-rust-restart.ps1'
    if (-not [IO.File]::Exists($restartScript)) { throw 'Rust restart script not found.' }
    $arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
        ('"' + $restartScript + '"'), '-RepoRoot', ('"' + $ScriptDir + '"'), '-Force')
    if ($identity) { $arguments += @('-ExpectedBotIdentity', $identity) }
    # Separate process: the UI stays responsive while the old runtime is killed
    # and the replacement publishes its heartbeat. No quiet/drain wait is queued.
    $helper = Start-Process -FilePath 'powershell.exe' -ArgumentList $arguments `
        -WindowStyle Hidden -PassThru
    Write-LauncherLog "tray_force_restart_requested identity=$identity helper_pid=$($helper.Id)"
}
