function Request-BotRestart {
    if ($RuntimeMode -eq 'rust') {
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
        return
    }
    $expectedIdentity = Get-CodexBotProcessIdentity `
        -BotScript $BotScript `
        -RuntimeLockPath $RuntimeLockPath
    if (-not $expectedIdentity) {
        throw 'running bot process identity could not be verified'
    }
    Publish-AtomicTextFile `
        -Path $RestartRequestPath `
        -Content "identity=$expectedIdentity"
    $task = Get-ScheduledTask -TaskName 'Codex Discord Bot' -ErrorAction SilentlyContinue
    if ($task -ne $null) {
        if (-not $task.Settings.Enabled) {
            Enable-ScheduledTask -TaskName 'Codex Discord Bot' | Out-Null
        }
        Start-ScheduledTask -TaskName 'Codex Discord Bot'
        Write-LauncherLog "tray_restart_requested task='Codex Discord Bot'"
        return
    }
    if (Test-Path -LiteralPath $HeadlessLauncher) {
        Start-Process `
            -FilePath 'wscript.exe' `
            -ArgumentList @("`"$HeadlessLauncher`"") `
            -WindowStyle Hidden
        Write-LauncherLog "tray_restart_requested launcher=$HeadlessLauncher"
    }
}
