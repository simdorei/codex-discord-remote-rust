#![cfg(windows)]
#![allow(clippy::too_many_lines)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn run_pwsh(script: &str) {
    let mut cmd = Command::new("powershell.exe");
    for (key, _) in std::env::vars_os() {
        if let Some(name) = key.to_str()
            && (name.starts_with("CODEX_REMOTE_MCP_")
                || name.starts_with("DISCORD_")
                || name == "CODEX_MCP_DATA_DIR"
                || name == "CODEX_BOT_DATA_DIR")
        {
            cmd.env_remove(name);
        }
    }
    let output = cmd
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-STA",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .current_dir(repo_root())
        .output()
        .expect("execute powershell test script");
    assert!(
        output.status.success(),
        "PowerShell contract failed (code {:?}):\nSTDOUT:\n{}\nSTDERR:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn t08_tray_crash_preserves_bot_and_mcp_and_launcher_restores_tray_without_starting_manually_stopped_bot()
 {
    let ws = repo_root();
    let tray_source = fs::read_to_string(ws.join("codex-discord-tray.ps1")).expect("read tray");
    assert!(
        !tray_source.contains("'Exit tray icon'"),
        "codex-discord-tray.ps1 must not retain old 'Exit tray icon' label"
    );
    assert!(
        !tray_source.contains("Exit tray icon only"),
        "codex-discord-tray.ps1 must not add 'Exit tray icon only' item"
    );
    assert!(
        tray_source.contains("전체 종료"),
        "codex-discord-tray.ps1 must label the full shutdown action '전체 종료'"
    );
    for forbidden_mcp_menu in [
        "Start MCP",
        "Stop MCP",
        "Restart MCP",
        "MCP 시작",
        "MCP 중지",
        "MCP 재시작",
    ] {
        assert!(
            !tray_source.contains(forbidden_mcp_menu),
            "codex-discord-tray.ps1 must not include MCP start/stop/restart menu item: {forbidden_mcp_menu}"
        );
    }

    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$ws = '{ws}'
$temp = Join-Path ([IO.Path]::GetTempPath()) ('cdr_t08_' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $temp 'scripts') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $temp 'bot_data') -Force | Out-Null
foreach ($f in @('codex-discord-tray.ps1','codex-discord-tray-runtime.ps1','codex-discord-tray-restart-runtime.ps1','codex-discord-rust-control.ps1','codex-discord-bot.cmd')) {{
    Copy-Item -LiteralPath (Join-Path $ws $f) -Destination (Join-Path $temp $f) -Force
}}
foreach ($f in Get-ChildItem -LiteralPath (Join-Path $ws 'scripts') -File) {{
    Copy-Item -LiteralPath $f.FullName -Destination (Join-Path $temp ('scripts\' + $f.Name)) -Force
}}
[IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_OWNERSHIP=external`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://127.0.0.1:9/bridge`nCODEX_REMOTE_MCP_DEVICE_ID=dev-t08`nCODEX_REMOTE_MCP_DEVICE_TOKEN=tok`n")

$psExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$botCoopScript = Join-Path $temp 'bot_coop.ps1'
[IO.File]::WriteAllText($botCoopScript, @"
`$stopFile = Join-Path '$($temp.Replace("'", "''"))' '.codex_discord_rust.stop'
while (-not [IO.File]::Exists(`$stopFile)) {{
    Start-Sleep -Milliseconds 40
}}
"@)
$mcpCoopScript = Join-Path $temp 'mcp_coop.ps1'
[IO.File]::WriteAllText($mcpCoopScript, @"
`$stopFile = Join-Path '$($temp.Replace("'", "''"))' 'bot_data\remote_mcp.stop'
while (-not [IO.File]::Exists(`$stopFile)) {{
    Start-Sleep -Milliseconds 40
}}
"@)

$botProc = $null
$mcpProc = $null
$trayProc = $null
$reopenedTrayProc = $null
try {{
    $ScriptDir = $temp
    $RuntimeMode = 'rust'
    $LauncherLogPath = Join-Path $temp 'launcher.log'
    . (Join-Path $temp 'scripts\CdrLaunchJournal.ps1')
    . (Join-Path $temp 'codex-discord-tray-runtime.ps1')

    # Verify Windows PowerShell 5.1 Parser.ParseFile reads codex-discord-tray.ps1 with UTF-8 BOM
    # and produces exact codepoints [51204,52404,32,51333,47308] ('전체 종료') for $exitItem.Text
    $trayPath = Join-Path $temp 'codex-discord-tray.ps1'
    $trayBytes = [IO.File]::ReadAllBytes($trayPath)
    if ($trayBytes.Length -lt 3 -or $trayBytes[0] -ne 0xEF -or $trayBytes[1] -ne 0xBB -or $trayBytes[2] -ne 0xBF) {{
        throw 'codex-discord-tray.ps1 must have a UTF-8 BOM for Windows PowerShell 5.1 parser compatibility'
    }}
    $tokens = $null; $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile($trayPath, [ref]$tokens, [ref]$parseErrors)
    if ($parseErrors.Count -gt 0) {{
        throw "codex-discord-tray.ps1 had PS5 parse errors: $($parseErrors | Out-String)"
    }}
    $exitAssign = $ast.FindAll({{
        param($n)
        $n -is [System.Management.Automation.Language.AssignmentStatementAst] -and
        $n.Left.Extent.Text -eq '$exitItem.Text'
    }}, $true) | Select-Object -First 1
    if ($null -eq $exitAssign) {{ throw 'Could not find $exitItem.Text assignment in AST' }}
    $strNode = $exitAssign.Right.FindAll({{
        param($n) $n -is [System.Management.Automation.Language.StringConstantExpressionAst]
    }}, $true) | Select-Object -First 1
    $exitCodepoints = ($strNode.Value.ToCharArray() | ForEach-Object {{ [int][char]$_ }}) -join ','
    if ($exitCodepoints -cne '51204,52404,32,51333,47308') {{
        throw "Expected $exitItem.Text AST value codepoints 51204,52404,32,51333,47308 ('전체 종료'), got: $exitCodepoints"
    }}

    # Verify production default GraceTimeoutMs is 45000 ms (not a 3000 ms fixture shortcut)
    foreach ($fnName in @('Stop-CdrComponentRuntime', 'Request-BotStop', 'Invoke-CdrTrayExitAll')) {{
        $cmdInfo = Get-Command $fnName -ErrorAction Stop
        $paramAst = $cmdInfo.ScriptBlock.Ast.FindAll({{
            param($n) $n -is [System.Management.Automation.Language.ParameterAst] -and $n.Name.VariablePath.UserPath -ceq 'GraceTimeoutMs'
        }}, $true) | Select-Object -First 1
        if ($null -eq $paramAst -or $paramAst.DefaultValue.Extent.Text -cne '45000') {{
            throw "Expected $fnName default GraceTimeoutMs to be 45000, got: $($paramAst.DefaultValue.Extent.Text)"
        }}
    }}
    $trayExitParam = $ast.FindAll({{
        param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -ceq 'Invoke-TrayExitAllAction'
    }}, $true) | Select-Object -First 1
    if ($null -eq $trayExitParam -or $trayExitParam.Extent.Text -notmatch 'GraceTimeoutMs\s*=\s*45000') {{
        throw 'Expected Invoke-TrayExitAllAction in codex-discord-tray.ps1 to default GraceTimeoutMs = 45000'
    }}

    # Verify Get-McpTrayStatus fails closed on incomplete or invalid ExternalMcpStatusRecord
    $selfProc = Get-Process -Id $PID -ErrorAction Stop
    $selfId = Get-CdrProcessIdentity -Process $selfProc
    $selfExe = [string]$selfProc.Path
    $selfSha = (Get-FileHash -LiteralPath $selfExe -Algorithm SHA256).Hash.ToLowerInvariant()
    $nowMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $statusPath = Join-Path $temp 'bot_data\remote_mcp_status.json'
    [IO.File]::WriteAllText($statusPath, (@{{
        pid = $PID
        process_identity = $selfId
        executable_path = $selfExe
        ready = $true
        hello_ack = $true
        draining = $false
        updated_at_ms = $nowMs
    }} | ConvertTo-Json -Compress))
    $incompleteObs = Get-McpTrayStatus -Root $temp
    if ($incompleteObs.State -ne 'unknown' -or $incompleteObs.Ready -or $incompleteObs.Running -or $incompleteObs.Diagnostic -notlike 'mcp_status_missing_field:*') {{
        throw "Expected Get-McpTrayStatus to reject incomplete status record, got: $($incompleteObs | ConvertTo-Json -Compress)"
    }}
    [IO.File]::WriteAllText($statusPath, (@{{
        schema_version = 1
        component = 'mcp'
        install_root = $temp
        pid = $PID
        process_identity = $selfId
        executable_path = $selfExe
        artifact_sha256 = $selfSha
        build_info = @{{ schema_version = 1; package_version = '0.1.0'; source_state = 'clean' }}
        updated_at_ms = $nowMs
        heartbeat_at_ms = $nowMs
        device_id = 'dev-t08'
        bridge_fingerprint = 'fp-t08'
        hello_ack = $true
        ready = $true
        draining = $false
        generation = 0
    }} | ConvertTo-Json -Compress))
    $zeroGenObs = Get-McpTrayStatus -Root $temp
    if ($zeroGenObs.State -ne 'unknown' -or $zeroGenObs.Ready -or $zeroGenObs.Diagnostic -ne 'mcp_status_generation_invalid') {{
        throw "Expected Get-McpTrayStatus to reject generation=0, got: $($zeroGenObs | ConvertTo-Json -Compress)"
    }}
    $validPaths = Resolve-CdrRemoteMcpPaths -RepoRoot $temp
    [IO.File]::WriteAllText($statusPath, (@{{
        schema_version = 1
        component = 'mcp'
        install_root = $temp
        pid = $PID
        process_identity = $selfId
        executable_path = $selfExe
        artifact_sha256 = $selfSha
        build_info = @{{ schema_version = 1; package_version = '0.1.0'; source_state = 'clean' }}
        updated_at_ms = $nowMs
        heartbeat_at_ms = $nowMs
        connection_verified_at_ms = $nowMs
        device_id = 'dev-t08'
        bridge_fingerprint = $validPaths.BridgeFingerprint
        hello_ack = $true
        ready = $true
        draining = $false
        generation = 1
    }} | ConvertTo-Json -Compress))
    # Verify complete status fails closed when remote MCP is disabled in .env
    [IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`nCODEX_REMOTE_MCP_ENABLED=0`nCODEX_REMOTE_MCP_OWNERSHIP=external`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://127.0.0.1:9/bridge`nCODEX_REMOTE_MCP_DEVICE_ID=dev-t08`nCODEX_REMOTE_MCP_DEVICE_TOKEN=tok`n")
    $disabledObs = Get-McpTrayStatus -Root $temp
    if ($disabledObs.State -ne 'not_configured' -or $disabledObs.Ready) {{
        throw "Expected Get-McpTrayStatus to fail closed when remote MCP is disabled, got: $($disabledObs | ConvertTo-Json -Compress)"
    }}
    # Verify complete status fails closed when ownership is default embedded (CODEX_REMOTE_MCP_OWNERSHIP omitted)
    [IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://127.0.0.1:9/bridge`nCODEX_REMOTE_MCP_DEVICE_ID=dev-t08`nCODEX_REMOTE_MCP_DEVICE_TOKEN=tok`n")
    $defaultEmbeddedObs = Get-McpTrayStatus -Root $temp
    if ($defaultEmbeddedObs.State -ne 'ownership_mismatch' -or $defaultEmbeddedObs.Ready) {{
        throw "Expected Get-McpTrayStatus to fail closed on default embedded ownership, got: $($defaultEmbeddedObs | ConvertTo-Json -Compress)"
    }}
    # Verify complete status fails closed when device_id / bridge_url config is missing
    [IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_OWNERSHIP=external`n")
    $missingCfgObs = Get-McpTrayStatus -Root $temp
    if ($missingCfgObs.State -ne 'unknown' -or $missingCfgObs.Ready -or $missingCfgObs.Diagnostic -ne 'mcp_config_unavailable') {{
        throw "Expected Get-McpTrayStatus to fail closed when device/bridge config is missing, got: $($missingCfgObs | ConvertTo-Json -Compress)"
    }}
    # Restore valid external .env config and verify missing/null/stale/future/wrong-type connection_verified_at_ms all fail closed
    [IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_OWNERSHIP=external`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://127.0.0.1:9/bridge`nCODEX_REMOTE_MCP_DEVICE_ID=dev-t08`nCODEX_REMOTE_MCP_DEVICE_TOKEN=tok`n")
    foreach ($badMode in @('missing', 'null', 'stale', 'future', 'bad_type')) {{
        $nowCheck = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        $recBad = [ordered]@{{
            schema_version = 1; component = 'mcp'; install_root = $temp; pid = $PID; process_identity = $selfId
            executable_path = $selfExe; artifact_sha256 = $selfSha
            build_info = @{{ schema_version = 1; package_version = '0.1.0'; source_state = 'clean' }}
            updated_at_ms = $nowCheck; heartbeat_at_ms = $nowCheck
            device_id = 'dev-t08'; bridge_fingerprint = $validPaths.BridgeFingerprint
            hello_ack = $true; ready = $true; draining = $false; generation = 1
        }}
        if ($badMode -eq 'null') {{ $recBad['connection_verified_at_ms'] = $null }}
        elseif ($badMode -eq 'stale') {{ $recBad['connection_verified_at_ms'] = $nowCheck - 30000 }}
        elseif ($badMode -eq 'future') {{ $recBad['connection_verified_at_ms'] = $nowCheck + 30000 }}
        elseif ($badMode -eq 'bad_type') {{ $recBad['connection_verified_at_ms'] = 'not-an-int' }}
        [IO.File]::WriteAllText($statusPath, ($recBad | ConvertTo-Json -Compress))
        $badObs = Get-McpTrayStatus -Root $temp
        if ($badObs.Ready -or $badObs.State -eq 'running') {{
            throw "Expected Get-McpTrayStatus to reject connection_verified_at_ms mode=$badMode, got: $($badObs | ConvertTo-Json -Compress)"
        }}
    }}
    $nowValid = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    [IO.File]::WriteAllText($statusPath, (@{{
        schema_version = 1; component = 'mcp'; install_root = $temp; pid = $PID; process_identity = $selfId
        executable_path = $selfExe; artifact_sha256 = $selfSha
        build_info = @{{ schema_version = 1; package_version = '0.1.0'; source_state = 'clean' }}
        updated_at_ms = $nowValid; heartbeat_at_ms = $nowValid; connection_verified_at_ms = $nowValid
        device_id = 'dev-t08'; bridge_fingerprint = $validPaths.BridgeFingerprint
        hello_ack = $true; ready = $true; draining = $false; generation = 1
    }} | ConvertTo-Json -Compress))
    $validObs = Get-McpTrayStatus -Root $temp
    if ($validObs.State -ne 'running' -or -not $validObs.Ready) {{
        throw "Expected Get-McpTrayStatus to accept complete status with valid external config, got: $($validObs | ConvertTo-Json -Compress)"
    }}
    Remove-Item -LiteralPath $statusPath -Force -ErrorAction SilentlyContinue

    Write-CdrAtomicJsonFile -Path (Join-Path $temp '.cdr-maintenance\mcp-ownership-state.json') -Object ([ordered]@{{
        schema_version = 1; mode = 'external'; state = 'committed'; committed = $true; updated_at_ms = 1
    }})
    Set-CdrComponentIntent -RepoRoot $temp -Component 'bot' -DesiredState 'running' -Action 'start' -Source 'test' | Out-Null
    Set-CdrComponentIntent -RepoRoot $temp -Component 'mcp' -DesiredState 'running' -Action 'start' -Source 'test' | Out-Null

    $botLaunch = Invoke-CdrJournaledProcessLaunch `
        -RepoRoot $temp -Component 'bot' -FilePath $psExe `
        -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',$botCoopScript) `
        -WorkingDirectory $temp -TargetBinaryPath $psExe -Action 'start-bot'
    $botProc = $botLaunch.Process

    $mcpLaunch = Invoke-CdrJournaledProcessLaunch `
        -RepoRoot $temp -Component 'mcp' -FilePath $psExe `
        -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',$mcpCoopScript) `
        -WorkingDirectory $temp -TargetBinaryPath $psExe -Action 'start-mcp'
    $mcpProc = $mcpLaunch.Process

    # Launch the real codex-discord-tray.ps1 process and wait until it holds the single-instance tray mutex
    $trayMutexName = Get-CdrTrayMutexName $temp
    $trayStart = Start-CdrTrayStandAlone -Root $temp
    if ($trayStart.State -ne 'spawn_requested' -or -not $trayStart.Pid) {{
        throw "Failed to start real tray process: $($trayStart | ConvertTo-Json -Compress)"
    }}
    $trayProc = Get-Process -Id ([int]$trayStart.Pid) -ErrorAction Stop
    $trayIdentity = Get-CdrProcessIdentity -Process $trayProc
    $mutexDeadline = [DateTime]::UtcNow.AddSeconds(6)
    while ([DateTime]::UtcNow -lt $mutexDeadline) {{
        if (Get-CdrTrayMutexBusy $trayMutexName) {{ break }}
        Start-Sleep -Milliseconds 50
    }}
    if (-not (Get-CdrTrayMutexBusy $trayMutexName)) {{
        throw 'Real tray process did not acquire its single-instance mutex within timeout'
    }}

    # Terminate the real tray UI process unexpectedly to simulate a tray crash
    Stop-CdrVerifiedProcess -ProcessId $trayProc.Id -ExpectedIdentity $trayIdentity -ExpectedBinaryPath $psExe | Out-Null
    $releaseDeadline = [DateTime]::UtcNow.AddSeconds(4)
    while ([DateTime]::UtcNow -lt $releaseDeadline) {{
        if (-not (Get-CdrTrayMutexBusy $trayMutexName)) {{ break }}
        Start-Sleep -Milliseconds 50
    }}

    # Verify both bot and MCP are STILL alive and their intents remain 'running'
    if (-not (Test-CdrProcessMatchesIdentity -ProcessId $botLaunch.ProcessId -ExpectedIdentity $botLaunch.ProcessIdentity -ExpectedBinaryPath $psExe)) {{
        throw 'Bot process died when tray process crashed!'
    }}
    if (-not (Test-CdrProcessMatchesIdentity -ProcessId $mcpLaunch.ProcessId -ExpectedIdentity $mcpLaunch.ProcessIdentity -ExpectedBinaryPath $psExe)) {{
        throw 'MCP process died when tray process crashed!'
    }}
    if ((Get-CdrComponentIntent -RepoRoot $temp -Component 'bot').DesiredState -ne 'running' -or
        (Get-CdrComponentIntent -RepoRoot $temp -Component 'mcp').DesiredState -ne 'running') {{
        throw 'Component intents were mutated by tray crash!'
    }}

    # Now gracefully stop ONLY the bot while MCP stays running (no force-kill!)
    $stopBotRes = Stop-CdrComponentRuntime -RepoRoot $temp -Component 'bot' -BinaryPath $psExe -GraceTimeoutMs 2000 -Source 'tray-stop-bot'
    if (-not $stopBotRes.Stopped) {{ throw "Expected cooperative bot to stop gracefully, got: $($stopBotRes | ConvertTo-Json -Compress)" }}
    if (-not (Test-CdrProcessMatchesIdentity -ProcessId $mcpLaunch.ProcessId -ExpectedIdentity $mcpLaunch.ProcessIdentity -ExpectedBinaryPath $psExe)) {{
        throw 'Stopping bot must not stop MCP!'
    }}
    if ((Get-CdrComponentIntent -RepoRoot $temp -Component 'bot').DesiredState -ne 'stopped' -or
        (Get-CdrComponentIntent -RepoRoot $temp -Component 'mcp').DesiredState -ne 'running') {{
        throw 'Bot stop mutated MCP intent!'
    }}

    # Invoke the public app reopen entrypoint (Invoke-CdrAppLaunch) while MCP is running and Bot is manually stopped:
    # It must restore ONLY the missing tray process and preserve both the running MCP and the manually stopped Bot!
    $reopen = Invoke-CdrAppLaunch -RepoRoot $temp -BotBinaryPath $psExe -McpBinaryPath $psExe
    if (-not $reopen.Reopened -or $reopen.BotState -ne 'stopped_by_intent' -or $reopen.McpState -ne 'already_running' -or $reopen.TrayState -ne 'spawn_requested') {{
        throw "Unexpected public app reopen result: $($reopen | ConvertTo-Json -Compress)"
    }}
    if ($reopen.TrayProcessId) {{
        $reopenedTrayProc = Get-Process -Id ([int]$reopen.TrayProcessId) -ErrorAction SilentlyContinue
    }}
    $reopenMutexDeadline = [DateTime]::UtcNow.AddSeconds(6)
    while ([DateTime]::UtcNow -lt $reopenMutexDeadline) {{
        if (Get-CdrTrayMutexBusy $trayMutexName) {{ break }}
        Start-Sleep -Milliseconds 50
    }}
    if (-not (Get-CdrTrayMutexBusy $trayMutexName)) {{
        throw 'Reopened tray process did not acquire its single-instance mutex'
    }}
    if ((Get-CdrComponentIntent -RepoRoot $temp -Component 'bot').DesiredState -ne 'stopped') {{
        throw 'App reopen must not clear manual bot stop intent while MCP is still running'
    }}
}} finally {{
    foreach ($p in @($botProc, $mcpProc, $trayProc, $reopenedTrayProc)) {{
        if ($null -ne $p) {{
            $id = Get-CdrProcessIdentity -Process $p
            if ($id) {{ Stop-CdrVerifiedProcess -ProcessId $p.Id -ExpectedIdentity $id -ExpectedBinaryPath $psExe | Out-Null }}
        }}
    }}
    $fullTemp = [IO.Path]::GetFullPath($temp)
    if ($fullTemp.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase)) {{
        Remove-Item -LiteralPath $fullTemp -Recurse -Force -ErrorAction SilentlyContinue
    }}
}}
"#,
        ws = ws.display()
    );
    run_pwsh(&script);
}

#[test]
fn t09_tray_exit_all_records_both_stop_intents_first_exits_only_when_both_stop_and_surfaces_partial_failure()
 {
    let ws = repo_root();
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$ws = '{ws}'
$temp = Join-Path ([IO.Path]::GetTempPath()) ('cdr_t09_' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $temp 'scripts') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $temp 'bot_data') -Force | Out-Null
foreach ($f in @('codex-discord-tray.ps1','codex-discord-tray-runtime.ps1','codex-discord-tray-restart-runtime.ps1','codex-discord-rust-control.ps1')) {{
    Copy-Item -LiteralPath (Join-Path $ws $f) -Destination (Join-Path $temp $f) -Force
}}
foreach ($f in Get-ChildItem -LiteralPath (Join-Path $ws 'scripts') -File) {{
    Copy-Item -LiteralPath $f.FullName -Destination (Join-Path $temp ('scripts\' + $f.Name)) -Force
}}
[IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_OWNERSHIP=external`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://127.0.0.1:9/bridge`nCODEX_REMOTE_MCP_DEVICE_ID=dev-t08`nCODEX_REMOTE_MCP_DEVICE_TOKEN=tok`n")

$psExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$pingExe = Join-Path $env:SystemRoot 'System32\ping.exe'
$botCoopScript = Join-Path $temp 'bot_coop.ps1'
[IO.File]::WriteAllText($botCoopScript, @"
`$stopFile = Join-Path '$($temp.Replace("'", "''"))' '.codex_discord_rust.stop'
while (-not [IO.File]::Exists(`$stopFile)) {{
    Start-Sleep -Milliseconds 30
}}
"@)
$mcpGateScript = Join-Path $temp 'mcp_gate.ps1'
$allowMcpExitFile = Join-Path $temp 'allow_mcp_exit.flag'
[IO.File]::WriteAllText($mcpGateScript, @"
`$stopFile = Join-Path '$($temp.Replace("'", "''"))' 'bot_data\remote_mcp.stop'
`$allowFile = '$($allowMcpExitFile.Replace("'", "''"))'
while (`$true) {{
    if ([IO.File]::Exists(`$stopFile) -and [IO.File]::Exists(`$allowFile)) {{
        break
    }}
    Start-Sleep -Milliseconds 30
}}
"@)

$botProc = $null
$mcpProc = $null
try {{
    $ScriptDir = $temp
    $RuntimeMode = 'rust'
    $LauncherLogPath = Join-Path $temp 'launcher.log'
    . (Join-Path $temp 'scripts\CdrLaunchJournal.ps1')
    . (Join-Path $temp 'codex-discord-tray-runtime.ps1')

    # Verify Windows PowerShell 5.1 Parser.ParseFile reads codex-discord-tray.ps1 with UTF-8 BOM
    # and produces exact codepoints [51204,52404,32,51333,47308] ('전체 종료') for $exitItem.Text
    $trayPath = Join-Path $temp 'codex-discord-tray.ps1'
    $trayBytes = [IO.File]::ReadAllBytes($trayPath)
    if ($trayBytes.Length -lt 3 -or $trayBytes[0] -ne 0xEF -or $trayBytes[1] -ne 0xBB -or $trayBytes[2] -ne 0xBF) {{
        throw 'codex-discord-tray.ps1 must have a UTF-8 BOM for Windows PowerShell 5.1 parser compatibility'
    }}
    $tokens = $null; $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile($trayPath, [ref]$tokens, [ref]$parseErrors)
    if ($parseErrors.Count -gt 0) {{
        throw "codex-discord-tray.ps1 had PS5 parse errors: $($parseErrors | Out-String)"
    }}
    $exitAssign = $ast.FindAll({{
        param($n)
        $n -is [System.Management.Automation.Language.AssignmentStatementAst] -and
        $n.Left.Extent.Text -eq '$exitItem.Text'
    }}, $true) | Select-Object -First 1
    if ($null -eq $exitAssign) {{ throw 'Could not find $exitItem.Text assignment in AST' }}
    $strNode = $exitAssign.Right.FindAll({{
        param($n) $n -is [System.Management.Automation.Language.StringConstantExpressionAst]
    }}, $true) | Select-Object -First 1
    $exitCodepoints = ($strNode.Value.ToCharArray() | ForEach-Object {{ [int][char]$_ }}) -join ','
    if ($exitCodepoints -cne '51204,52404,32,51333,47308') {{
        throw "Expected $exitItem.Text AST value codepoints 51204,52404,32,51333,47308 ('전체 종료'), got: $exitCodepoints"
    }}

    # Verify production default GraceTimeoutMs is 45000 ms (not a 3000 ms fixture shortcut)
    foreach ($fnName in @('Stop-CdrComponentRuntime', 'Request-BotStop', 'Invoke-CdrTrayExitAll')) {{
        $cmdInfo = Get-Command $fnName -ErrorAction Stop
        $paramAst = $cmdInfo.ScriptBlock.Ast.FindAll({{
            param($n) $n -is [System.Management.Automation.Language.ParameterAst] -and $n.Name.VariablePath.UserPath -ceq 'GraceTimeoutMs'
        }}, $true) | Select-Object -First 1
        if ($null -eq $paramAst -or $paramAst.DefaultValue.Extent.Text -cne '45000') {{
            throw "Expected $fnName default GraceTimeoutMs to be 45000, got: $($paramAst.DefaultValue.Extent.Text)"
        }}
    }}
    $trayExitParam = $ast.FindAll({{
        param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -ceq 'Invoke-TrayExitAllAction'
    }}, $true) | Select-Object -First 1
    if ($null -eq $trayExitParam -or $trayExitParam.Extent.Text -notmatch 'GraceTimeoutMs\s*=\s*45000') {{
        throw 'Expected Invoke-TrayExitAllAction in codex-discord-tray.ps1 to default GraceTimeoutMs = 45000'
    }}

    # Verify Get-McpTrayStatus fails closed on incomplete or invalid ExternalMcpStatusRecord
    $selfProc = Get-Process -Id $PID -ErrorAction Stop
    $selfId = Get-CdrProcessIdentity -Process $selfProc
    $selfExe = [string]$selfProc.Path
    $selfSha = (Get-FileHash -LiteralPath $selfExe -Algorithm SHA256).Hash.ToLowerInvariant()
    $nowMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $statusPath = Join-Path $temp 'bot_data\remote_mcp_status.json'
    [IO.File]::WriteAllText($statusPath, (@{{
        pid = $PID
        process_identity = $selfId
        executable_path = $selfExe
        ready = $true
        hello_ack = $true
        draining = $false
        updated_at_ms = $nowMs
    }} | ConvertTo-Json -Compress))
    $incompleteObs = Get-McpTrayStatus -Root $temp
    if ($incompleteObs.State -ne 'unknown' -or $incompleteObs.Ready -or $incompleteObs.Running -or $incompleteObs.Diagnostic -notlike 'mcp_status_missing_field:*') {{
        throw "Expected Get-McpTrayStatus to reject incomplete status record, got: $($incompleteObs | ConvertTo-Json -Compress)"
    }}
    [IO.File]::WriteAllText($statusPath, (@{{
        schema_version = 1
        component = 'mcp'
        install_root = $temp
        pid = $PID
        process_identity = $selfId
        executable_path = $selfExe
        artifact_sha256 = $selfSha
        build_info = @{{ schema_version = 1; package_version = '0.1.0'; source_state = 'clean' }}
        updated_at_ms = $nowMs
        heartbeat_at_ms = $nowMs
        device_id = 'dev-t08'
        bridge_fingerprint = 'fp-t08'
        hello_ack = $true
        ready = $true
        draining = $false
        generation = 0
    }} | ConvertTo-Json -Compress))
    $zeroGenObs = Get-McpTrayStatus -Root $temp
    if ($zeroGenObs.State -ne 'unknown' -or $zeroGenObs.Ready -or $zeroGenObs.Diagnostic -ne 'mcp_status_generation_invalid') {{
        throw "Expected Get-McpTrayStatus to reject generation=0, got: $($zeroGenObs | ConvertTo-Json -Compress)"
    }}
    $validPaths = Resolve-CdrRemoteMcpPaths -RepoRoot $temp
    [IO.File]::WriteAllText($statusPath, (@{{
        schema_version = 1
        component = 'mcp'
        install_root = $temp
        pid = $PID
        process_identity = $selfId
        executable_path = $selfExe
        artifact_sha256 = $selfSha
        build_info = @{{ schema_version = 1; package_version = '0.1.0'; source_state = 'clean' }}
        updated_at_ms = $nowMs
        heartbeat_at_ms = $nowMs
        connection_verified_at_ms = $nowMs
        device_id = 'dev-t08'
        bridge_fingerprint = $validPaths.BridgeFingerprint
        hello_ack = $true
        ready = $true
        draining = $false
        generation = 1
    }} | ConvertTo-Json -Compress))
    # Verify complete status fails closed when remote MCP is disabled in .env
    [IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`nCODEX_REMOTE_MCP_ENABLED=0`nCODEX_REMOTE_MCP_OWNERSHIP=external`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://127.0.0.1:9/bridge`nCODEX_REMOTE_MCP_DEVICE_ID=dev-t08`nCODEX_REMOTE_MCP_DEVICE_TOKEN=tok`n")
    $disabledObs = Get-McpTrayStatus -Root $temp
    if ($disabledObs.State -ne 'not_configured' -or $disabledObs.Ready) {{
        throw "Expected Get-McpTrayStatus to fail closed when remote MCP is disabled, got: $($disabledObs | ConvertTo-Json -Compress)"
    }}
    # Verify complete status fails closed when ownership is default embedded (CODEX_REMOTE_MCP_OWNERSHIP omitted)
    [IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://127.0.0.1:9/bridge`nCODEX_REMOTE_MCP_DEVICE_ID=dev-t08`nCODEX_REMOTE_MCP_DEVICE_TOKEN=tok`n")
    $defaultEmbeddedObs = Get-McpTrayStatus -Root $temp
    if ($defaultEmbeddedObs.State -ne 'ownership_mismatch' -or $defaultEmbeddedObs.Ready) {{
        throw "Expected Get-McpTrayStatus to fail closed on default embedded ownership, got: $($defaultEmbeddedObs | ConvertTo-Json -Compress)"
    }}
    # Verify complete status fails closed when device_id / bridge_url config is missing
    [IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_OWNERSHIP=external`n")
    $missingCfgObs = Get-McpTrayStatus -Root $temp
    if ($missingCfgObs.State -ne 'unknown' -or $missingCfgObs.Ready -or $missingCfgObs.Diagnostic -ne 'mcp_config_unavailable') {{
        throw "Expected Get-McpTrayStatus to fail closed when device/bridge config is missing, got: $($missingCfgObs | ConvertTo-Json -Compress)"
    }}
    # Restore valid external .env config and verify missing/null/stale/future/wrong-type connection_verified_at_ms all fail closed
    [IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_OWNERSHIP=external`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://127.0.0.1:9/bridge`nCODEX_REMOTE_MCP_DEVICE_ID=dev-t08`nCODEX_REMOTE_MCP_DEVICE_TOKEN=tok`n")
    foreach ($badMode in @('missing', 'null', 'stale', 'future', 'bad_type')) {{
        $nowCheck = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        $recBad = [ordered]@{{
            schema_version = 1; component = 'mcp'; install_root = $temp; pid = $PID; process_identity = $selfId
            executable_path = $selfExe; artifact_sha256 = $selfSha
            build_info = @{{ schema_version = 1; package_version = '0.1.0'; source_state = 'clean' }}
            updated_at_ms = $nowCheck; heartbeat_at_ms = $nowCheck
            device_id = 'dev-t08'; bridge_fingerprint = $validPaths.BridgeFingerprint
            hello_ack = $true; ready = $true; draining = $false; generation = 1
        }}
        if ($badMode -eq 'null') {{ $recBad['connection_verified_at_ms'] = $null }}
        elseif ($badMode -eq 'stale') {{ $recBad['connection_verified_at_ms'] = $nowCheck - 30000 }}
        elseif ($badMode -eq 'future') {{ $recBad['connection_verified_at_ms'] = $nowCheck + 30000 }}
        elseif ($badMode -eq 'bad_type') {{ $recBad['connection_verified_at_ms'] = 'not-an-int' }}
        [IO.File]::WriteAllText($statusPath, ($recBad | ConvertTo-Json -Compress))
        $badObs = Get-McpTrayStatus -Root $temp
        if ($badObs.Ready -or $badObs.State -eq 'running') {{
            throw "Expected Get-McpTrayStatus to reject connection_verified_at_ms mode=$badMode, got: $($badObs | ConvertTo-Json -Compress)"
        }}
    }}
    $nowValid = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    [IO.File]::WriteAllText($statusPath, (@{{
        schema_version = 1; component = 'mcp'; install_root = $temp; pid = $PID; process_identity = $selfId
        executable_path = $selfExe; artifact_sha256 = $selfSha
        build_info = @{{ schema_version = 1; package_version = '0.1.0'; source_state = 'clean' }}
        updated_at_ms = $nowValid; heartbeat_at_ms = $nowValid; connection_verified_at_ms = $nowValid
        device_id = 'dev-t08'; bridge_fingerprint = $validPaths.BridgeFingerprint
        hello_ack = $true; ready = $true; draining = $false; generation = 1
    }} | ConvertTo-Json -Compress))
    $validObs = Get-McpTrayStatus -Root $temp
    if ($validObs.State -ne 'running' -or -not $validObs.Ready) {{
        throw "Expected Get-McpTrayStatus to accept complete status with valid external config, got: $($validObs | ConvertTo-Json -Compress)"
    }}
    Remove-Item -LiteralPath $statusPath -Force -ErrorAction SilentlyContinue

    # Case 1: Graceful partial failure (cooperative bot stops on .codex_discord_rust.stop, stubborn MCP ignores remote_mcp.stop; NO force-kill!)
    Set-CdrComponentIntent -RepoRoot $temp -Component 'bot' -DesiredState 'running' -Action 'start' -Source 'test' | Out-Null
    Set-CdrComponentIntent -RepoRoot $temp -Component 'mcp' -DesiredState 'running' -Action 'start' -Source 'test' | Out-Null

    $botLaunch = Invoke-CdrJournaledProcessLaunch `
        -RepoRoot $temp -Component 'bot' -FilePath $psExe `
        -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',$botCoopScript) `
        -WorkingDirectory $temp -TargetBinaryPath $psExe -Action 'start-bot'
    $botProc = $botLaunch.Process

    $mcpLaunch = Invoke-CdrJournaledProcessLaunch `
        -RepoRoot $temp -Component 'mcp' -FilePath $psExe `
        -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',$mcpGateScript) `
        -WorkingDirectory $temp -TargetBinaryPath $psExe -Action 'start-mcp'
    $mcpProc = $mcpLaunch.Process

    $partialRes = Invoke-CdrTrayExitAll `
        -Root $temp `
        -BotBinaryPath $psExe `
        -McpBinaryPath $psExe `
        -GraceTimeoutMs 350

    if ($partialRes.AllStopped -or -not $partialRes.BotStopped -or $partialRes.McpStopped -or $partialRes.McpState -ne 'still_running' -or -not $partialRes.KeepTrayOpen) {{
        throw "Unexpected graceful partial failure result: $($partialRes | ConvertTo-Json -Compress)"
    }}
    # Ensure both intents are durably stopped and stubborn MCP was NOT force-killed
    if ((Get-CdrComponentIntent -RepoRoot $temp -Component 'bot').DesiredState -ne 'stopped' -or
        (Get-CdrComponentIntent -RepoRoot $temp -Component 'mcp').DesiredState -ne 'stopped') {{
        throw 'Stop intents must remain stopped after partial exit-all failure'
    }}
    if (Test-CdrProcessMatchesIdentity -ProcessId $botLaunch.ProcessId -ExpectedIdentity $botLaunch.ProcessIdentity -ExpectedBinaryPath $psExe) {{
        throw 'Cooperative bot should have exited gracefully'
    }}
    if (-not (Test-CdrProcessMatchesIdentity -ProcessId $mcpLaunch.ProcessId -ExpectedIdentity $mcpLaunch.ProcessIdentity -ExpectedBinaryPath $psExe)) {{
        throw 'Graceful whole-exit must NOT force-kill a non-cooperative MCP process!'
    }}

    # Verify tray UI retains the Exit-All failure diagnostic across timer status refreshes
    Add-Type -AssemblyName System.Drawing
    $notify = [pscustomobject]@{{ Text = ''; Icon = $null; Visible = $true }}
    $statusItem = [pscustomobject]@{{ Text = '' }}
    $mcpStatusItem = [pscustomobject]@{{ Text = '' }}
    $script:LastTrayIcon = $null
    $script:ExitAllFailureDiagnostic = "Bot=$($partialRes.BotStopped), MCP=$($partialRes.McpStopped)"
    function Limit-TrayText {{ param([string]$Text) if ($Text.Length -le 63) {{ return $Text }} return $Text.Substring(0, 60) + '...' }}
    function Write-TrayStatusError {{ param([string]$Diagnostic) }}

    $trayCode = [IO.File]::ReadAllText((Join-Path $temp 'codex-discord-tray.ps1'), [Text.UTF8Encoding]::UTF8)
    $updateFnMatch = [regex]::Match($trayCode, '(?ms)function Update-TrayStatus \{{.*?\n\}}')
    if (-not $updateFnMatch.Success) {{ throw 'Could not extract Update-TrayStatus from codex-discord-tray.ps1' }}
    Invoke-Expression $updateFnMatch.Value
    Update-TrayStatus
    if ($notify.Text -notlike '*(Bot=True, MCP=False)*' -or $statusItem.Text -notlike '*(Bot=True, MCP=False)*' -or $script:LastTrayIcon -ne 'Error') {{
        throw "Update-TrayStatus overwrote Exit-All failure diagnostic! notify.Text=$($notify.Text) statusItem.Text=$($statusItem.Text) icon=$($script:LastTrayIcon)"
    }}

    # Case 2: Path mismatch on live identity is classified as 'unknown' (not 'stopped') and holds Exit-All open
    $pathMismatchObs = Get-CdrProcessObservation -ProcessId $mcpLaunch.ProcessId -ExpectedIdentity $mcpLaunch.ProcessIdentity -ExpectedBinaryPath $pingExe
    if ($pathMismatchObs.State -ne 'unknown' -or $pathMismatchObs.Reason -ne 'path_mismatch') {{
        throw "Expected live process with mismatched binary path to be State=unknown, Reason=path_mismatch, got: $($pathMismatchObs | ConvertTo-Json -Compress)"
    }}
    $unknownExitRes = Invoke-CdrTrayExitAll -Root $temp -BotBinaryPath $psExe -McpBinaryPath $pingExe -GraceTimeoutMs 100
    if ($unknownExitRes.AllStopped -or $unknownExitRes.McpState -ne 'unknown' -or -not $unknownExitRes.KeepTrayOpen) {{
        throw "Expected unknown MCP observation to hold Exit-All open, got: $($unknownExitRes | ConvertTo-Json -Compress)"
    }}

    # Case 3: Retry Exit All when MCP now cooperates with remote_mcp.stop -> both stopped, tray allowed to close
    [IO.File]::WriteAllText($allowMcpExitFile, '1')
    $cleanRes = Invoke-CdrTrayExitAll `
        -Root $temp `
        -BotBinaryPath $psExe `
        -McpBinaryPath $psExe `
        -GraceTimeoutMs 2000
    if (-not $cleanRes.AllStopped -or -not $cleanRes.BotStopped -or -not $cleanRes.McpStopped -or $cleanRes.KeepTrayOpen) {{
        throw "Expected clean exit-all on retry, got: $($cleanRes | ConvertTo-Json -Compress)"
    }}
    if (Test-CdrProcessMatchesIdentity -ProcessId $mcpLaunch.ProcessId -ExpectedIdentity $mcpLaunch.ProcessIdentity -ExpectedBinaryPath $psExe) {{
        throw 'MCP process should have exited gracefully after allow_mcp_exit.flag'
    }}

    # Case 4 (R2 Race A): Newer MCP Start arrives while Bot leg is stopping (before MCP leg starts).
    # Late MCP stop leg must NOT recreate remote_mcp.stop, must NOT stop the new MCP process, and Exit-All must report AllStopped=$false.
    $raceMcpProc = $null
    $raceBotProc = $null
    try {{
        $raceA = Invoke-CdrTrayExitAll `
            -Root $temp `
            -BotBinaryPath $psExe `
            -McpBinaryPath $psExe `
            -GraceTimeoutMs 1000 `
            -OnAfterBotStop {{
                param($exitDecision, $botLeg)
                Set-CdrComponentIntent -RepoRoot $temp -Component 'mcp' -DesiredState 'running' -Action 'start' -Source 'newer-user-start-mcp' | Out-Null
                $newMcpLaunch = Invoke-CdrJournaledProcessLaunch `
                    -RepoRoot $temp -Component 'mcp' -FilePath $psExe `
                    -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',$mcpGateScript) `
                    -WorkingDirectory $temp -TargetBinaryPath $psExe -Action 'start-mcp'
                $script:raceMcpLaunch = $newMcpLaunch
            }}
        $raceMcpProc = $script:raceMcpLaunch.Process
        if ($raceA.AllStopped -or $raceA.McpStopped -or $raceA.McpState -ne 'still_running' -or -not $raceA.KeepTrayOpen) {{
            throw "Expected R2 Race A (newer MCP Start during Bot Stop) to keep MCP running and AllStopped=false, got: $($raceA | ConvertTo-Json -Compress)"
        }}
        if (Test-CdrManualStopRequested -RepoRoot $temp -Component 'mcp') {{
            throw 'Late Exit-All MCP leg recreated manual stop marker after newer MCP Start!'
        }}
        $mcpIntentAfterRaceA = Get-CdrComponentIntent -RepoRoot $temp -Component 'mcp'
        if ($mcpIntentAfterRaceA.DesiredState -ne 'running' -or $mcpIntentAfterRaceA.Source -ne 'newer-user-start-mcp') {{
            throw "Expected MCP intent to stay running after Race A, got: $($mcpIntentAfterRaceA | ConvertTo-Json -Compress)"
        }}
        if (-not (Test-CdrProcessMatchesIdentity -ProcessId $script:raceMcpLaunch.ProcessId -ExpectedIdentity $script:raceMcpLaunch.ProcessIdentity -ExpectedBinaryPath $psExe)) {{
            throw 'Newer MCP process should still be running after R2 Race A!'
        }}

        # Case 5 (R2 Race B): Newer Bot Start arrives while MCP leg is stopping.
        # Even though MCP stops cleanly and Bot leg initially stopped, final recheck must observe newer Bot Start and return AllStopped=$false.
        $raceB = Invoke-CdrTrayExitAll `
            -Root $temp `
            -BotBinaryPath $psExe `
            -McpBinaryPath $psExe `
            -GraceTimeoutMs 2000 `
            -OnAfterBotStop {{
                param($exitDecision, $botLeg)
                Set-CdrComponentIntent -RepoRoot $temp -Component 'bot' -DesiredState 'running' -Action 'start' -Source 'newer-user-start-bot' | Out-Null
                $newBotLaunch = Invoke-CdrJournaledProcessLaunch `
                    -RepoRoot $temp -Component 'bot' -FilePath $psExe `
                    -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',$botCoopScript) `
                    -WorkingDirectory $temp -TargetBinaryPath $psExe -Action 'start-bot'
                $script:raceBotLaunch = $newBotLaunch
            }}
        $raceBotProc = $script:raceBotLaunch.Process
        if ($raceB.AllStopped -or $raceB.BotStopped -or $raceB.BotState -ne 'still_running' -or -not $raceB.McpStopped -or -not $raceB.KeepTrayOpen) {{
            throw "Expected R2 Race B (newer Bot Start during MCP Stop) to report BotStopped=false, McpStopped=true, AllStopped=false, got: $($raceB | ConvertTo-Json -Compress)"
        }}
        if (Test-CdrManualStopRequested -RepoRoot $temp -Component 'bot') {{
            throw 'Bot manual stop marker should remain cleared after newer Bot Start in R2 Race B!'
        }}
        if (-not (Test-CdrProcessMatchesIdentity -ProcessId $script:raceBotLaunch.ProcessId -ExpectedIdentity $script:raceBotLaunch.ProcessIdentity -ExpectedBinaryPath $psExe)) {{
            throw 'Newer Bot process should still be running after R2 Race B!'
        }}
    }} finally {{
        foreach ($rp in @($raceMcpProc, $raceBotProc)) {{
            if ($null -ne $rp) {{
                $rid = Get-CdrProcessIdentity -Process $rp
                if ($rid) {{ Stop-CdrVerifiedProcess -ProcessId $rp.Id -ExpectedIdentity $rid -ExpectedBinaryPath $psExe | Out-Null }}
            }}
        }}
    }}
}} finally {{
    foreach ($p in @($botProc, $mcpProc)) {{
        if ($null -ne $p) {{
            $id = Get-CdrProcessIdentity -Process $p
            if ($id) {{ Stop-CdrVerifiedProcess -ProcessId $p.Id -ExpectedIdentity $id -ExpectedBinaryPath $psExe | Out-Null }}
        }}
    }}
    $fullTemp = [IO.Path]::GetFullPath($temp)
    if ($fullTemp.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase)) {{
        Remove-Item -LiteralPath $fullTemp -Recurse -Force -ErrorAction SilentlyContinue
    }}
}}
"#,
        ws = ws.display()
    );
    run_pwsh(&script);
}
