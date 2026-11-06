#![cfg(windows)]
#![allow(clippy::too_many_lines)]

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use cdr_remote_agent::protocol::message::{
    GatewayCommand, GatewayInboundMessage, ProtocolV10, parse_bridge_message,
};
use cdr_remote_agent::protocol::request::{ProjectOperation, TerminalRequest, TerminalShell};
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

struct DiscordRuntimeDouble {
    http_addr: SocketAddr,
    ws_addr: SocketAddr,
    http_task: tokio::task::JoinHandle<()>,
    ws_task: tokio::task::JoinHandle<()>,
}

impl DiscordRuntimeDouble {
    async fn spawn() -> Self {
        let http_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind discord http double");
        let ws_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind discord ws double");
        let http_addr = http_listener.local_addr().expect("http addr");
        let ws_addr = ws_listener.local_addr().expect("ws addr");

        let ready_flag = Arc::new(AtomicBool::new(false));
        let help_reply_count = Arc::new(AtomicUsize::new(0));
        let msg_seq = Arc::new(AtomicU64::new(10));
        let ws_inject_tx = Arc::new(tokio::sync::Mutex::new(
            None::<tokio::sync::mpsc::UnboundedSender<String>>,
        ));

        let ready_for_ws = ready_flag.clone();
        let inject_for_ws = ws_inject_tx.clone();
        let ws_task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = ws_listener.accept().await else {
                    break;
                };
                let ready_conn = ready_for_ws.clone();
                let inject_conn = inject_for_ws.clone();
                tokio::spawn(async move {
                    let Ok(mut ws) = accept_async(stream).await else {
                        return;
                    };
                    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
                    *inject_conn.lock().await = Some(tx);
                    let _ = ws
                        .send(Message::Text(
                            r#"{"op":10,"d":{"heartbeat_interval":45000}}"#.into(),
                        ))
                        .await;
                    loop {
                        tokio::select! {
                            Some(payload) = rx.recv() => {
                                if ws.send(Message::Text(payload.into())).await.is_err() {
                                    break;
                                }
                            }
                            msg = ws.next() => {
                                match msg {
                                    Some(Ok(Message::Text(text))) => {
                                        if text.contains("\"op\":2") || text.contains("\"op\": 2") {
                                            let ready = format!(
                                                r#"{{"op":0,"s":1,"t":"READY","d":{{"v":10,"user":{{"id":"1000","username":"bot","discriminator":"0001","avatar":null,"bot":true,"mfa_enabled":false}},"guilds":[],"session_id":"sess-1","resume_gateway_url":"ws://{ws_addr}","application":{{"id":"1000","flags":0}}}}}}"#
                                            );
                                            let _ = ws.send(Message::Text(ready.into())).await;
                                            ready_conn.store(true, Ordering::SeqCst);
                                        } else if text.contains("\"op\":1") || text.contains("\"op\": 1") {
                                            let _ = ws.send(Message::Text(r#"{"op":11}"#.into())).await;
                                        }
                                    }
                                    Some(Ok(Message::Ping(p))) => {
                                        let _ = ws.send(Message::Pong(p)).await;
                                    }
                                    Some(Ok(Message::Close(_))) => {
                                        let _ = ws.flush().await;
                                        break;
                                    }
                                    Some(Ok(_)) => {}
                                    None | Some(Err(_)) => break,
                                }
                            }
                        }
                    }
                    ready_conn.store(false, Ordering::SeqCst);
                });
            }
        });

        let ready_for_http = ready_flag.clone();
        let help_for_http = help_reply_count.clone();
        let seq_for_http = msg_seq.clone();
        let inject_for_http = ws_inject_tx.clone();
        let http_task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = http_listener.accept().await else {
                    break;
                };
                let ready_req = ready_for_http.clone();
                let help_req = help_for_http.clone();
                let seq_req = seq_for_http.clone();
                let inject_req = inject_for_http.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    let mut header_end = None;
                    let mut content_len = 0usize;
                    loop {
                        let Ok(n) = stream.read(&mut tmp).await else {
                            return;
                        };
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                        if header_end.is_none()
                            && let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n")
                        {
                            header_end = Some(pos + 4);
                            let headers = String::from_utf8_lossy(&buf[..pos]);
                            for line in headers.lines() {
                                if let Some(rest) = line
                                    .strip_prefix("Content-Length:")
                                    .or_else(|| line.strip_prefix("content-length:"))
                                {
                                    content_len = rest.trim().parse::<usize>().unwrap_or(0);
                                }
                            }
                        }
                        if let Some(hend) = header_end
                            && buf.len() >= hend + content_len
                        {
                            break;
                        }
                    }
                    let req_str = String::from_utf8_lossy(&buf);
                    let first_line = req_str.lines().next().unwrap_or("");
                    let body_str = header_end
                        .and_then(|hend| buf.get(hend..))
                        .map(|b| String::from_utf8_lossy(b).into_owned())
                        .unwrap_or_default();

                    let (status_line, resp_body) = if first_line.contains("/api/v10/gateway/bot") {
                        (
                            "HTTP/1.1 200 OK",
                            format!(
                                r#"{{"url":"ws://{ws_addr}","shards":1,"session_start_limit":{{"total":1000,"remaining":999,"reset_after":14400000,"max_concurrency":1}}}}"#
                            ),
                        )
                    } else if first_line.contains("/api/v10/channels/42/messages")
                        && first_line.starts_with("POST ")
                    {
                        if body_str.contains("Codex Discord Remote") {
                            help_req.fetch_add(1, Ordering::SeqCst);
                        }
                        (
                            "HTTP/1.1 200 OK",
                            r#"{"id":"9001","channel_id":"42","content":"ok","author":{"id":"1000","username":"bot","discriminator":"0001","avatar":null,"bot":true},"timestamp":"2026-01-01T00:00:00.000000+00:00","edited_timestamp":null,"tts":false,"mention_everyone":false,"mentions":[],"mention_roles":[],"attachments":[],"embeds":[],"pinned":false,"type":0}"#.to_owned(),
                        )
                    } else if (first_line.contains("/api/v10/channels/42/messages")
                        && first_line.starts_with("GET "))
                        || first_line.contains("/commands")
                    {
                        ("HTTP/1.1 200 OK", "[]".to_owned())
                    } else if first_line.contains("/__test/inject_help") {
                        let before = help_req.load(Ordering::SeqCst);
                        let reply_deadline = tokio::time::Instant::now() + Duration::from_secs(15);
                        let mut last_send = None::<tokio::time::Instant>;
                        while help_req.load(Ordering::SeqCst) <= before
                            && tokio::time::Instant::now() < reply_deadline
                        {
                            if ready_req.load(Ordering::SeqCst)
                                && last_send.is_none_or(|t| t.elapsed() >= Duration::from_secs(1))
                            {
                                let seq = seq_req.fetch_add(1, Ordering::SeqCst);
                                let msg_id = 2000 + seq;
                                let event = format!(
                                    r#"{{"op":0,"s":{seq},"t":"MESSAGE_CREATE","d":{{"id":"{msg_id}","channel_id":"42","author":{{"id":"2001","username":"tester","discriminator":"0001","avatar":null,"bot":false}},"content":"!help","timestamp":"2026-01-01T00:00:00.000000+00:00","edited_timestamp":null,"tts":false,"mention_everyone":false,"mentions":[],"mention_roles":[],"attachments":[],"embeds":[],"pinned":false,"type":0}}}}"#
                                );
                                if let Some(tx) = inject_req.lock().await.as_ref()
                                    && tx.send(event).is_ok()
                                {
                                    last_send = Some(tokio::time::Instant::now());
                                }
                            }
                            tokio::time::sleep(Duration::from_millis(25)).await;
                        }
                        let after = help_req.load(Ordering::SeqCst);
                        if after > before {
                            (
                                "HTTP/1.1 200 OK",
                                format!(r#"{{"ok":true,"help_replies":{after}}}"#),
                            )
                        } else if !ready_req.load(Ordering::SeqCst) {
                            (
                                "HTTP/1.1 500 Internal Server Error",
                                r#"{"ok":false,"error":"bot_gateway_not_ready"}"#.to_owned(),
                            )
                        } else {
                            (
                                "HTTP/1.1 500 Internal Server Error",
                                format!(
                                    r#"{{"ok":false,"error":"help_reply_timeout","before":{before},"after":{after}}}"#
                                ),
                            )
                        }
                    } else {
                        ("HTTP/1.1 200 OK", "{}".to_owned())
                    };

                    let resp = format!(
                        "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        resp_body.len(),
                        resp_body
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                });
            }
        });

        Self {
            http_addr,
            ws_addr,
            http_task,
            ws_task,
        }
    }

    fn abort(self) {
        self.http_task.abort();
        self.ws_task.abort();
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn ensure_mcp_bin() -> PathBuf {
    static BUILT_MCP_BIN: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT_MCP_BIN
        .get_or_init(|| {
            let runtime_bin = PathBuf::from(env!("CARGO_BIN_EXE_cdr-runtime"));
            let candidate = runtime_bin.with_file_name("cdr-remote-agent.exe");
            let status = Command::new("cargo")
                .args([
                    "build",
                    "--locked",
                    "-p",
                    "cdr-remote-agent",
                    "--bin",
                    "cdr-remote-agent",
                ])
                .current_dir(repo_root())
                .status()
                .expect("build cdr-remote-agent");
            assert!(status.success(), "cargo build cdr-remote-agent failed");
            assert!(
                candidate.exists(),
                "cdr-remote-agent.exe missing at {}",
                candidate.display()
            );
            candidate
        })
        .clone()
}

fn run_pwsh(script: &str) {
    static SCRIPT_COUNTER: AtomicU64 = AtomicU64::new(1);
    let seq = SCRIPT_COUNTER.fetch_add(1, Ordering::SeqCst);
    let script_path =
        std::env::temp_dir().join(format!("cdr_contract_{}_{}.ps1", std::process::id(), seq));
    fs::write(&script_path, script).expect("write temp powershell script");
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
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &script_path.display().to_string(),
        ])
        .current_dir(repo_root())
        .output()
        .expect("execute powershell test script");
    let _ = fs::remove_file(&script_path);
    assert!(
        output.status.success(),
        "PowerShell contract failed (code {:?}):\nSTDOUT:\n{}\nSTDERR:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn t06_manual_stop_intent_revision_wins_over_watchdog_and_delayed_restart_helpers() {
    let ws = repo_root();
    let journal_script = ws.join("scripts").join("CdrLaunchJournal.ps1");
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
. '{journal}'

$temp = Join-Path ([IO.Path]::GetTempPath()) ('cdr_t06_' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $temp 'bot_data') -Force | Out-Null
$fakeBot = Join-Path $temp 'bin\win-x64\stub-bot.exe'
New-Item -ItemType Directory -Path (Split-Path -Parent $fakeBot) -Force | Out-Null
[IO.File]::WriteAllText($fakeBot, 'stub')
$psExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'

try {{
    # 0. Verify Invoke-CdrJournaledProcessLaunch does NOT swallow Get-CdrRuntimeArtifact errors on malformed cdr-runtime.exe
    $malformedBot = Join-Path $temp 'bin\win-x64\cdr-runtime.exe'
    [IO.File]::WriteAllText($malformedBot, 'malformed-not-a-valid-pe')
    Set-CdrComponentIntent -RepoRoot $temp -Component 'bot' -DesiredState 'running' -Action 'artifact-check' -Source 'test' | Out-Null
    $artifactRejected = $false
    try {{
        Invoke-CdrJournaledProcessLaunch -RepoRoot $temp -Component 'bot' -FilePath $malformedBot -TargetBinaryPath $malformedBot -Action 'artifact-check' | Out-Null
    }} catch {{
        $artifactRejected = $true
    }}
    if (-not $artifactRejected) {{
        throw 'Invoke-CdrJournaledProcessLaunch must fail closed when cdr-runtime.exe fails Get-CdrRuntimeArtifact validation!'
    }}
    Remove-Item -LiteralPath $malformedBot -Force

    # 1. Watchdog or delayed restart captures current running intent revision (rev 1)
    $startIntent = Set-CdrComponentIntent -RepoRoot $temp -Component 'bot' -DesiredState 'running' -Action 'watchdog-check' -Source 'watchdog'
    if ($startIntent.Revision -lt 1) {{ throw 'Expected revision >= 1' }}

    # 2. User issues manual stop right after watchdog check, before final spawn (rev 2)
    # Verify durable intent JSON is persisted BEFORE the stop marker is written!
    $script:intentJsonExistedBeforeMarker = $false
    $stopIntent = Set-CdrComponentIntent -RepoRoot $temp -Component 'bot' -DesiredState 'stopped' -Action 'stop' -Source 'user-manual-stop' -OnAfterIntentPersisted {{
        param($rec)
        $intentFile = Get-CdrComponentIntentPath -RepoRoot $temp -Component 'bot'
        $stopMarker = Join-Path $temp '.codex_discord_rust.stop'
        if ([IO.File]::Exists($intentFile) -and -not [IO.File]::Exists($stopMarker)) {{
            $diskRec = [IO.File]::ReadAllText($intentFile) | ConvertFrom-Json
            if ($diskRec.DesiredState -eq 'stopped' -and [uint64]$diskRec.Revision -eq $rec.Revision) {{
                $script:intentJsonExistedBeforeMarker = $true
            }}
        }}
    }}
    if (-not $script:intentJsonExistedBeforeMarker) {{
        throw 'Set-CdrComponentIntent must persist durable intent JSON before writing the stop marker!'
    }}
    if ($stopIntent.Revision -le $startIntent.Revision) {{
        throw "Stop revision ($($stopIntent.Revision)) did not increment past start revision ($($startIntent.Revision))"
    }}

    $script:spawnCount = 0
    function Start-CdrDetachedProcess {{
        $script:spawnCount++
        throw 'Start-CdrDetachedProcess must not be called when newer stop intent exists'
    }}

    # 3. Stale helper carrying ExpectedIntentRevision = $startIntent.Revision attempts to spawn -> must fail closed
    $blocked = $false
    try {{
        Invoke-CdrJournaledProcessLaunch `
            -RepoRoot $temp `
            -Component 'bot' `
            -FilePath $fakeBot `
            -TargetBinaryPath $fakeBot `
            -Action 'watchdog-start' `
            -ExpectedIntentRevision $startIntent.Revision | Out-Null
    }} catch {{
        $blocked = $true
    }}
    if (-not $blocked -or $script:spawnCount -ne 0) {{
        throw "Stale watchdog launch was not blocked (blocked=$blocked, spawnCount=$($script:spawnCount))"
    }}

    # 4. Delayed restart completion receipt with old revision cannot overwrite newer user stop intent
    $receiptBlocked = Complete-CdrComponentIntentIfCurrent `
        -RepoRoot $temp `
        -Component 'bot' `
        -ExpectedRevision $startIntent.Revision `
        -ResultState 'running' `
        -Source 'delayed-restart-helper'
    if ($receiptBlocked) {{
        throw 'Stale delayed restart receipt should not have overwritten newer user stop intent'
    }}

    $finalIntent = Get-CdrComponentIntent -RepoRoot $temp -Component 'bot'
    if ($finalIntent.DesiredState -ne 'stopped' -or $finalIntent.Revision -ne $stopIntent.Revision) {{
        throw "User stop intent was corrupted: $($finalIntent | ConvertTo-Json -Compress)"
    }}
    if (-not (Test-CdrManualStopRequested -RepoRoot $temp -Component 'bot')) {{
        throw 'Manual stop marker was unexpectedly cleared'
    }}

    # 5. Real competing asynchronous Stop controller test (matching Review-IntentSerialization.ps1):
    # Separate child PowerShell controller attempts Stop while Start-CdrDetachedProcess is in progress.
    # Because Enter-CdrIntentLock is held across launching -> Start-CdrDetachedProcess -> launched,
    # Stop cannot be accepted midway while Phase='launching' and ProcessIdentity=''.
    $raceRoot = Join-Path $temp 'race_sub'
    New-Item -ItemType Directory -Path (Join-Path $raceRoot 'bot_data') -Force | Out-Null
    $rev1 = Set-CdrComponentIntent -RepoRoot $raceRoot -Component 'bot' -DesiredState 'running' -Action 'start' -Source 'race-controller-a'
    $attemptPath = Join-Path $raceRoot 'stop-attempted'
    $acceptedPath = Join-Path $raceRoot 'stop-accepted.json'
    $stopChildScript = Join-Path $raceRoot 'stop_child.ps1'
    [IO.File]::WriteAllText($stopChildScript, @"
`$ErrorActionPreference = 'Stop'
. '$($('{journal}').Replace("'", "''"))'
[IO.File]::WriteAllText('$($attemptPath.Replace("'", "''"))', 'attempting')
`$s = Set-CdrComponentIntent -RepoRoot '$($raceRoot.Replace("'", "''"))' -Component 'bot' -DesiredState 'stopped' -Action 'stop' -Source 'race-controller-b'
`$j = Read-CdrComponentLaunchJournal -RepoRoot '$($raceRoot.Replace("'", "''"))' -Component 'bot'
[IO.File]::WriteAllText('$($acceptedPath.Replace("'", "''"))', (@{{
    Revision = `$s.Revision
    JournalPhaseAtAcceptance = if (`$j) {{ `$j.Phase }} else {{ `$null }}
    JournalRevisionAtAcceptance = if (`$j) {{ `$j.IntentRevision }} else {{ `$null }}
    ChildIdentityAtAcceptance = if (`$j) {{ `$j.ProcessIdentity }} else {{ `$null }}
}} | ConvertTo-Json -Compress))
"@)
    $script:acceptedInsideSpawn = $false
    $script:stopChildProc = $null
    function Start-CdrDetachedProcess {{
        param($FilePath, $ArgumentList, $WorkingDirectory, $WindowStyle)
        $script:stopChildProc = [Diagnostics.Process]::Start((New-Object Diagnostics.ProcessStartInfo -Property @{{
            FileName = $psExe
            Arguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$stopChildScript`""
            UseShellExecute = $false
            CreateNoWindow = $true
        }}))
        $attemptDeadline = [DateTime]::UtcNow.AddSeconds(4)
        while (-not [IO.File]::Exists($attemptPath) -and [DateTime]::UtcNow -lt $attemptDeadline) {{
            Start-Sleep -Milliseconds 20
        }}
        Start-Sleep -Milliseconds 250
        $script:acceptedInsideSpawn = [IO.File]::Exists($acceptedPath)
        return (Get-Process -Id $PID)
    }}
    $raceRes = Invoke-CdrJournaledProcessLaunch `
        -RepoRoot $raceRoot -Component 'bot' -FilePath $psExe -TargetBinaryPath $psExe `
        -Action 'race-start' -ExpectedIntentRevision $rev1.Revision
    if (-not $script:stopChildProc.WaitForExit(8000)) {{
        try {{ $script:stopChildProc.Kill() }} catch {{ }}
        throw 'Competing Stop process did not complete after launch lock was released!'
    }}
    if ($script:stopChildProc.ExitCode -ne 0) {{
        throw "Competing Stop process exited with code $($script:stopChildProc.ExitCode)"
    }}
    $accepted = [IO.File]::ReadAllText($acceptedPath) | ConvertFrom-Json
    if ($script:acceptedInsideSpawn -or
        $accepted.JournalPhaseAtAcceptance -ne 'launched' -or
        [uint64]$accepted.JournalRevisionAtAcceptance -ne $rev1.Revision -or
        [string]::IsNullOrWhiteSpace([string]$accepted.ChildIdentityAtAcceptance)) {{
        throw "Competing Stop controller serialization failure: acceptedInsideSpawn=$($script:acceptedInsideSpawn) accepted=$($accepted | ConvertTo-Json -Compress)"
    }}

    # 6. Stale Stop completion regression (matching Review-StaleStopCompletion.ps1):
    # Remove the mock Start-CdrDetachedProcess by re-dot-sourcing CdrLaunchJournal.ps1, launch real owned PowerShell child1,
    # inject a newer Start (revision 3) + child2 launch inside KillOverride after child1 exits, and verify old Stop does NOT overwrite journal!
    Remove-Item Function:\Start-CdrDetachedProcess -ErrorAction SilentlyContinue
    . '{journal}'
    $staleStopRoot = Join-Path $temp 'stale_stop_sub'
    New-Item -ItemType Directory -Path (Join-Path $staleStopRoot 'bot_data') -Force | Out-Null
    $child1 = $null; $child2 = $null
    try {{
        $ssRev1 = Set-CdrComponentIntent -RepoRoot $staleStopRoot -Component 'bot' -DesiredState 'running' -Action 'start' -Source 'stale-stop-test'
        $launch1 = Invoke-CdrJournaledProcessLaunch -RepoRoot $staleStopRoot -Component 'bot' -FilePath $psExe `
            -ArgumentList @('-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 30') `
            -WorkingDirectory $staleStopRoot -TargetBinaryPath $psExe -Action 'start' -ExpectedIntentRevision $ssRev1.Revision
        $child1 = $launch1.Process
        $script:launch2 = $null
        $stopRes = Stop-CdrComponentRuntime -RepoRoot $staleStopRoot -Component 'bot' -BinaryPath $psExe -GraceTimeoutMs 0 -Source 'stale-stop' -KillOverride {{
            param($live)
            Stop-CdrVerifiedProcess -ProcessId $live.ProcessId -ExpectedIdentity $live.ProcessIdentity -ExpectedBinaryPath $psExe -Process $live.Process | Out-Null
            $ssRev3 = Set-CdrComponentIntent -RepoRoot $staleStopRoot -Component 'bot' -DesiredState 'running' -Action 'start' -Source 'newer-start-after-old-exit'
            $script:launch2 = Invoke-CdrJournaledProcessLaunch -RepoRoot $staleStopRoot -Component 'bot' -FilePath $psExe `
                -ArgumentList @('-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 30') `
                -WorkingDirectory $staleStopRoot -TargetBinaryPath $psExe -Action 'start' -ExpectedIntentRevision $ssRev3.Revision
        }}
        $child2 = $script:launch2.Process
        $finalSsJournal = Read-CdrComponentLaunchJournal -RepoRoot $staleStopRoot -Component 'bot'
        $finalSsIntent = Get-CdrComponentIntent -RepoRoot $staleStopRoot -Component 'bot'
        if ($finalSsJournal.Phase -cne 'launched' -or
            $finalSsJournal.ProcessIdentity -cne $script:launch2.ProcessIdentity -or
            [uint64]$finalSsJournal.IntentRevision -ne [uint64]$script:launch2.IntentRevision -or
            $finalSsIntent.DesiredState -cne 'running' -or
            (Test-CdrManualStopRequested -RepoRoot $staleStopRoot -Component 'bot')) {{
            throw "Stale Stop overwrote newer Start journal or stop marker: journal=$($finalSsJournal | ConvertTo-Json -Compress) intent=$($finalSsIntent | ConvertTo-Json -Compress)"
        }}

        # 7. Superseded Force Stop regression (matching Review-SupersededForceStop.ps1):
        # Admit a newer running Start intent during an older Stop's grace wait (before force-kill commitment),
        # and verify the older Stop rechecks latest intent under the short boundary and makes 0 kill attempts!
        $script:supersededArmed = $true
        $script:supersededKillAttempts = 0
        $script:newerRunningIntent = $null
        function Start-Sleep {{
            param([int]$Milliseconds = 0, [int]$Seconds = 0)
            if ($script:supersededArmed -and $Milliseconds -eq 50) {{
                $script:supersededArmed = $false
                $script:newerRunningIntent = Set-CdrComponentIntent -RepoRoot $staleStopRoot -Component 'bot' -DesiredState 'running' -Action 'start' -Source 'newer-start-during-grace-wait'
            }}
            if ($Milliseconds -gt 0) {{ Microsoft.PowerShell.Utility\Start-Sleep -Milliseconds $Milliseconds }}
            elseif ($Seconds -gt 0) {{ Microsoft.PowerShell.Utility\Start-Sleep -Seconds $Seconds }}
        }}
        try {{
            $supStopRes = Stop-CdrComponentRuntime -RepoRoot $staleStopRoot -Component 'bot' -BinaryPath $psExe -GraceTimeoutMs 180 -Source 'older-stop-superseded' -KillOverride {{
                param($target)
                $script:supersededKillAttempts++
            }}
        }} finally {{
            Remove-Item Function:\Start-Sleep -ErrorAction SilentlyContinue
        }}
        $afterSupIntent = Get-CdrComponentIntent -RepoRoot $staleStopRoot -Component 'bot'
        if ($null -eq $script:newerRunningIntent -or
            $script:supersededKillAttempts -ne 0 -or
            $supStopRes.Stopped -or
            $supStopRes.Reason -cne 'superseded_by_newer_intent' -or
            $afterSupIntent.DesiredState -cne 'running' -or
            $child2.HasExited) {{
            throw "Superseded force-stop regression failed: killAttempts=$($script:supersededKillAttempts) stopRes=$($supStopRes | ConvertTo-Json -Compress) intent=$($afterSupIntent | ConvertTo-Json -Compress)"
        }}
    }} finally {{
        foreach ($c in @($child1, $child2)) {{
            if ($null -ne $c) {{
                $cid = Get-CdrProcessIdentity -Process $c
                if ($cid) {{ Stop-CdrVerifiedProcess -ProcessId $c.Id -ExpectedIdentity $cid -ExpectedBinaryPath $psExe -Process $c | Out-Null }}
            }}
        }}
    }}
}} finally {{
    $fullTemp = [IO.Path]::GetFullPath($temp)
    if ($fullTemp.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase)) {{
        Remove-Item -LiteralPath $fullTemp -Recurse -Force -ErrorAction SilentlyContinue
    }}
}}
"#,
        journal = journal_script.display()
    );
    run_pwsh(&script);
}

#[test]
fn t07_concurrent_control_serialization_pid_reuse_rejection_and_unknown_spawn_hold_for_bot_and_mcp()
{
    let ws = repo_root();
    let journal_script = ws.join("scripts").join("CdrLaunchJournal.ps1");
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
. '{journal}'

$temp = Join-Path ([IO.Path]::GetTempPath()) ('cdr_t07_' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $temp 'bot_data') -Force | Out-Null
$fakeBot = Join-Path $temp 'bin\win-x64\stub-bot.exe'
$fakeMcp = Join-Path $temp 'bin\win-x64\stub-mcp.exe'
New-Item -ItemType Directory -Path (Split-Path -Parent $fakeBot) -Force | Out-Null
[IO.File]::WriteAllText($fakeBot, 'stub-bot')
[IO.File]::WriteAllText($fakeMcp, 'stub-mcp')
$psExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'

try {{
    # 1. PID reuse rejection & path mismatch classification & status-only no-kill-authority & retained-handle WaitForExit
    $reusedIdentity = "$PID|1234567890"
    $reusedObs = Get-CdrProcessObservation -ProcessId $PID -ExpectedIdentity $reusedIdentity
    if ($reusedObs.Matches -or $reusedObs.State -ne 'stopped' -or $reusedObs.Reason -ne 'identity_reused') {{
        throw "PID reuse with mismatched creation ticks must be State=stopped/identity_reused: $($reusedObs | ConvertTo-Json -Compress)"
    }}
    $realIdentity = Get-CdrProcessIdentity -Process (Get-Process -Id $PID)
    if (-not (Test-CdrProcessMatchesIdentity -ProcessId $PID -ExpectedIdentity $realIdentity -ExpectedBinaryPath $psExe)) {{
        throw 'Exact live process identity + path should match'
    }}
    $mismatchObs = Get-CdrProcessObservation -ProcessId $PID -ExpectedIdentity $realIdentity -ExpectedBinaryPath $fakeBot
    if ($mismatchObs.Matches -or $mismatchObs.State -ne 'unknown' -or $mismatchObs.Reason -ne 'path_mismatch') {{
        throw "Live process with matching identity but different image path must be State=unknown/path_mismatch: $($mismatchObs | ConvertTo-Json -Compress)"
    }}

    # 1b. Status-only observation must NEVER grant force-termination authority!
    $statusAuthRoot = Join-Path $temp 'status_authority_sub'
    New-Item -ItemType Directory -Path (Join-Path $statusAuthRoot 'bot_data') -Force | Out-Null
    Write-CdrAtomicJsonFile -Path (Join-Path $statusAuthRoot 'bot_data\remote_mcp_status.json') -Object ([ordered]@{{
        pid = [int]$PID; process_identity = $realIdentity; executable_path = $psExe
    }})
    $statusOnlyObs = Get-CdrComponentObservation -RepoRoot $statusAuthRoot -Component 'mcp' -BinaryPath $psExe
    if ($statusOnlyObs.State -ne 'running' -or $statusOnlyObs.Reason -ne 'matched_status' -or $statusOnlyObs.HasControlAuthority) {{
        throw "Status-only observation must have HasControlAuthority=false: $($statusOnlyObs | ConvertTo-Json -Compress)"
    }}
    $script:statusKillAttempts = 0
    $statusStopRes = Stop-CdrComponentRuntime -RepoRoot $statusAuthRoot -Component 'mcp' -BinaryPath $psExe -GraceTimeoutMs 0 -AllowForceKill -KillOverride {{
        $script:statusKillAttempts++
        return $false
    }}
    if ($script:statusKillAttempts -ne 0 -or $statusStopRes.Stopped -or $statusStopRes.Reason -ne 'status_only_no_kill_authority') {{
        throw "Status-only observation must refuse force kill (attempts=$($script:statusKillAttempts)): $($statusStopRes | ConvertTo-Json -Compress)"
    }}

    # 1c. Retained-handle termination in Stop-CdrVerifiedProcess waits for actual Process exit
    $ownedVictim = Start-Process -FilePath $psExe -ArgumentList @('-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 30') -WindowStyle Hidden -PassThru
    $victimIdentity = Get-CdrProcessIdentity -Process $ownedVictim
    $killedOk = Stop-CdrVerifiedProcess -ProcessId $ownedVictim.Id -ExpectedIdentity $victimIdentity -ExpectedBinaryPath $psExe -WaitTimeoutMs 4000
    if (-not $killedOk -or -not $ownedVictim.HasExited) {{
        try {{ $ownedVictim.Kill() }} catch {{ }}
        throw "Stop-CdrVerifiedProcess failed to terminate and wait for owned process handle (killedOk=$killedOk)"
    }}

    # 1d. Legacy bot lock authority: stale startup.process_identity or PID-only .codex_discord_rust.runtime.lock
    # must NEVER grant HasControlAuthority=true or allow force-termination!
    $legacyLockRoot = Join-Path $temp 'legacy_bot_lock_auth'
    New-Item -ItemType Directory -Path (Join-Path $legacyLockRoot 'bot_data') -Force | Out-Null
    $staleStartup = @{{
        schema_version = 1
        process_identity = "$PID|1"
        executable_path = $psExe
        artifact_sha256 = ('0' * 64)
        build_info = @{{ schema_version = 1; package_version = '0.1.0'; source_state = 'clean' }}
    }}
    $staleLockText = "pid=$PID`nstartup=$($staleStartup | ConvertTo-Json -Compress -Depth 5)`n"
    [IO.File]::WriteAllText((Join-Path $legacyLockRoot '.codex_discord_rust.runtime.lock'), $staleLockText, [Text.UTF8Encoding]::new($false))
    $staleLockObs = Get-CdrComponentObservation -RepoRoot $legacyLockRoot -Component 'bot' -BinaryPath $psExe
    if ($staleLockObs.State -ne 'running' -or $staleLockObs.HasControlAuthority -or $staleLockObs.Reason -cne 'lock_startup_identity_mismatch') {{
        throw "Stale startup.process_identity in bot lock must set HasControlAuthority=false: $($staleLockObs | ConvertTo-Json -Compress)"
    }}
    $script:botLockKillAttempts = 0
    $staleLockStop = Stop-CdrComponentRuntime -RepoRoot $legacyLockRoot -Component 'bot' -BinaryPath $psExe -GraceTimeoutMs 0 -AllowForceKill -KillOverride {{
        $script:botLockKillAttempts++
        return $false
    }}
    if ($script:botLockKillAttempts -ne 0 -or $staleLockStop.Stopped -or $staleLockStop.Reason -cne 'status_only_no_kill_authority') {{
        throw "Stale startup.process_identity in bot lock must refuse force termination (attempts=$($script:botLockKillAttempts)): $($staleLockStop | ConvertTo-Json -Compress)"
    }}
    # Also verify PID-only legacy lock without startup= or identity= sets HasControlAuthority=false
    [IO.File]::WriteAllText((Join-Path $legacyLockRoot '.codex_discord_rust.runtime.lock'), "pid=$PID`n", [Text.UTF8Encoding]::new($false))
    $pidOnlyObs = Get-CdrComponentObservation -RepoRoot $legacyLockRoot -Component 'bot' -BinaryPath $psExe
    if ($pidOnlyObs.State -ne 'running' -or $pidOnlyObs.HasControlAuthority -or $pidOnlyObs.Reason -cne 'matched_legacy_lock_pid_only') {{
        throw "PID-only legacy bot lock must set HasControlAuthority=false: $($pidOnlyObs | ConvertTo-Json -Compress)"
    }}

    # 1e. Corrupt legacy launch journal (.codex_discord_rust.restart.launch) must hold with launch_outcome_unknown, not be swallowed as absence!
    $corruptLegacyRoot = Join-Path $temp 'corrupt_legacy_journal'
    New-Item -ItemType Directory -Path (Join-Path $corruptLegacyRoot 'bot_data') -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $corruptLegacyRoot '.codex_discord_rust.restart.launch'), '{{corrupt-not-json', [Text.UTF8Encoding]::new($false))
    Set-CdrComponentIntent -RepoRoot $corruptLegacyRoot -Component 'bot' -DesiredState 'running' -Action 'start' -Source 'test' | Out-Null
    $corruptLegacyHeld = $false
    try {{
        Invoke-CdrJournaledProcessLaunch -RepoRoot $corruptLegacyRoot -Component 'bot' -FilePath $fakeBot -TargetBinaryPath $fakeBot -Action 'start' | Out-Null
    }} catch {{
        if ($_.Exception.Message -like 'launch_outcome_unknown*') {{ $corruptLegacyHeld = $true }} else {{ throw }}
    }}
    if (-not $corruptLegacyHeld) {{
        throw 'Corrupt .codex_discord_rust.restart.launch must hold with launch_outcome_unknown!'
    }}

    # 2. Uncertain spawn outcome for MCP transitions to launch_outcome_unknown hold and blocks duplicate launch
    Set-CdrComponentIntent -RepoRoot $temp -Component 'mcp' -DesiredState 'running' -Action 'start' -Source 'test' | Out-Null
    $script:mcpSpawnAttempts = 0
    function Start-CdrDetachedProcess {{
        param($FilePath, $ArgumentList, $WorkingDirectory, $WindowStyle)
        $script:mcpSpawnAttempts++
        throw 'SIMULATED_MCP_SPAWN_UNCERTAINTY'
    }}

    $firstFailed = $false
    try {{
        Invoke-CdrJournaledProcessLaunch `
            -RepoRoot $temp `
            -Component 'mcp' `
            -FilePath $fakeMcp `
            -TargetBinaryPath $fakeMcp `
            -Action 'mcp-start' | Out-Null
    }} catch {{
        $firstFailed = $true
    }}
    if (-not $firstFailed -or $script:mcpSpawnAttempts -ne 1) {{
        throw 'Expected first uncertain MCP spawn to throw after 1 attempt'
    }}

    # Second launch attempt must be blocked by launch_outcome_unknown BEFORE calling Start-CdrDetachedProcess again
    $secondBlocked = $false
    try {{
        Invoke-CdrJournaledProcessLaunch `
            -RepoRoot $temp `
            -Component 'mcp' `
            -FilePath $fakeMcp `
            -TargetBinaryPath $fakeMcp `
            -Action 'mcp-retry' | Out-Null
    }} catch {{
        if ($_.Exception.Message -match 'launch_outcome_unknown') {{
            $secondBlocked = $true
        }} else {{
            throw
        }}
    }}
    if (-not $secondBlocked -or $script:mcpSpawnAttempts -ne 1) {{
        throw "Duplicate MCP spawn was not held by launch_outcome_unknown (attempts=$($script:mcpSpawnAttempts))"
    }}

    # 3. Exit-All atomic decision persistence before signaling either runtime, plus interruption recovery
    $interruptedAfterFirst = $false
    try {{
        Set-CdrExitAllIntent -RepoRoot $temp -Source 'interrupted-exit-all' -OnAfterFirstIntentPersisted {{
            param($botRec)
            # At this boundary, bot-intent.json is written, mcp-intent.json is not yet updated,
            # and NEITHER stop marker may exist yet!
            if ([IO.File]::Exists((Join-Path $temp '.codex_discord_rust.stop')) -or
                [IO.File]::Exists((Join-Path $temp 'bot_data\remote_mcp.stop'))) {{
                throw 'Stop marker was written before both Exit-All intents were durable!'
            }}
            throw 'SIMULATED_EXIT_ALL_CRASH_BETWEEN_INTENTS'
        }} | Out-Null
    }} catch {{
        if ($_.Exception.Message -match 'SIMULATED_EXIT_ALL_CRASH_BETWEEN_INTENTS') {{
            $interruptedAfterFirst = $true
        }} else {{
            throw
        }}
    }}
    if (-not $interruptedAfterFirst) {{ throw 'Expected simulated interruption during Exit-All' }}

    # Reading either component intent must recover the pending atomic Exit-All decision so BOTH are stopped!
    $botRecovered = Get-CdrComponentIntent -RepoRoot $temp -Component 'bot'
    $mcpRecovered = Get-CdrComponentIntent -RepoRoot $temp -Component 'mcp'
    if ($botRecovered.DesiredState -ne 'stopped' -or $mcpRecovered.DesiredState -ne 'stopped') {{
        throw "Interrupted Exit-All did not recover both stopped intents: bot=$($botRecovered | ConvertTo-Json -Compress) mcp=$($mcpRecovered | ConvertTo-Json -Compress)"
    }}
    if (-not (Test-CdrManualStopRequested -RepoRoot $temp -Component 'bot') -or -not (Test-CdrManualStopRequested -RepoRoot $temp -Component 'mcp')) {{
        throw 'Recovered Exit-All must ensure manual stop markers for both bot and mcp'
    }}

    # 4. Concurrent multi-process intent writers serialize cleanly without revision collision
    $concRoot = Join-Path $temp 'concurrent_writers'
    New-Item -ItemType Directory -Path (Join-Path $concRoot 'bot_data') -Force | Out-Null
    $workerScript = Join-Path $concRoot 'worker.ps1'
    [IO.File]::WriteAllText($workerScript, @"
param([string]`$WorkerName)
`$ErrorActionPreference = 'Stop'
. '$($('{journal}').Replace("'", "''"))'
for (`$i = 1; `$i -le 5; `$i++) {{
    `$state = if (`$i % 2 -eq 0) {{ 'stopped' }} else {{ 'running' }}
    Set-CdrComponentIntent -RepoRoot '$($concRoot.Replace("'", "''"))' -Component 'bot' -DesiredState `$state -Action "step-`$i" -Source `$WorkerName | Out-Null
}}
"@)
    $p1 = Start-Process -FilePath $psExe -ArgumentList @('-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',$workerScript,'w1') -WindowStyle Hidden -PassThru
    $p2 = Start-Process -FilePath $psExe -ArgumentList @('-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',$workerScript,'w2') -WindowStyle Hidden -PassThru
    $p1.WaitForExit(10000) | Out-Null
    $p2.WaitForExit(10000) | Out-Null
    if ($p1.ExitCode -ne 0 -or $p2.ExitCode -ne 0) {{
        throw "Concurrent intent workers failed: exit1=$($p1.ExitCode) exit2=$($p2.ExitCode)"
    }}
    $concFinal = Get-CdrComponentIntent -RepoRoot $concRoot -Component 'bot'
    if ($concFinal.Revision -ne 10) {{
        throw "Expected 2x5 concurrent intent updates to reach exact Revision 10 without lost updates, got $($concFinal.Revision)"
    }}

    # 5. Cross-entrypoint simultaneous duplicate-start exclusion (Invoke-CdrAppLaunch vs Start-RustRuntime)
    # and Start-RustRuntime stdout/stderr log redirection verification
    $dupRoot = Join-Path $temp 'cross_entrypoint_race'
    New-Item -ItemType Directory -Path (Join-Path $dupRoot 'bot_data') -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $dupRoot '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`n")
    $dupBotBin = Join-Path $dupRoot 'bin\win-x64\cdr-runtime.exe'
    New-Item -ItemType Directory -Path (Split-Path -Parent $dupBotBin) -Force | Out-Null
    [IO.File]::WriteAllText($dupBotBin, 'stub-bot-binary')

    # Verify Start-RustRuntime passes RedirectStandardOutput/RedirectStandardError to Start-CdrDetachedProcess
    # and includes $StderrLog details when a startup child exits early
    $wdAst = [System.Management.Automation.Language.Parser]::ParseFile((Join-Path (Split-Path -Parent '{journal}') '..\codex-discord-rust-watchdog.ps1'), [ref]$null, [ref]$null)
    foreach ($fnName in @('Get-RustProcessIdentity', 'Start-RustRuntime')) {{
        $fnNode = $wdAst.FindAll({{ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -ceq $fnName }}, $true) | Select-Object -First 1
        . ([scriptblock]::Create($fnNode.Extent.Text))
    }}
    $RepoRoot = $dupRoot
    $BinaryPath = $dupBotBin
    $EnvPath = Join-Path $dupRoot '.env'
    $StdoutLog = Join-Path $dupRoot 'codex_discord_rust.log'
    $StderrLog = Join-Path $dupRoot 'codex_discord_rust.error.log'
    $ExpectedBinaryHash = ''; $ExpectedSourceCommit = ''; $script:CdrLaunchJournalPath = $null
    function Get-CdrRuntimeArtifact($Path, $ExpectedBinaryHash, $ExpectedSourceCommit) {{
        return [pscustomobject]@{{ ArtifactHash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash; BuildInfo = $null; RequireStartupRecord = $false }}
    }}
    function Get-CdrRuntimeArtifactPin($Path, $ExpectedHash) {{
        $p = [pscustomobject]@{{}}; $p | Add-Member ScriptMethod Dispose {{}}; return $p
    }}
    function Invoke-CdrProductionCheckedLaunch($DeadlineUtc, [scriptblock]$Launch) {{ & $Launch }}
    function Get-VerifiedRuntimeProcess {{ return $null }}
    function Write-RustWatchdogLog([string]$Message) {{ }}

    $script:capturedRedirectErr = ''
    function Start-CdrDetachedProcess {{
        param($FilePath, $ArgumentList, $WorkingDirectory, $WindowStyle, $RedirectStandardOutput, $RedirectStandardError)
        $script:capturedRedirectErr = [string]$RedirectStandardError
        if ($RedirectStandardError) {{
            [IO.File]::WriteAllText($RedirectStandardError, 'FATAL_CONFIG_PARSE_FAILURE_ON_STDERR')
        }}
        $exitedProc = [pscustomobject]@{{
            Id = 54321
            Path = $FilePath
            StartTime = [DateTime]::UtcNow
            HasExited = $true
            ExitCode = 9
        }}
        return $exitedProc
    }}
    $stderrSurfaced = $false
    try {{
        Start-RustRuntime
    }} catch {{
        if ($_.Exception.Message -match 'FATAL_CONFIG_PARSE_FAILURE_ON_STDERR') {{
            $stderrSurfaced = $true
        }} else {{
            throw
        }}
    }}
    if (-not $stderrSurfaced -or $script:capturedRedirectErr -cne $StderrLog) {{
        throw "Start-RustRuntime did not redirect or surface StderrLog ($($script:capturedRedirectErr))"
    }}
    Clear-CdrComponentLaunchJournal -RepoRoot $dupRoot -Component 'bot'

    # Now test simultaneous Start-RustRuntime and Invoke-CdrAppLaunch:
    # While Start-RustRuntime is inside Start-CdrDetachedProcess (holding Enter-CdrIntentLock),
    # a competing child process runs Invoke-CdrAppLaunch against the same $dupRoot.
    # Only 1 bot spawn may occur!
    $competingScript = Join-Path $dupRoot 'competing_app_launch.ps1'
    $competingAttempt = Join-Path $dupRoot 'competing_attempt.flag'
    $competingResult = Join-Path $dupRoot 'competing_result.json'
    [IO.File]::WriteAllText($competingScript, @"
`$ErrorActionPreference = 'Stop'
. '$($('{journal}').Replace("'", "''"))'
[IO.File]::WriteAllText('$($competingAttempt.Replace("'", "''"))', '1')
function Start-CdrDetachedProcess {{
    throw 'COMPETING_APP_LAUNCH_MUST_NOT_SPAWN_DUPLICATE_BOT'
}}
`$res = Invoke-CdrAppLaunch -RepoRoot '$($dupRoot.Replace("'", "''"))' -BotBinaryPath '$($psExe.Replace("'", "''"))' -SkipTray
[IO.File]::WriteAllText('$($competingResult.Replace("'", "''"))', (`$res | ConvertTo-Json -Compress))
"@)
    $BinaryPath = $psExe
    $script:botSpawns = 0
    $script:competingChild = $null
    $liveSelf = Get-Process -Id $PID -ErrorAction Stop
    function Get-VerifiedRuntimeProcess {{ return $liveSelf }}
    function Wait-CdrReplacementReady($ExpectedIdentity, $Artifact, $TimeoutSeconds) {{ }}
    function Start-CdrDetachedProcess {{
        param($FilePath, $ArgumentList, $WorkingDirectory, $WindowStyle, $RedirectStandardOutput, $RedirectStandardError)
        $script:botSpawns++
        $script:competingChild = [Diagnostics.Process]::Start((New-Object Diagnostics.ProcessStartInfo -Property @{{
            FileName = $psExe
            Arguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$competingScript`""
            UseShellExecute = $false
            CreateNoWindow = $true
        }}))
        $attDeadline = [DateTime]::UtcNow.AddSeconds(4)
        while (-not [IO.File]::Exists($competingAttempt) -and [DateTime]::UtcNow -lt $attDeadline) {{
            Start-Sleep -Milliseconds 20
        }}
        Start-Sleep -Milliseconds 200
        return $liveSelf
    }}
    Start-RustRuntime
    if (-not $script:competingChild.WaitForExit(8000)) {{
        try {{ $script:competingChild.Kill() }} catch {{ }}
        throw 'Competing Invoke-CdrAppLaunch process timed out waiting for lock'
    }}
    if ($script:competingChild.ExitCode -ne 0) {{
        throw "Competing Invoke-CdrAppLaunch process failed with exit code $($script:competingChild.ExitCode)"
    }}
    $compRes = [IO.File]::ReadAllText($competingResult) | ConvertFrom-Json
    if ($script:botSpawns -ne 1 -or $compRes.BotState -ne 'already_running' -or [int]$compRes.BotProcessId -ne $PID) {{
        throw "Cross-entrypoint duplicate-start race failed: botSpawns=$($script:botSpawns) compRes=$($compRes | ConvertTo-Json -Compress)"
    }}
}} finally {{
    $fullTemp = [IO.Path]::GetFullPath($temp)
    if ($fullTemp.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase)) {{
        Remove-Item -LiteralPath $fullTemp -Recurse -Force -ErrorAction SilentlyContinue
    }}
}}
"#,
        journal = journal_script.display()
    );
    run_pwsh(&script);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t01_bot_stop_start_and_restart_preserve_external_mcp_pid_identity_and_epoch() {
    let ws = repo_root();
    let mcp_bin = ensure_mcp_bin();
    let journal_script = ws.join("scripts").join("CdrLaunchJournal.ps1");

    let temp = tempfile::tempdir().expect("tempdir");
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).expect("create project");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind bridge");
    let bridge_addr = listener.local_addr().expect("bridge addr");

    let (cmd_req_tx, mut cmd_req_rx) =
        tokio::sync::mpsc::unbounded_channel::<(String, tokio::sync::oneshot::Sender<String>)>();
    let project_for_bridge = project_dir.clone();
    let bridge_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept mcp ws");
        let mut ws = accept_async(stream).await.expect("ws handshake");
        let hello = ws.next().await.expect("hello").expect("hello ok");
        parse_bridge_message(hello.to_text().expect("text")).expect("valid hello");
        ws.send(Message::Text(
            serde_json::to_string(&GatewayInboundMessage::HelloAck {
                protocol_version: ProtocolV10,
            })
            .unwrap()
            .into(),
        ))
        .await
        .expect("send hello_ack");
        ws.send(Message::Text(
            serde_json::to_string(&GatewayInboundMessage::Command(
                GatewayCommand::DeviceSession {
                    request_id: "bind-t01".into(),
                    thread_id: "thread-t01".into(),
                    deadline_at: Utc::now() + ChronoDuration::minutes(5),
                    computer_session_id: "session-t01-valid-01".into(),
                    computer_session_generation: 1,
                    working_directory: project_for_bridge.display().to_string(),
                    expires_at: Utc::now() + ChronoDuration::minutes(10),
                },
            ))
            .unwrap()
            .into(),
        ))
        .await
        .expect("send device session");
        let _bind_ack = ws.next().await.expect("bind ack").expect("bind ack ok");

        loop {
            tokio::select! {
                cmd = cmd_req_rx.recv() => {
                    let Some((marker, reply_tx)) = cmd else { break; };
                    let req_id = format!("op-{marker}");
                    ws.send(Message::Text(
                        serde_json::to_string(&GatewayInboundMessage::Command(
                            GatewayCommand::ProjectOperation {
                                request_id: req_id,
                                thread_id: "thread-t01".into(),
                                deadline_at: Utc::now() + ChronoDuration::minutes(2),
                                computer_session_id: Some("session-t01-valid-01".into()),
                                operation: ProjectOperation::Terminal(TerminalRequest::TerminalExec {
                                    terminal_id: None,
                                    shell: TerminalShell::Powershell,
                                    command: format!("Write-Output 'MCP_EXEC_OK:{marker}'"),
                                    cwd: None,
                                    environment: std::collections::BTreeMap::new(),
                                    timeout_seconds: 10,
                                    cancel_previous: false,
                                }),
                            },
                        ))
                        .unwrap()
                        .into(),
                    ))
                    .await
                    .expect("send terminal op");
                    while let Some(Ok(msg)) = ws.next().await {
                        if let Message::Ping(payload) = msg {
                            let _ = ws.send(Message::Pong(payload)).await;
                        } else if msg.is_text() {
                            let _ = reply_tx.send(msg.to_text().unwrap_or("").to_owned());
                            break;
                        }
                    }
                }
                ws_msg = ws.next() => {
                    match ws_msg {
                        Some(Ok(Message::Ping(payload))) => {
                            let _ = ws.send(Message::Pong(payload)).await;
                        }
                        None | Some(Err(_) | Ok(Message::Close(_))) => break,
                        _ => {}
                    }
                }
            }
        }
    });

    // Step 1: Launch real external MCP + cooperative bot via Invoke-CdrAppLaunch, verify bot stop/start/force-restart
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
. '{journal}'

$temp = '{temp}'
New-Item -ItemType Directory -Path (Join-Path $temp 'bot_data') -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_OWNERSHIP=external`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://{bridge_addr}`nCODEX_REMOTE_MCP_DEVICE_ID=device-t01`nCODEX_REMOTE_MCP_DEVICE_TOKEN=secret-t01`n")
Write-CdrAtomicJsonFile -Path (Join-Path $temp '.cdr-maintenance\mcp-ownership-state.json') -Object ([ordered]@{{
    schema_version = 1; mode = 'external'; state = 'committed'; committed = $true; updated_at_ms = 1
}})

$psExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
New-Item -ItemType Directory -Path (Join-Path $temp 'target\release') -Force | Out-Null
$mcpBin = Join-Path $temp 'target\release\cdr-remote-agent.exe'
Copy-Item -LiteralPath '{mcp_bin}' -Destination $mcpBin -Force
$botScript = Join-Path $temp 'bot_coop.ps1'
[IO.File]::WriteAllText($botScript, @"
`$stopFile = Join-Path '$($temp.Replace("'", "''"))' '.codex_discord_rust.stop'
while (-not [IO.File]::Exists(`$stopFile)) {{ Start-Sleep -Milliseconds 30 }}
"@)

# First verify that even if Bot binary is missing/fails during Invoke-CdrAppLaunch, External MCP still starts and becomes ready!
$appInitial = Invoke-CdrAppLaunch `
    -RepoRoot $temp `
    -BotBinaryPath (Join-Path $temp 'nonexistent-bot.exe') `
    -McpBinaryPath $mcpBin `
    -SkipTray
if ($appInitial.McpState -ne 'started' -or $appInitial.BotState -ne 'binary_missing') {{
    throw "Expected MCP to start even when bot binary is missing, got: $($appInitial | ConvertTo-Json -Compress)"
}}

# Wait for real cdr-remote-agent.exe to publish ready=true, hello_ack=true, generation=1
$statusPath = Join-Path $temp 'bot_data\remote_mcp_status.json'
$deadline = [DateTime]::UtcNow.AddSeconds(8)
$statusBefore = $null
while ([DateTime]::UtcNow -lt $deadline) {{
    if ([IO.File]::Exists($statusPath)) {{
        try {{
            $s = [IO.File]::ReadAllText($statusPath) | ConvertFrom-Json
            if ($s.ready -and $s.hello_ack -and [int]$s.generation -eq 1) {{
                $statusBefore = $s
                break
            }}
        }} catch {{ }}
    }}
    Start-Sleep -Milliseconds 50
}}
if ($null -eq $statusBefore) {{ throw 'Real external MCP did not become ready within timeout' }}

# Verify positive SessionId and UserSid match on the detached external MCP process
$meSec = Get-CdrProcessSecurityContext -ProcessId $PID
$mcpSec = Get-CdrProcessSecurityContext -ProcessId $appInitial.McpProcessId
if ($meSec.SessionId -ne $mcpSec.SessionId -or $meSec.UserSid -cne $mcpSec.UserSid) {{
    throw "MCP security context mismatch: meSession=$($meSec.SessionId) mcpSession=$($mcpSec.SessionId)"
}}

# Now start Bot 1, stop it gracefully, start Bot 2, and force-restart Bot 2 -> Bot 3
$bot1 = Invoke-CdrJournaledProcessLaunch -RepoRoot $temp -Component 'bot' -FilePath $psExe `
    -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',$botScript) `
    -WorkingDirectory $temp -TargetBinaryPath $psExe -Action 'start-bot-1' -AllowClearManualStop
$stop1 = Stop-CdrComponentRuntime -RepoRoot $temp -Component 'bot' -BinaryPath $psExe -GraceTimeoutMs 2000 -Source 'user-stop-bot'
if (-not $stop1.Stopped) {{ throw 'Bot 1 did not stop gracefully' }}

$bot2 = Invoke-CdrJournaledProcessLaunch -RepoRoot $temp -Component 'bot' -FilePath $psExe `
    -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',$botScript) `
    -WorkingDirectory $temp -TargetBinaryPath $psExe -Action 'start-bot-2' -AllowClearManualStop
if ($bot2.ProcessIdentity -ceq $bot1.ProcessIdentity) {{ throw 'Bot 2 must have new identity' }}

# Force-stop Bot 2 and verify External MCP is STILL untouched at generation 1
$stop2 = Stop-CdrComponentRuntime -RepoRoot $temp -Component 'bot' -BinaryPath $psExe -GraceTimeoutMs 500 -Source 'force-restart-bot' -AllowForceKill
if (-not $stop2.Stopped) {{ throw 'Bot 2 did not stop on force-restart' }}

$statusAfter = [IO.File]::ReadAllText($statusPath) | ConvertFrom-Json
if ([int]$statusAfter.pid -ne $appInitial.McpProcessId -or
    [string]$statusAfter.process_identity -cne $appInitial.McpIdentity -or
    [int]$statusAfter.generation -ne 1 -or -not $statusAfter.ready) {{
    throw "External MCP status/epoch changed across bot stop/start/restart: $($statusAfter | ConvertTo-Json -Compress)"
}}

# Verify Invoke-CdrAppLaunch recorded the default 64-hex ArtifactHash and BuildInfo in mcp-launch-journal.json without explicit ExpectedBinaryHash
$mcpJournalRec = Read-CdrComponentLaunchJournal -RepoRoot $temp -Component 'mcp'
if ($null -eq $mcpJournalRec -or $mcpJournalRec.ArtifactHash -notmatch '^[A-F0-9]{{64}}$' -or $null -eq $mcpJournalRec.BuildInfo) {{
    throw "Expected Invoke-CdrAppLaunch to record default ArtifactHash and BuildInfo in mcp-launch-journal.json, got: $($mcpJournalRec | ConvertTo-Json -Compress)"
}}

# Also verify normal no-argument codex-discord-bot.cmd routes through Invoke-CdrAppLaunch
New-Item -ItemType Directory -Path (Join-Path $temp 'scripts') -Force | Out-Null
Copy-Item -LiteralPath (Join-Path (Split-Path -Parent '{journal}') '..\codex-discord-bot.cmd') -Destination (Join-Path $temp 'codex-discord-bot.cmd') -Force
foreach ($sf in Get-ChildItem -LiteralPath (Split-Path -Parent '{journal}') -File) {{
    Copy-Item -LiteralPath $sf.FullName -Destination (Join-Path $temp ('scripts\' + $sf.Name)) -Force
}}
$cmdOut = & cmd.exe /c "`"$(Join-Path $temp 'codex-discord-bot.cmd')`"" 2>&1 | Out-String
if ($LASTEXITCODE -ne 0 -or $cmdOut -notmatch '"McpState"\s*:\s*"already_running"' -or $cmdOut -notmatch '"BotState"\s*:\s*"stopped_by_intent"') {{
    throw "Normal no-argument codex-discord-bot.cmd did not route through Invoke-CdrAppLaunch: exit=$LASTEXITCODE out=$cmdOut"
}}

# Also verify codex-discord-bot-headless.vbs and codex-discord-bot.cmd in an installation path containing both spaces AND apostrophes
$spaceQuoteRoot = Join-Path $temp "sub path with spaces and O'Brien"
New-Item -ItemType Directory -Path (Join-Path $spaceQuoteRoot 'scripts') -Force | Out-Null
Copy-Item -LiteralPath (Join-Path (Split-Path -Parent '{journal}') '..\codex-discord-bot-headless.vbs') -Destination (Join-Path $spaceQuoteRoot 'codex-discord-bot-headless.vbs') -Force
Copy-Item -LiteralPath (Join-Path (Split-Path -Parent '{journal}') '..\codex-discord-bot.cmd') -Destination (Join-Path $spaceQuoteRoot 'codex-discord-bot.cmd') -Force
$spaceMarker = Join-Path $spaceQuoteRoot 'called.txt'
[IO.File]::WriteAllText((Join-Path $spaceQuoteRoot 'scripts\CdrLaunchJournal.ps1'), @"
function Invoke-CdrAppLaunch {{
    param([string]`$RepoRoot, [string]`$BotBinaryPath)
    [IO.File]::WriteAllText((Join-Path `$RepoRoot 'called.txt'), "reached:`$RepoRoot")
    return [pscustomobject]@{{ Reopened = `$false; BotState = 'fixture_ok'; McpState = 'fixture_ok' }}
}}
"@)
& wscript.exe //B //Nologo (Join-Path $spaceQuoteRoot 'codex-discord-bot-headless.vbs')
$vbsDeadline = [DateTime]::UtcNow.AddSeconds(6)
while (-not [IO.File]::Exists($spaceMarker) -and [DateTime]::UtcNow -lt $vbsDeadline) {{
    Start-Sleep -Milliseconds 50
}}
if (-not [IO.File]::Exists($spaceMarker)) {{
    throw "codex-discord-bot-headless.vbs failed in installation path with spaces and apostrophes: $spaceQuoteRoot"
}}
Remove-Item -LiteralPath $spaceMarker -Force
$spaceCmdOut = & cmd.exe /c "`"$(Join-Path $spaceQuoteRoot 'codex-discord-bot.cmd')`"" 2>&1 | Out-String
if ($LASTEXITCODE -ne 0 -or -not [IO.File]::Exists($spaceMarker) -or $spaceCmdOut -notmatch 'fixture_ok') {{
    throw "codex-discord-bot.cmd failed in installation path with spaces and apostrophes: exit=$LASTEXITCODE out=$spaceCmdOut"
}}
"#,
        journal = journal_script.display(),
        temp = temp.path().display(),
        bridge_addr = bridge_addr,
        mcp_bin = mcp_bin.display()
    );
    run_pwsh(&script);

    // Execute a real remote MCP terminal operation over the live WebSocket session after the bot lifecycle churn!
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    cmd_req_tx
        .send(("after-bot-churn".into(), reply_tx))
        .expect("request mcp op");
    let ws_reply = tokio::time::timeout(Duration::from_secs(5), reply_rx)
        .await
        .expect("mcp op timeout")
        .expect("mcp op reply");
    assert!(
        ws_reply.contains("MCP_EXEC_OK:after-bot-churn"),
        "Expected live external MCP to execute terminal command after bot lifecycle churn, got: {ws_reply}"
    );

    // Gracefully stop the real external MCP process
    let stop_mcp_script = format!(
        ". '{}'; $res = Stop-CdrComponentRuntime -RepoRoot '{}' -Component 'mcp' -BinaryPath '{}' -GraceTimeoutMs 3000 -Source 'test-cleanup'; if (-not $res.Stopped) {{ throw 'Real MCP did not stop gracefully' }}",
        journal_script.display(),
        temp.path().display(),
        temp.path()
            .join("target")
            .join("release")
            .join("cdr-remote-agent.exe")
            .display()
    );
    run_pwsh(&stop_mcp_script);
    drop(cmd_req_tx);
    let _ = bridge_task.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t02_bot_only_binary_update_preserves_running_external_mcp_and_stages_mcp_binary() {
    let ws = repo_root();
    let mcp_bin = ensure_mcp_bin();
    let journal_script = ws.join("scripts").join("CdrLaunchJournal.ps1");

    let temp = tempfile::tempdir().expect("tempdir");
    let project_dir = temp.path().join("project_t02");
    fs::create_dir_all(&project_dir).expect("create project_t02");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind bridge");
    let bridge_addr = listener.local_addr().expect("bridge addr");

    let (cmd_req_tx, cmd_req_rx) =
        tokio::sync::mpsc::unbounded_channel::<(String, tokio::sync::oneshot::Sender<String>)>();
    let cmd_req_rx = std::sync::Arc::new(tokio::sync::Mutex::new(cmd_req_rx));
    let project_for_bridge = project_dir.clone();
    let bridge_task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let cmd_rx = cmd_req_rx.clone();
            let proj = project_for_bridge.clone();
            tokio::spawn(async move {
                if let Ok(mut ws) = accept_async(stream).await {
                    let Some(Ok(hello_msg)) = ws.next().await else {
                        return;
                    };
                    let hello_text = hello_msg.to_text().unwrap_or("").to_owned();
                    let is_external_mcp = hello_text.contains("\"device-t02\"");
                    let _ = ws
                        .send(Message::Text(
                            serde_json::to_string(&GatewayInboundMessage::HelloAck {
                                protocol_version: ProtocolV10,
                            })
                            .unwrap()
                            .into(),
                        ))
                        .await;
                    if is_external_mcp {
                        let _ = ws
                            .send(Message::Text(
                                serde_json::to_string(&GatewayInboundMessage::Command(
                                    GatewayCommand::DeviceSession {
                                        request_id: "bind-t02".into(),
                                        thread_id: "thread-t02".into(),
                                        deadline_at: Utc::now() + ChronoDuration::minutes(5),
                                        computer_session_id: "session-t02-valid-01".into(),
                                        computer_session_generation: 1,
                                        working_directory: proj.display().to_string(),
                                        expires_at: Utc::now() + ChronoDuration::minutes(10),
                                    },
                                ))
                                .unwrap()
                                .into(),
                            ))
                            .await;
                        while let Some(Ok(msg)) = ws.next().await {
                            if let Message::Ping(payload) = msg {
                                let _ = ws.send(Message::Pong(payload)).await;
                            } else if msg.is_text() {
                                break;
                            }
                        }
                        let mut rx = cmd_rx.lock().await;
                        loop {
                            tokio::select! {
                                cmd = rx.recv() => {
                                    let Some((marker, reply_tx)) = cmd else { break; };
                                    let req_id = format!("op-t02-{marker}");
                                    let _ = ws
                                        .send(Message::Text(
                                            serde_json::to_string(&GatewayInboundMessage::Command(
                                                GatewayCommand::ProjectOperation {
                                                    request_id: req_id,
                                                    thread_id: "thread-t02".into(),
                                                    deadline_at: Utc::now() + ChronoDuration::minutes(2),
                                                    computer_session_id: Some(
                                                        "session-t02-valid-01".into(),
                                                    ),
                                                    operation: ProjectOperation::Terminal(
                                                        TerminalRequest::TerminalExec {
                                                            terminal_id: None,
                                                            shell: TerminalShell::Powershell,
                                                            command: format!(
                                                                "Write-Output 'MCP_RECOVERY_EXEC_OK:{marker}'"
                                                            ),
                                                            cwd: None,
                                                            environment: std::collections::BTreeMap::new(),
                                                            timeout_seconds: 10,
                                                            cancel_previous: false,
                                                        },
                                                    ),
                                                },
                                            ))
                                            .unwrap()
                                            .into(),
                                        ))
                                        .await;
                                    while let Some(Ok(msg)) = ws.next().await {
                                        if let Message::Ping(payload) = msg {
                                            let _ = ws.send(Message::Pong(payload)).await;
                                        } else if msg.is_text() {
                                            let _ = reply_tx.send(msg.to_text().unwrap_or("").to_owned());
                                            break;
                                        }
                                    }
                                }
                                ws_msg = ws.next() => {
                                    match ws_msg {
                                        Some(Ok(Message::Ping(payload))) => {
                                            let _ = ws.send(Message::Pong(payload)).await;
                                        }
                                        None | Some(Err(_) | Ok(Message::Close(_))) => break,
                                        _ => {}
                                    }
                                }
                            }
                        }
                    } else {
                        while let Some(Ok(msg)) = ws.next().await {
                            if let Message::Ping(payload) = msg {
                                let _ = ws.send(Message::Pong(payload)).await;
                            }
                        }
                    }
                }
            });
        }
    });

    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
. '{journal}'

$temp = '{temp}'
New-Item -ItemType Directory -Path (Join-Path $temp 'bot_data') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $temp 'bot_fixture_data') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $temp 'target\release') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $temp 'scripts') -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_OWNERSHIP=external`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://{bridge_addr}`nCODEX_REMOTE_MCP_DEVICE_ID=device-t02`nCODEX_REMOTE_MCP_DEVICE_TOKEN=secret-t02`n")
$botEnvPath = Join-Path $temp 'bot.env'
[IO.File]::WriteAllText($botEnvPath, "CODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_OWNERSHIP=external`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://{bridge_addr}`nCODEX_REMOTE_MCP_DEVICE_ID=bot-fixture-t02`nCODEX_REMOTE_MCP_DEVICE_TOKEN=secret-t02`nCODEX_MCP_DATA_DIR=bot_fixture_data`nCODEX_REMOTE_MCP_STOP_FILE=.codex_discord_rust.stop`n")

$installedBot = Join-Path $temp 'target\release\cdr-runtime.exe'
$installedMcp = Join-Path $temp 'target\release\cdr-remote-agent.exe'
Copy-Item -LiteralPath '{mcp_bin}' -Destination $installedBot -Force
Copy-Item -LiteralPath '{mcp_bin}' -Destination $installedMcp -Force
[IO.File]::WriteAllText((Join-Path $temp 'scripts\active-helper.ps1'), '# original active helper')

$candidateBot = Join-Path $temp 'candidate-cdr-runtime.exe'
$candidateMcp = Join-Path $temp 'candidate-cdr-remote-agent.exe'
$candidateScripts = Join-Path $temp 'candidate-scripts'
New-Item -ItemType Directory -Path $candidateScripts -Force | Out-Null
Copy-Item -LiteralPath '{mcp_bin}' -Destination $candidateBot -Force
[IO.File]::WriteAllText($candidateMcp, 'new-mcp-binary-bytes-that-must-only-be-staged')
[IO.File]::WriteAllText((Join-Path $candidateScripts 'active-helper.ps1'), '# new staged helper')

$botCmdArgs = @('--env', $botEnvPath)

$bot1 = $null; $bot2 = $null
try {{
    Set-CdrComponentIntent -RepoRoot $temp -Component 'bot' -DesiredState 'running' -Action 'init' -Source 'test' | Out-Null
    Set-CdrComponentIntent -RepoRoot $temp -Component 'mcp' -DesiredState 'running' -Action 'init' -Source 'test' | Out-Null

    $mcpLaunch = Invoke-CdrJournaledProcessLaunch `
        -RepoRoot $temp -Component 'mcp' -FilePath $installedMcp `
        -ArgumentList @('--env', (Join-Path $temp '.env')) `
        -WorkingDirectory $temp -TargetBinaryPath $installedMcp -Action 'start-mcp'

    $statusPath = Join-Path $temp 'bot_data\remote_mcp_status.json'
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    while ([DateTime]::UtcNow -lt $deadline) {{
        if ([IO.File]::Exists($statusPath)) {{
            try {{
                $st = [IO.File]::ReadAllText($statusPath) | ConvertFrom-Json
                if ($st.ready -and $st.hello_ack) {{ break }}
            }} catch {{ }}
        }}
        Start-Sleep -Milliseconds 50
    }}

    $bot1Launch = Invoke-CdrJournaledProcessLaunch `
        -RepoRoot $temp -Component 'bot' -FilePath $installedBot `
        -ArgumentList $botCmdArgs `
        -WorkingDirectory $temp -TargetBinaryPath $installedBot -Action 'start-bot'
    $bot1 = $bot1Launch.Process

    $mcpHashBefore = (Get-FileHash -LiteralPath $installedMcp -Algorithm SHA256).Hash

    # 1. Perform bot-only update while real external MCP is actively running
    $updateRes = Update-CdrBotBinaryOnly `
        -RepoRoot $temp `
        -CandidateBotBinary $candidateBot `
        -CandidateMcpBinary $candidateMcp `
        -CandidateScriptsDir $candidateScripts `
        -BotLaunchArguments $botCmdArgs `
        -GraceTimeoutMs 3000
    $bot2 = $updateRes.ReplacementBot.Process

    if (-not $updateRes.BotUpdated -or $updateRes.McpInterrupted) {{
        throw "Unexpected Update-CdrBotBinaryOnly result: $($updateRes | ConvertTo-Json -Compress)"
    }}
    if (-not $updateRes.McpStagedForNextRestart -or -not [IO.File]::Exists($updateRes.StagedMcpPath)) {{
        throw 'Expected new MCP binary to be staged without touching running MCP executable'
    }}
    if (-not [IO.File]::Exists((Join-Path $temp '.cdr-maintenance\staged-scripts\active-helper.ps1')) -or
        [IO.File]::ReadAllText((Join-Path $temp 'scripts\active-helper.ps1')) -ne '# original active helper') {{
        throw 'Active helper scripts must remain untouched while candidate scripts are staged!'
    }}
    if ((Get-FileHash -LiteralPath $installedMcp -Algorithm SHA256).Hash -cne $mcpHashBefore) {{
        throw 'Running MCP executable on disk was modified during bot-only update!'
    }}
    if (-not (Test-CdrProcessMatchesIdentity -ProcessId $mcpLaunch.ProcessId -ExpectedIdentity $mcpLaunch.ProcessIdentity -ExpectedBinaryPath $installedMcp)) {{
        throw 'Running external MCP process was interrupted during bot-only update!'
    }}
    if (Test-CdrProcessMatchesIdentity -ProcessId $bot1Launch.ProcessId -ExpectedIdentity $bot1Launch.ProcessIdentity -ExpectedBinaryPath $installedBot) {{
        throw 'Old bot process should have exited gracefully'
    }}
    if (-not (Test-CdrProcessMatchesIdentity -ProcessId $updateRes.ReplacementBot.ProcessId -ExpectedIdentity $updateRes.ReplacementBot.ProcessIdentity -ExpectedBinaryPath $installedBot)) {{
        throw 'Replacement bot process is not running after bot-only update'
    }}

    # 2. Verify that if botDesiredState is stopped (user manual stop), Update-CdrBotBinaryOnly updates binary without restarting bot!
    Stop-CdrComponentRuntime -RepoRoot $temp -Component 'bot' -BinaryPath $installedBot -GraceTimeoutMs 3000 -Source 'user-manual-stop' | Out-Null
    $stoppedUpdate = Update-CdrBotBinaryOnly `
        -RepoRoot $temp `
        -CandidateBotBinary $candidateBot `
        -BotLaunchArguments $botCmdArgs `
        -GraceTimeoutMs 3000
    if ($null -ne $stoppedUpdate.ReplacementBot -or (Get-CdrComponentIntent -RepoRoot $temp -Component 'bot').DesiredState -ne 'stopped') {{
        throw 'Update-CdrBotBinaryOnly must not restart bot or clear manual stop when user stop intent is active!'
    }}

    # 3. Also exercise the existing supported maintenance installation function (Install-CdrMaintenanceCandidate in scripts/CdrMaintenanceActions.ps1)
    # while real external cdr-remote-agent.exe is live, proving it replaces ONLY cdr-runtime.exe and leaves cdr-remote-agent.exe and helper scripts untouched!
    $helperBeforeHash = (Get-FileHash -LiteralPath (Join-Path $temp 'scripts\active-helper.ps1') -Algorithm SHA256).Hash
    $maintActions = Join-Path (Split-Path -Parent '{journal}') 'CdrMaintenanceActions.ps1'
    $maintAst = [System.Management.Automation.Language.Parser]::ParseFile($maintActions, [ref]$null, [ref]$null)
    $installFn = $maintAst.FindAll({{ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -ceq 'Install-CdrMaintenanceCandidate' }}, $true) | Select-Object -First 1
    . ([scriptblock]::Create($installFn.Extent.Text))
    $RepoRoot = $temp
    $BinaryPath = $installedBot
    function Assert-CdrMaintenanceNoRuntime($s) {{ }}
    function Assert-CdrMaintenanceArtifacts($s) {{ }}
    function Assert-CdrMaintenanceDeadline($s) {{ }}
    function Get-CdrArtifactHash([string]$p) {{ return (Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash }}
    $maintState = [pscustomobject]@{{
        Operation = 'aabbccddeeff00112233445566778899'
        CandidatePath = $candidateBot
        CandidateHash = (Get-CdrArtifactHash $candidateBot)
    }}
    Install-CdrMaintenanceCandidate $maintState
    if ((Get-FileHash -LiteralPath $installedMcp -Algorithm SHA256).Hash -cne $mcpHashBefore -or
        (Get-FileHash -LiteralPath (Join-Path $temp 'scripts\active-helper.ps1') -Algorithm SHA256).Hash -cne $helperBeforeHash -or
        -not (Test-CdrProcessMatchesIdentity -ProcessId $mcpLaunch.ProcessId -ExpectedIdentity $mcpLaunch.ProcessIdentity -ExpectedBinaryPath $installedMcp)) {{
        throw 'Supported Install-CdrMaintenanceCandidate path modified MCP binary/helpers or interrupted live External MCP!'
    }}

    # 4. Also simulate a candidate bot binary replacement that fails validation/startup and verify External MCP remains alive and ready for remote recovery!
    $badCandidateBot = Join-Path $temp 'bad-candidate-cdr-runtime.exe'
    [IO.File]::WriteAllText($badCandidateBot, 'corrupted-bot-candidate-bytes')
    Set-CdrComponentIntent -RepoRoot $temp -Component 'bot' -DesiredState 'running' -Action 'start-bad-candidate' -Source 'test' | Out-Null
    $badCandidateFailed = $false
    try {{
        Update-CdrBotBinaryOnly -RepoRoot $temp -CandidateBotBinary $badCandidateBot -BotLaunchArguments $botCmdArgs -GraceTimeoutMs 1000 | Out-Null
    }} catch {{
        $badCandidateFailed = $true
    }}
    if (-not $badCandidateFailed) {{
        throw 'Expected corrupted candidate bot launch to fail closed during Update-CdrBotBinaryOnly'
    }}
    if (-not (Test-CdrProcessMatchesIdentity -ProcessId $mcpLaunch.ProcessId -ExpectedIdentity $mcpLaunch.ProcessIdentity -ExpectedBinaryPath $installedMcp)) {{
        throw 'External MCP process was interrupted when candidate bot update failed!'
    }}
}} finally {{
    foreach ($p in @($bot1, $bot2)) {{
        if ($null -ne $p) {{
            $id = Get-CdrProcessIdentity -Process $p
            if ($id) {{ Stop-CdrVerifiedProcess -ProcessId $p.Id -ExpectedIdentity $id | Out-Null }}
        }}
    }}
}}
"#,
        journal = journal_script.display(),
        temp = temp.path().display(),
        bridge_addr = bridge_addr,
        mcp_bin = mcp_bin.display()
    );
    run_pwsh(&script);

    // Approved T03 acceptance criterion: Execute an actual remote MCP recovery command (TerminalExec)
    // over the live WebSocket bridge after bot binary replacement and candidate bot failure!
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    cmd_req_tx
        .send(("after-bot-update-and-candidate-failure".into(), reply_tx))
        .expect("request mcp recovery op");
    let ws_reply = tokio::time::timeout(Duration::from_secs(5), reply_rx)
        .await
        .expect("mcp recovery op timeout")
        .expect("mcp recovery op reply");
    assert!(
        ws_reply.contains("MCP_RECOVERY_EXEC_OK:after-bot-update-and-candidate-failure"),
        "Expected live external MCP to execute remote recovery command after bot binary replacement and candidate failure, got: {ws_reply}"
    );

    let stop_mcp_script = format!(
        ". '{}'; $res = Stop-CdrComponentRuntime -RepoRoot '{}' -Component 'mcp' -BinaryPath '{}' -GraceTimeoutMs 3000 -Source 'test-cleanup' -AllowForceKill; if (-not $res.Stopped) {{ throw 'Real MCP did not stop in t02 cleanup' }}",
        journal_script.display(),
        temp.path().display(),
        temp.path()
            .join("target")
            .join("release")
            .join("cdr-remote-agent.exe")
            .display()
    );
    run_pwsh(&stop_mcp_script);
    drop(cmd_req_tx);
    bridge_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t03_mcp_restart_or_crash_preserves_running_bot_process_identity() {
    let ws = repo_root();
    let bot_bin = PathBuf::from(env!("CARGO_BIN_EXE_cdr-runtime"));
    let soak_bin = PathBuf::from(env!("CARGO_BIN_EXE_cdr-offline-soak"));
    let mcp_bin = ensure_mcp_bin();
    let journal_script = ws.join("scripts").join("CdrLaunchJournal.ps1");

    let discord_double = DiscordRuntimeDouble::spawn().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind bridge");
    let bridge_addr = listener.local_addr().expect("bridge addr");
    let bridge_task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                if let Ok(mut ws) = accept_async(stream).await {
                    let _ = ws.next().await;
                    let _ = ws
                        .send(Message::Text(
                            serde_json::to_string(&GatewayInboundMessage::HelloAck {
                                protocol_version: ProtocolV10,
                            })
                            .unwrap()
                            .into(),
                        ))
                        .await;
                    while let Some(Ok(msg)) = ws.next().await {
                        if let Message::Ping(payload) = msg {
                            let _ = ws.send(Message::Pong(payload)).await;
                        }
                    }
                }
            });
        }
    });

    let local_soak_t03 = temp.path().join("cdr-offline-soak.exe");
    fs::copy(&soak_bin, &local_soak_t03).expect("copy cdr-offline-soak.exe for t03");
    let soak_bin_env = local_soak_t03.to_string_lossy().replace('\\', "/");
    let codex_home_env = temp
        .path()
        .join("codex_home")
        .to_string_lossy()
        .replace('\\', "/");
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
. '{journal}'

$temp = '{temp}'
New-Item -ItemType Directory -Path (Join-Path $temp 'bot_data') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $temp 'target\release') -Force | Out-Null
& '{soak_bin_env}' init-state-db (Join-Path $temp 'codex_home\state_5.sqlite')
if ($LASTEXITCODE -ne 0) {{ throw 'init-state-db failed in t03' }}
$env:CDR_DISCORD_HTTP_PROXY = '{discord_http}'
$env:CDR_DISCORD_GATEWAY_PROXY_URL = 'ws://{discord_ws}'
[IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=Bot.test.token`nDISCORD_ALLOWED_CHANNEL_IDS=42`nCDR_DISCORD_HTTP_PROXY={discord_http}`nCDR_DISCORD_GATEWAY_PROXY_URL=ws://{discord_ws}`nCODEX_HOME={codex_home_env}`nCODEX_EXE={soak_bin_env}`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_OWNERSHIP=external`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://{bridge_addr}`nCODEX_REMOTE_MCP_DEVICE_ID=device-t03`nCODEX_REMOTE_MCP_DEVICE_TOKEN=secret-t03`n")
Write-CdrAtomicJsonFile -Path (Join-Path $temp '.cdr-maintenance\mcp-ownership-state.json') -Object ([ordered]@{{
    schema_version = 1; mode = 'external'; state = 'committed'; committed = $true; updated_at_ms = 1
}})

$installedBot = Join-Path $temp 'target\release\cdr-runtime.exe'
$installedMcp = Join-Path $temp 'target\release\cdr-remote-agent.exe'
Copy-Item -LiteralPath '{bot_bin}' -Destination $installedBot -Force
Copy-Item -LiteralPath '{mcp_bin}' -Destination $installedMcp -Force

function Assert-RealBotProcessesDiscordHelp([int]$ExpectedReplies) {{
    try {{
        $resp = Invoke-RestMethod -Method Get -Uri 'http://{discord_http}/__test/inject_help' -TimeoutSec 25
    }} catch {{
        $outTxt = if ([IO.File]::Exists((Join-Path $temp 'bot.stdout.log'))) {{ (Get-Content -LiteralPath (Join-Path $temp 'bot.stdout.log') -Raw -ErrorAction SilentlyContinue) }} else {{ '' }}
        $errTxt = if ([IO.File]::Exists((Join-Path $temp 'bot.stderr.log'))) {{ (Get-Content -LiteralPath (Join-Path $temp 'bot.stderr.log') -Raw -ErrorAction SilentlyContinue) }} else {{ '' }}
        throw "inject_help failed ($($_.Exception.Message))`nBOT_STDOUT:`n$outTxt`nBOT_STDERR:`n$errTxt"
    }}
    if (-not $resp.ok -or [int]$resp.help_replies -ne $ExpectedReplies) {{
        throw "Real cdr-runtime.exe failed to process Discord !help (expected=$ExpectedReplies): $($resp | ConvertTo-Json -Compress)"
    }}
}}

$bot = $null
try {{
    Set-CdrComponentIntent -RepoRoot $temp -Component 'bot' -DesiredState 'running' -Action 'init' -Source 'test' | Out-Null
    Set-CdrComponentIntent -RepoRoot $temp -Component 'mcp' -DesiredState 'running' -Action 'init' -Source 'test' | Out-Null

    # Start the REAL cdr-runtime.exe binary and verify live Discord command processing before MCP starts
    $botLaunch = Invoke-CdrJournaledProcessLaunch `
        -RepoRoot $temp -Component 'bot' -FilePath $installedBot `
        -ArgumentList @('--env', (Join-Path $temp '.env')) `
        -WorkingDirectory $temp -TargetBinaryPath $installedBot -Action 'start-bot' `
        -RedirectStandardOutput (Join-Path $temp 'bot.stdout.log') `
        -RedirectStandardError (Join-Path $temp 'bot.stderr.log')
    $bot = $botLaunch.Process
    Assert-RealBotProcessesDiscordHelp -ExpectedReplies 1

    # Start real external MCP via the public opt-in supervision entrypoint Invoke-CdrMcpWatchdog
    $wd1 = Invoke-CdrMcpWatchdog -RepoRoot $temp -McpBinaryPath $installedMcp
    if ($wd1.State -ne 'started') {{ throw "Expected Invoke-CdrMcpWatchdog to start MCP, got: $($wd1 | ConvertTo-Json -Compress)" }}

    # Wait for ready status
    $statusPath = Join-Path $temp 'bot_data\remote_mcp_status.json'
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    while ([DateTime]::UtcNow -lt $deadline) {{
        if ([IO.File]::Exists($statusPath)) {{
            try {{
                $st = [IO.File]::ReadAllText($statusPath) | ConvertFrom-Json
                if ($st.ready -and $st.hello_ack) {{ break }}
            }} catch {{ }}
        }}
        Start-Sleep -Milliseconds 50
    }}
    Assert-RealBotProcessesDiscordHelp -ExpectedReplies 2

    # Gracefully stop MCP -> verify watchdog respects stopped_by_intent -> verify real cdr-runtime.exe still processes Discord !help while MCP is stopped!
    $stopMcp = Stop-CdrComponentRuntime -RepoRoot $temp -Component 'mcp' -BinaryPath $installedMcp -GraceTimeoutMs 3000 -Source 'mcp-restart'
    if (-not $stopMcp.Stopped) {{ throw 'Real external MCP did not stop gracefully' }}

    $wdStopped = Invoke-CdrMcpWatchdog -RepoRoot $temp -McpBinaryPath $installedMcp
    if ($wdStopped.State -ne 'stopped_by_intent') {{
        throw "Watchdog must honor MCP stop intent, got: $($wdStopped | ConvertTo-Json -Compress)"
    }}
    Assert-RealBotProcessesDiscordHelp -ExpectedReplies 3

    Set-CdrComponentIntent -RepoRoot $temp -Component 'mcp' -DesiredState 'running' -Action 'start' -Source 'mcp-restart' | Out-Null
    $wd2 = Invoke-CdrMcpWatchdog -RepoRoot $temp -McpBinaryPath $installedMcp
    if ($wd2.State -ne 'started' -or $wd2.ProcessIdentity -ceq $wd1.ProcessIdentity) {{
        throw "Expected watchdog to start a fresh MCP instance, got: $($wd2 | ConvertTo-Json -Compress)"
    }}

    if (-not (Test-CdrProcessMatchesIdentity -ProcessId $botLaunch.ProcessId -ExpectedIdentity $botLaunch.ProcessIdentity -ExpectedBinaryPath $installedBot)) {{
        throw 'Real cdr-runtime.exe process was interrupted when MCP stopped/restarted!'
    }}
    Assert-RealBotProcessesDiscordHelp -ExpectedReplies 4

    # Crash/kill External MCP, verify real cdr-runtime.exe still processes Discord !help while MCP is crashed,
    # and verify that while Wait-RustRuntimeExit in codex-discord-rust-watchdog.ps1 is blocked waiting for a bot process to exit,
    # the crashed External MCP is supervised and restarted during the blocked bot wait!
    Stop-CdrVerifiedProcess -ProcessId $wd2.ProcessId -ExpectedIdentity $wd2.ProcessIdentity -ExpectedBinaryPath $installedMcp | Out-Null
    Assert-RealBotProcessesDiscordHelp -ExpectedReplies 5

    $wdAst = [System.Management.Automation.Language.Parser]::ParseFile((Join-Path (Split-Path -Parent '{journal}') '..\codex-discord-rust-watchdog.ps1'), [ref]$null, [ref]$null)
    $waitFn = $wdAst.FindAll({{ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -ceq 'Wait-RustRuntimeExit' }}, $true) | Select-Object -First 1
    . ([scriptblock]::Create($waitFn.Extent.Text))
    $RepoRoot = $temp
    function Write-RustWatchdogLog([string]$Message) {{ }}
    function Get-VerifiedRustProcessById([int]$ProcessId) {{ return $bot }}
    function Get-RustProcessIdentity($Process) {{ return $botLaunch.ProcessIdentity }}
    $waitTimedOut = $false
    try {{
        Wait-RustRuntimeExit -Process $bot -ExpectedIdentity $botLaunch.ProcessIdentity -Reason 'blocked-bot-stop' -TimeoutSeconds 2
    }} catch {{
        if ($_.Exception.Message -match 'graceful_exit_timeout') {{ $waitTimedOut = $true }} else {{ throw }}
    }}
    if (-not $waitTimedOut) {{ throw 'Expected Wait-RustRuntimeExit to time out on blocked bot' }}
    $mcpAfterBlockedBotWait = Get-CdrComponentObservation -RepoRoot $temp -Component 'mcp' -BinaryPath $installedMcp
    if ($mcpAfterBlockedBotWait.State -ne 'running' -or $mcpAfterBlockedBotWait.ProcessIdentity -ceq $wd2.ProcessIdentity) {{
        throw "Expected crashed External MCP to be supervised and restarted while bot watchdog wait was blocked, got: $($mcpAfterBlockedBotWait | ConvertTo-Json -Compress)"
    }}
    if (-not (Test-CdrProcessMatchesIdentity -ProcessId $botLaunch.ProcessId -ExpectedIdentity $botLaunch.ProcessIdentity -ExpectedBinaryPath $installedBot)) {{
        throw 'Real cdr-runtime.exe process was interrupted after MCP crash + watchdog restart!'
    }}
    Assert-RealBotProcessesDiscordHelp -ExpectedReplies 6

    # Also verify Wait-RustThreadsQuietForRestart (--restart-readiness) supervises and restarts a crashed External MCP
    # while blocked waiting on the readiness probe (covering an existing IgnoreNew scheduled task during 900s readiness waits)!
    Stop-CdrVerifiedProcess -ProcessId $mcpAfterBlockedBotWait.ProcessId -ExpectedIdentity $mcpAfterBlockedBotWait.ProcessIdentity -ExpectedBinaryPath $installedMcp | Out-Null
    $quietFn = $wdAst.FindAll({{ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -ceq 'Wait-RustThreadsQuietForRestart' }}, $true) | Select-Object -First 1
    . ([scriptblock]::Create($quietFn.Extent.Text))
    $slowProbeCmd = Join-Path $temp 'slow_readiness_probe.cmd'
    [IO.File]::WriteAllText($slowProbeCmd, "@echo off`r`nping -n 3 127.0.0.1 >nul`r`nexit /b 0`r`n")
    $BinaryPath = $slowProbeCmd
    $RestartQuietSeconds = 1
    $RestartWaitTimeoutSeconds = 5
    $EnvPath = Join-Path $temp '.env'
    Wait-RustThreadsQuietForRestart
    $mcpAfterQuietWait = Get-CdrComponentObservation -RepoRoot $temp -Component 'mcp' -BinaryPath $installedMcp
    if ($mcpAfterQuietWait.State -ne 'running' -or $mcpAfterQuietWait.ProcessIdentity -ceq $mcpAfterBlockedBotWait.ProcessIdentity) {{
        throw "Expected crashed External MCP to be supervised and restarted during Wait-RustThreadsQuietForRestart, got: $($mcpAfterQuietWait | ConvertTo-Json -Compress)"
    }}
}} finally {{
    Stop-CdrComponentRuntime -RepoRoot $temp -Component 'mcp' -BinaryPath $installedMcp -GraceTimeoutMs 2000 -Source 'cleanup' -AllowForceKill | Out-Null
    Stop-CdrComponentRuntime -RepoRoot $temp -Component 'bot' -BinaryPath $installedBot -GraceTimeoutMs 3000 -Source 'cleanup' -AllowForceKill | Out-Null
    $soakPath = [IO.Path]::GetFullPath((Join-Path $temp 'cdr-offline-soak.exe'))
    Get-Process -Name 'cdr-offline-soak' -ErrorAction SilentlyContinue | ForEach-Object {{
        try {{
            if ($_.Path -and [IO.Path]::GetFullPath($_.Path) -ieq $soakPath) {{
                $soakId = Get-CdrProcessIdentity -Process $_
                if (-not [string]::IsNullOrWhiteSpace($soakId)) {{
                    Stop-CdrVerifiedProcess -ProcessId $_.Id -ExpectedIdentity $soakId -ExpectedBinaryPath $soakPath | Out-Null
                }}
            }}
        }} catch {{ }}
    }}
}}
"#,
        journal = journal_script.display(),
        temp = temp.path().display(),
        discord_http = discord_double.http_addr,
        discord_ws = discord_double.ws_addr,
        codex_home_env = codex_home_env,
        soak_bin_env = soak_bin_env,
        bridge_addr = bridge_addr,
        bot_bin = bot_bin.display(),
        mcp_bin = mcp_bin.display()
    );
    run_pwsh(&script);
    bridge_task.abort();
    discord_double.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t11_safe_embedded_to_external_cutover_and_manual_rollback_guard() {
    let ws = repo_root();
    let bot_bin = PathBuf::from(env!("CARGO_BIN_EXE_cdr-runtime"));
    let soak_bin = PathBuf::from(env!("CARGO_BIN_EXE_cdr-offline-soak"));
    let mcp_bin = ensure_mcp_bin();
    let journal_script = ws.join("scripts").join("CdrLaunchJournal.ps1");

    let discord_double = DiscordRuntimeDouble::spawn().await;
    let temp = tempfile::tempdir().expect("tempdir");
    // The legacy readiness process must stay alive until the cutover scans it.
    // A real running queue row blocks readiness; otherwise the helper may exit
    // before the test starts, which says nothing about the cutover fence.
    {
        let conn = cdr_store::schema::open_initialized(&temp.path().join("discord_mirror.sqlite"))
            .expect("initialize legacy helper fixture store");
        conn.execute_batch(
            "INSERT INTO codex_turn_queue (job_id,target_thread_id,channel_id,prompt,queued,ack_sent,state,attempt_count,baseline_turn_ids,last_error,created_at,updated_at) VALUES ('fixture-job','fixture-thread',1,'fixture',0,0,'running',1,'[]','',1,1);",
        )
        .expect("insert blocking fixture job");
    }
    let project_dir = temp.path().join("project_t11");
    fs::create_dir_all(&project_dir).expect("create project_t11");
    let embedded_bound_ack_path = temp.path().join("bot_data").join("embedded_bound_ack.txt");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind bridge");
    let bridge_addr = listener.local_addr().expect("bridge addr");
    let real_embedded_bound = Arc::new(AtomicBool::new(false));
    let (cmd_req_tx, cmd_req_rx) =
        tokio::sync::mpsc::unbounded_channel::<(String, tokio::sync::oneshot::Sender<String>)>();
    let cmd_req_rx = Arc::new(tokio::sync::Mutex::new(cmd_req_rx));

    let bound_for_bridge = real_embedded_bound.clone();
    let proj_for_bridge = project_dir.clone();
    let ack_path_for_bridge = embedded_bound_ack_path.clone();
    let cmd_rx_for_bridge = cmd_req_rx.clone();
    let bridge_task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let bound_flag = bound_for_bridge.clone();
            let proj = proj_for_bridge.clone();
            let ack_path = ack_path_for_bridge.clone();
            let cmd_rx = cmd_rx_for_bridge.clone();
            tokio::spawn(async move {
                if let Ok(mut ws) = accept_async(stream).await {
                    let Some(Ok(hello_msg)) = ws.next().await else {
                        return;
                    };
                    let hello_text = hello_msg.to_text().unwrap_or("").to_owned();
                    let is_real_cutover_device = hello_text.contains("\"device-t11-real\"");
                    let _ = ws
                        .send(Message::Text(
                            serde_json::to_string(&GatewayInboundMessage::HelloAck {
                                protocol_version: ProtocolV10,
                            })
                            .unwrap()
                            .into(),
                        ))
                        .await;
                    if is_real_cutover_device {
                        let is_first_real_conn = !bound_flag.swap(true, Ordering::SeqCst);
                        if is_first_real_conn {
                            // First connection with device-t11-real is the real embedded cdr-runtime.exe owner:
                            // bind a live DeviceSession before cutover so it must be persisted in remote-mcp-restart.json!
                            let _ = ws
                                .send(Message::Text(
                                    serde_json::to_string(&GatewayInboundMessage::Command(
                                        GatewayCommand::DeviceSession {
                                            request_id: "bind-t11-real".into(),
                                            thread_id: "thread-t11-real".into(),
                                            deadline_at: Utc::now() + ChronoDuration::minutes(5),
                                            computer_session_id: "session-t11-handoff-01".into(),
                                            computer_session_generation: 1,
                                            working_directory: proj.display().to_string(),
                                            expires_at: Utc::now() + ChronoDuration::minutes(10),
                                        },
                                    ))
                                    .unwrap()
                                    .into(),
                                ))
                                .await;
                            while let Some(Ok(msg)) = ws.next().await {
                                if let Message::Ping(payload) = msg {
                                    let _ = ws.send(Message::Pong(payload)).await;
                                } else if msg.is_text() {
                                    let _ = fs::write(&ack_path, "bound");
                                    break;
                                }
                            }
                            while let Some(Ok(msg)) = ws.next().await {
                                if let Message::Ping(payload) = msg {
                                    let _ = ws.send(Message::Pong(payload)).await;
                                }
                            }
                        } else {
                            // Second connection with device-t11-real is the external cdr-remote-agent.exe launched by Invoke-CdrMcpOwnershipCutover:
                            // Do NOT send DeviceSession (which carries working_directory); send ProjectSession (which has NO working_directory and
                            // fails with binding_missing unless thread-t11-real -> project_t11 was restored from remote-mcp-restart.json!),
                            // then execute TerminalExec against the restored project!
                            let _ = ws
                                .send(Message::Text(
                                    serde_json::to_string(&GatewayInboundMessage::Command(
                                        GatewayCommand::ProjectSession {
                                            request_id: "proj-session-t11-real".into(),
                                            thread_id: "thread-t11-real".into(),
                                            deadline_at: Utc::now() + ChronoDuration::minutes(5),
                                            computer_session_id: "session-t11-handoff-01".into(),
                                            computer_session_generation: 2,
                                        },
                                    ))
                                    .unwrap()
                                    .into(),
                                ))
                                .await;
                            while let Some(Ok(msg)) = ws.next().await {
                                if let Message::Ping(payload) = msg {
                                    let _ = ws.send(Message::Pong(payload)).await;
                                } else if msg.is_text() {
                                    break;
                                }
                            }
                            let mut rx = cmd_rx.lock().await;
                            loop {
                                tokio::select! {
                                    cmd = rx.recv() => {
                                        let Some((marker, reply_tx)) = cmd else { break; };
                                        let req_id = format!("op-t11-{marker}");
                                        let _ = ws
                                            .send(Message::Text(
                                                serde_json::to_string(&GatewayInboundMessage::Command(
                                                    GatewayCommand::ProjectOperation {
                                                        request_id: req_id,
                                                        thread_id: "thread-t11-real".into(),
                                                        deadline_at: Utc::now()
                                                            + ChronoDuration::minutes(2),
                                                        computer_session_id: Some(
                                                            "session-t11-handoff-01".into(),
                                                        ),
                                                        operation: ProjectOperation::Terminal(
                                                            TerminalRequest::TerminalExec {
                                                                terminal_id: None,
                                                                shell: TerminalShell::Powershell,
                                                                command: format!(
                                                                    "Write-Output 'CUTOVER_HANDOFF_RESTORED_OK:{marker}'"
                                                                ),
                                                                cwd: None,
                                                                environment:
                                                                    std::collections::BTreeMap::new(),
                                                                timeout_seconds: 10,
                                                                cancel_previous: false,
                                                            },
                                                        ),
                                                    },
                                                ))
                                                .unwrap()
                                                .into(),
                                            ))
                                            .await;
                                        while let Some(Ok(msg)) = ws.next().await {
                                            if let Message::Ping(payload) = msg {
                                                let _ = ws.send(Message::Pong(payload)).await;
                                            } else if msg.is_text() {
                                                let _ =
                                                    reply_tx.send(msg.to_text().unwrap_or("").to_owned());
                                                break;
                                            }
                                        }
                                    }
                                    ws_msg = ws.next() => {
                                        match ws_msg {
                                            Some(Ok(Message::Ping(payload))) => {
                                                let _ = ws.send(Message::Pong(payload)).await;
                                            }
                                            None | Some(Err(_) | Ok(Message::Close(_))) => break,
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        while let Some(Ok(msg)) = ws.next().await {
                            if let Message::Ping(payload) = msg {
                                let _ = ws.send(Message::Pong(payload)).await;
                            }
                        }
                    }
                }
            });
        }
    });

    let baseline_cmd_fixture = repo_root()
        .join("crates")
        .join("cdr-runtime")
        .join("tests")
        .join("fixtures")
        .join("baseline_56ab736f_codex_discord_bot.cmd");
    let local_soak_t03 = temp.path().join("cdr-offline-soak.exe");
    fs::copy(&soak_bin, &local_soak_t03).expect("copy cdr-offline-soak.exe for t03");
    let soak_bin_env = local_soak_t03.to_string_lossy().replace('\\', "/");
    let codex_home_env = temp
        .path()
        .join("codex_home")
        .to_string_lossy()
        .replace('\\', "/");
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
. '{journal}'

$temp = '{temp}'
New-Item -ItemType Directory -Path (Join-Path $temp 'bot_data') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $temp 'target\release') -Force | Out-Null
$psExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$botBin = Join-Path $temp 'target\release\cdr-runtime.exe'
$mcpBin = Join-Path $temp 'target\release\cdr-remote-agent.exe'
Copy-Item -LiteralPath '{bot_bin}' -Destination $botBin -Force
Copy-Item -LiteralPath '{mcp_bin}' -Destination $mcpBin -Force
$statePath = Join-Path $temp '.cdr-maintenance\mcp-ownership-state.json'

# Verify exact baseline 56ab736f42b23d1a5359d0e6714f97f22179b90a codex-discord-bot.cmd fixture provenance (git blob 3f65ed33a810e05637ddfdcedab0cdae8768b314)
$baselineCmdFixture = '{baseline_cmd_fixture}'
$baselineCmdSha = (Get-FileHash -LiteralPath $baselineCmdFixture -Algorithm SHA256).Hash.ToUpperInvariant()
if ($baselineCmdSha -cne '841C6F91A01A8E45686EA469F6D2779BE69C6365E0B9DB62A14B4C447A814FAB') {{
    throw "Baseline codex-discord-bot.cmd fixture SHA256 mismatch: $baselineCmdSha"
}}
$oldCmdPath = Join-Path $temp 'codex-discord-bot.cmd'
Copy-Item -LiteralPath $baselineCmdFixture -Destination $oldCmdPath -Force

# Case 0: Verify Invoke-CdrMcpOwnershipCutover default EmbeddedExitTimeoutMs is 45000ms (preserving 45s bot cleanup deadline),
# and verify Test-CdrExternalMcpCutoverStatusReady strictly rejects string "false" booleans, missing/null/stale connection_verified_at_ms, and config mismatches!
$cutoverCmd = Get-Command Invoke-CdrMcpOwnershipCutover -ErrorAction Stop
$cutoverAst = $cutoverCmd.ScriptBlock.Ast
$embedParam = $cutoverAst.Body.ParamBlock.Parameters | Where-Object {{ $_.Name.VariablePath.UserPath -eq 'EmbeddedExitTimeoutMs' }} | Select-Object -First 1
if ($null -eq $embedParam -or [int]$embedParam.DefaultValue.Extent.Text -ne 45000) {{
    throw "Expected Invoke-CdrMcpOwnershipCutover default EmbeddedExitTimeoutMs=45000, got: $($embedParam.DefaultValue.Extent.Text)"
}}

& '{soak_bin_env}' init-state-db (Join-Path $temp 'codex_home\state_5.sqlite')
if ($LASTEXITCODE -ne 0) {{ throw 'init-state-db failed in t11 Case 0' }}
[IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`nCODEX_HOME={codex_home_env}`nCODEX_EXE={soak_bin_env}`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_OWNERSHIP=external`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://{bridge_addr}`nCODEX_REMOTE_MCP_DEVICE_ID=device-t11`nCODEX_REMOTE_MCP_DEVICE_TOKEN=secret-t11`n")
$checkPaths = Resolve-CdrRemoteMcpPaths -RepoRoot $temp
$mcpBinHash = (Get-FileHash -LiteralPath $mcpBin -Algorithm SHA256).Hash.ToLowerInvariant()
$selfIdentity = Get-CdrProcessIdentity -Process (Get-Process -Id $PID)
$stringBoolStatusPath = Join-Path $temp 'bot_data\bad_string_bool_status.json'
$nowCase0 = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
Write-CdrAtomicJsonFile -Path $stringBoolStatusPath -Object ([ordered]@{{
    schema_version = 1; component = 'mcp'; install_root = $temp; pid = [int]$PID; process_identity = $selfIdentity
    executable_path = $mcpBin; artifact_sha256 = $mcpBinHash
    build_info = @{{ schema_version = 1; package_version = '0.1.0-dev'; source_state = 'dirty' }}
    updated_at_ms = $nowCase0; heartbeat_at_ms = $nowCase0; connection_verified_at_ms = $nowCase0
    device_id = 'device-t11'; bridge_fingerprint = $checkPaths.BridgeFingerprint
    hello_ack = 'false'; ready = 'false'; draining = $false; generation = 1
}})
if (Test-CdrExternalMcpCutoverStatusReady -StatusPath $stringBoolStatusPath -McpPaths $checkPaths -ExpectedPid $PID -ExpectedIdentity $selfIdentity -ExpectedBinaryPath $mcpBin -ExpectedArtifactHash $mcpBinHash) {{
    throw 'Test-CdrExternalMcpCutoverStatusReady must reject string "false" on ready/hello_ack!'
}}
# Verify Test-CdrExternalMcpCutoverStatusReady rejects missing, null, stale, future, and wrong-type connection_verified_at_ms (R3)
foreach ($connMode in @('missing', 'null', 'stale', 'future', 'bad_type')) {{
    $nowConn = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $recConn = [ordered]@{{
        schema_version = 1; component = 'mcp'; install_root = $temp; pid = [int]$PID; process_identity = $selfIdentity
        executable_path = $mcpBin; artifact_sha256 = $mcpBinHash
        build_info = @{{ schema_version = 1; package_version = '0.1.0-dev'; source_state = 'dirty' }}
        updated_at_ms = $nowConn; heartbeat_at_ms = $nowConn
        device_id = 'device-t11'; bridge_fingerprint = $checkPaths.BridgeFingerprint
        hello_ack = $true; ready = $true; draining = $false; generation = 1
    }}
    if ($connMode -eq 'null') {{ $recConn['connection_verified_at_ms'] = $null }}
    elseif ($connMode -eq 'stale') {{ $recConn['connection_verified_at_ms'] = $nowConn - 30000 }}
    elseif ($connMode -eq 'future') {{ $recConn['connection_verified_at_ms'] = $nowConn + 30000 }}
    elseif ($connMode -eq 'bad_type') {{ $recConn['connection_verified_at_ms'] = 'not-an-int' }}
    Write-CdrAtomicJsonFile -Path $stringBoolStatusPath -Object $recConn
    if (Test-CdrExternalMcpCutoverStatusReady -StatusPath $stringBoolStatusPath -McpPaths $checkPaths -ExpectedPid $PID -ExpectedIdentity $selfIdentity -ExpectedBinaryPath $mcpBin -ExpectedArtifactHash $mcpBinHash) {{
        throw "Test-CdrExternalMcpCutoverStatusReady must reject connection_verified_at_ms mode=$connMode!"
    }}
}}
# Also verify Test-CdrExternalMcpCutoverStatusReady rejects malformed JSON, stale timestamps, and wrong-config device_id/bridge_fingerprint
[IO.File]::WriteAllText($stringBoolStatusPath, '{{malformed-not-json', [Text.UTF8Encoding]::new($false))
if (Test-CdrExternalMcpCutoverStatusReady -StatusPath $stringBoolStatusPath -McpPaths $checkPaths -ExpectedPid $PID -ExpectedIdentity $selfIdentity -ExpectedBinaryPath $mcpBin -ExpectedArtifactHash $mcpBinHash) {{
    throw 'Test-CdrExternalMcpCutoverStatusReady must reject malformed JSON status!'
}}
$staleMs = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() - 60000
Write-CdrAtomicJsonFile -Path $stringBoolStatusPath -Object ([ordered]@{{
    schema_version = 1; component = 'mcp'; install_root = $temp; pid = [int]$PID; process_identity = $selfIdentity
    executable_path = $mcpBin; artifact_sha256 = $mcpBinHash
    build_info = @{{ schema_version = 1; package_version = '0.1.0-dev'; source_state = 'dirty' }}
    updated_at_ms = $staleMs; heartbeat_at_ms = $staleMs; connection_verified_at_ms = $staleMs
    device_id = 'device-t11'; bridge_fingerprint = $checkPaths.BridgeFingerprint
    hello_ack = $true; ready = $true; draining = $false; generation = 1
}})
if (Test-CdrExternalMcpCutoverStatusReady -StatusPath $stringBoolStatusPath -McpPaths $checkPaths -ExpectedPid $PID -ExpectedIdentity $selfIdentity -ExpectedBinaryPath $mcpBin -ExpectedArtifactHash $mcpBinHash) {{
    throw 'Test-CdrExternalMcpCutoverStatusReady must reject stale status timestamps!'
}}
$nowWrongCfg = [int64][DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
Write-CdrAtomicJsonFile -Path $stringBoolStatusPath -Object ([ordered]@{{
    schema_version = 1; component = 'mcp'; install_root = $temp; pid = [int]$PID; process_identity = $selfIdentity
    executable_path = $mcpBin; artifact_sha256 = $mcpBinHash
    build_info = @{{ schema_version = 1; package_version = '0.1.0-dev'; source_state = 'dirty' }}
    updated_at_ms = $nowWrongCfg; heartbeat_at_ms = $nowWrongCfg; connection_verified_at_ms = $nowWrongCfg
    device_id = 'wrong-device-id'; bridge_fingerprint = $checkPaths.BridgeFingerprint
    hello_ack = $true; ready = $true; draining = $false; generation = 1
}})
if (Test-CdrExternalMcpCutoverStatusReady -StatusPath $stringBoolStatusPath -McpPaths $checkPaths -ExpectedPid $PID -ExpectedIdentity $selfIdentity -ExpectedBinaryPath $mcpBin -ExpectedArtifactHash $mcpBinHash) {{
    throw 'Test-CdrExternalMcpCutoverStatusReady must reject wrong device_id/config!'
}}

# Case 0b: Exact baseline 56ab736f42b23d1a5359d0e6714f97f22179b90a codex-discord-bot.cmd launched by relative path (cmd.exe /d /c codex-discord-bot.cmd ...) and admitted before cutover fence:
# Invoke-CdrMcpOwnershipCutover must detect the live relative legacy helper, hold in cutover_failed_hold with legacy_helper_still_active,
# and keep .codex_discord_bot.disabled and .codex_discord_runtime.cutover in place!
Remove-Item -LiteralPath (Join-Path $temp '.codex_discord_bot.disabled') -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath (Join-Path $temp 'discord_launcher.log') -Force -ErrorAction SilentlyContinue
$admittedOldCmd = Start-Process -FilePath 'cmd.exe' -ArgumentList @('/d', '/c', 'codex-discord-bot.cmd --restart-readiness --restart-quiet-seconds 5 --restart-wait-timeout-seconds 10') -WorkingDirectory $temp -WindowStyle Hidden -PassThru
try {{
    Start-Sleep -Milliseconds 150
    $admittedOldCmd.Refresh()
    if ($admittedOldCmd.HasExited) {{ throw "Legacy helper fixture exited before cutover (exit=$($admittedOldCmd.ExitCode))" }}
    $legacyHelperHeld = $false
    try {{
        Invoke-CdrMcpOwnershipCutover -RepoRoot $temp -McpBinaryPath $mcpBin -LegacyHelperExitTimeoutMs 250 -ReadinessTimeoutMs 500 | Out-Null
    }} catch {{
        if ($_.Exception.Message -match 'already-admitted legacy helper codex-discord-bot\.cmd') {{
            $legacyHelperHeld = $true
        }} else {{
            throw
        }}
    }}
    $helperHoldState = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    if (-not $legacyHelperHeld -or
        $helperHoldState.state -cne 'cutover_failed_hold' -or
        [bool]$helperHoldState.committed -or
        $helperHoldState.reason -notmatch '^legacy_helper_still_active:codex-discord-bot\.cmd:' -or
        -not [IO.File]::Exists((Join-Path $temp '.codex_discord_bot.disabled')) -or
        -not [IO.File]::Exists((Join-Path $temp '.codex_discord_runtime.cutover'))) {{
        throw "Active delayed codex-discord-bot.cmd did not hold cutover with fences intact: $($helperHoldState | ConvertTo-Json -Compress)"
    }}
    # While the fence is in place, a new invocation of the exact baseline codex-discord-bot.cmd must immediately exit disabled and log direct_launch_disabled!
    $fencedOut = & cmd.exe /d /c "`"$oldCmdPath`"" 2>&1 | Out-String
    $launcherLog0b = if ([IO.File]::Exists((Join-Path $temp 'discord_launcher.log'))) {{ [IO.File]::ReadAllText((Join-Path $temp 'discord_launcher.log')) }} else {{ '' }}
    if ($LASTEXITCODE -ne 0 -or $fencedOut -notmatch 'disabled by marker' -or $launcherLog0b -notmatch 'direct_launch_disabled marker=') {{
        throw "Baseline codex-discord-bot.cmd was not fenced by .codex_discord_bot.disabled: out=$fencedOut log=$launcherLog0b"
    }}
}} finally {{
    if ($null -ne $admittedOldCmd -and -not $admittedOldCmd.HasExited) {{
        try {{ & taskkill.exe /PID $admittedOldCmd.Id /T /F | Out-Null }} catch {{ }}
        $admittedOldCmd.WaitForExit(3000) | Out-Null
    }}
    Remove-Item -LiteralPath (Join-Path $temp '.codex_discord_bot.disabled') -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath (Join-Path $temp '.codex_discord_runtime.cutover') -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath (Join-Path $temp 'discord_launcher.log') -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $statePath -Force -ErrorAction SilentlyContinue
    Move-Item -LiteralPath (Join-Path $temp 'discord_mirror.sqlite') -Destination (Join-Path $temp 'legacy-helper-fixture.sqlite') -ErrorAction Stop
}}

# Case 1: If external MCP readiness fails during cutover, state MUST be held in cutover_failed_hold
# (mode='external', state='cutover_failed_hold', committed=$false) and NEVER auto-rolled back to embedded!
[IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_OWNERSHIP=external`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://127.0.0.1:9/unreachable`nCODEX_REMOTE_MCP_DEVICE_ID=device-t11`nCODEX_REMOTE_MCP_DEVICE_TOKEN=secret-t11`n")
$failedCutoverThrew = $false
try {{
    Invoke-CdrMcpOwnershipCutover -RepoRoot $temp -McpBinaryPath $mcpBin -ReadinessTimeoutMs 500 | Out-Null
}} catch {{
    $failedCutoverThrew = $true
}}
if (-not $failedCutoverThrew) {{ throw 'Expected cutover against unreachable bridge to fail readiness' }}
$failedState = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
if ($failedState.mode -ne 'external' -or $failedState.state -ne 'cutover_failed_hold' -or [bool]$failedState.committed -or -not [bool]$failedState.embedded_owner_exited) {{
    throw "Failed cutover must hold in cutover_failed_hold and retain embedded_owner_exited: $($failedState | ConvertTo-Json -Compress)"
}}
Stop-CdrComponentRuntime -RepoRoot $temp -Component 'mcp' -BinaryPath $mcpBin -GraceTimeoutMs 500 -AllowForceKill | Out-Null
Clear-CdrComponentLaunchJournal -RepoRoot $temp -Component 'mcp'

# Case 1b: If an interrupted cutover state has lost its embedded_owner_identity evidence, resuming cutover without ExpectedEmbeddedIdentity must hold with embedded_exit_evidence_lost!
Write-CdrAtomicJsonFile -Path $statePath -Object ([ordered]@{{
    schema_version = 1; mode = 'external'; state = 'cutover_pending'; committed = $false; updated_at_ms = 1
}})
$lostEvidenceHeld = $false
try {{
    Invoke-CdrMcpOwnershipCutover -RepoRoot $temp -McpBinaryPath $mcpBin -ReadinessTimeoutMs 500 | Out-Null
}} catch {{
    if ($_.Exception.Message -match 'missing embedded owner exit evidence') {{ $lostEvidenceHeld = $true }} else {{ throw }}
}}
$lostState = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
if (-not $lostEvidenceHeld -or $lostState.reason -ne 'embedded_exit_evidence_lost') {{
    throw "Expected interrupted cutover with lost embedded exit evidence to hold, got: $($lostState | ConvertTo-Json -Compress)"
}}

# Case 1c: If a Stop intent for MCP is issued while Invoke-CdrMcpOwnershipCutover is waiting for the embedded owner to exit,
# cutover must NOT silently clear the Stop and must hold in cutover_failed_hold while retaining embedded_owner_exited = $true!
$embeddedStopRaceScript = Join-Path $temp 'embedded_stop_race.ps1'
[IO.File]::WriteAllText($embeddedStopRaceScript, @"
`$ErrorActionPreference = 'Stop'
. '$($('{journal}').Replace("'", "''"))'
`$stopFile = Join-Path '$($temp.Replace("'", "''"))' 'bot_data\remote_mcp.stop'
while (-not [IO.File]::Exists(`$stopFile)) {{ Start-Sleep -Milliseconds 20 }}
# Issue a manual Stop for MCP during the embedded-owner exit wait!
Set-CdrComponentIntent -RepoRoot '$($temp.Replace("'", "''"))' -Component 'mcp' -DesiredState 'stopped' -Action 'stop' -Source 'operator-stop-during-cutover' | Out-Null
"@)
$embeddedRaceProc = Start-Process -FilePath $psExe -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',$embeddedStopRaceScript) -WindowStyle Hidden -PassThru
$embeddedRaceId = Get-CdrProcessIdentity -Process $embeddedRaceProc
Set-CdrComponentIntent -RepoRoot $temp -Component 'mcp' -DesiredState 'running' -Action 'pre-cutover' -Source 'test' | Out-Null
$stopDuringCutoverHeld = $false
try {{
    Invoke-CdrMcpOwnershipCutover `
        -RepoRoot $temp `
        -McpBinaryPath $mcpBin `
        -BotBinaryPath $psExe `
        -ExpectedEmbeddedIdentity $embeddedRaceId `
        -EmbeddedExitTimeoutMs 3000 `
        -ReadinessTimeoutMs 2000 | Out-Null
}} catch {{
    if ($_.Exception.Message -match 'cdr_stale_intent_revision|cdr_manual_stop_active') {{
        $stopDuringCutoverHeld = $true
    }} else {{
        throw
    }}
}}
$raceHoldState = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
if (-not $stopDuringCutoverHeld -or
    $raceHoldState.state -ne 'cutover_failed_hold' -or
    $raceHoldState.embedded_owner_identity -cne $embeddedRaceId -or
    -not [bool]$raceHoldState.embedded_owner_exited -or
    (Get-CdrComponentIntent -RepoRoot $temp -Component 'mcp').DesiredState -ne 'stopped') {{
    throw "Stop issued during cutover embedded-owner wait was not honored or exit evidence was lost: $($raceHoldState | ConvertTo-Json -Compress)"
}}
Set-CdrComponentIntent -RepoRoot $temp -Component 'mcp' -DesiredState 'running' -Action 'resume-cutover' -Source 'test' | Out-Null

# Case 2: Active embedded owner with healthy .codex_discord_rust.drain.identity (state=open) + R4 commit-before-fence-release fault injections!
[IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_OWNERSHIP=external`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://{bridge_addr}`nCODEX_REMOTE_MCP_DEVICE_ID=device-t11`nCODEX_REMOTE_MCP_DEVICE_TOKEN=secret-t11`n")

# First verify that a non-open drain.identity (state=sealed, i.e., active restart drain) is rejected!
$drainIdPath = Join-Path $temp '.codex_discord_rust.drain.identity'
[IO.File]::WriteAllText($drainIdPath, "version=1`nruntime_id=sealed-rt-1`npid=$PID`nstate=sealed`n")
$sealedRejected = $false
try {{
    Invoke-CdrMcpOwnershipCutover -RepoRoot $temp -McpBinaryPath $mcpBin -ReadinessTimeoutMs 500 | Out-Null
}} catch {{
    if ($_.Exception.Message -match 'drain_identity_state_not_open:sealed') {{ $sealedRejected = $true }} else {{ throw }}
}}
if (-not $sealedRejected) {{
    throw 'Expected non-open .codex_discord_rust.drain.identity (state=sealed) to hold cutover!'
}}

$embeddedScript = Join-Path $temp 'embedded_owner.ps1'
[IO.File]::WriteAllText($embeddedScript, @"
`$stopFile = Join-Path '$($temp.Replace("'", "''"))' 'bot_data\remote_mcp.stop'
`$disabledFence = Join-Path '$($temp.Replace("'", "''"))' '.codex_discord_bot.disabled'
`$fenceObservedPath = Join-Path '$($temp.Replace("'", "''"))' 'bot_data\disabled_fence_observed.txt'
while (`$true) {{
    if ([IO.File]::Exists(`$stopFile)) {{
        if ([IO.File]::Exists(`$disabledFence)) {{
            [IO.File]::WriteAllText(`$fenceObservedPath, [IO.File]::ReadAllText(`$disabledFence).Trim())
        }}
        `$txt = [IO.File]::ReadAllText(`$stopFile).Trim().ToLowerInvariant()
        if (`$txt -match 'handoff') {{
            [IO.File]::Delete(`$stopFile)
            break
        }}
    }}
    Start-Sleep -Milliseconds 30
}}
"@)
$embeddedProc = Start-Process -FilePath $psExe -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',$embeddedScript) -WindowStyle Hidden -PassThru
$embeddedIdentity = Get-CdrProcessIdentity -Process $embeddedProc
[IO.File]::WriteAllText($drainIdPath, "version=1`nruntime_id=healthy-emb-rt-1`npid=$($embeddedProc.Id)`nstate=open`n")

$mcpProc = $null
try {{
    # R4 Fault Injection A: Simulate commit write failure (-OnBeforeCommitWrite) -> fences MUST remain in place, external MCP stopped, state=cutover_failed_hold
    $commitWriteFailed = $false
    try {{
        Invoke-CdrMcpOwnershipCutover `
            -RepoRoot $temp `
            -McpBinaryPath $mcpBin `
            -BotBinaryPath $psExe `
            -ExpectedEmbeddedIdentity $embeddedIdentity `
            -EmbeddedExitTimeoutMs 3000 `
            -ReadinessTimeoutMs 8000 `
            -OnBeforeCommitWrite {{ throw 'injected_commit_write_io_failure' }} | Out-Null
    }} catch {{
        if ($_.Exception.Message -match 'injected_commit_write_io_failure') {{
            $commitWriteFailed = $true
        }} else {{
            throw
        }}
    }}
    $afterCommitFailState = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    if (-not $commitWriteFailed -or
        $afterCommitFailState.state -cne 'cutover_failed_hold' -or
        [bool]$afterCommitFailState.committed -or
        -not [bool]$afterCommitFailState.embedded_owner_exited -or
        -not [IO.File]::Exists((Join-Path $temp '.codex_discord_runtime.cutover')) -or
        -not [IO.File]::Exists((Join-Path $temp '.codex_discord_bot.disabled'))) {{
        throw "R4 OnBeforeCommitWrite failure did not retain cutover fences and cutover_failed_hold: $($afterCommitFailState | ConvertTo-Json -Compress)"
    }}
    # Execute exact baseline 56ab736f42b23d1a5359d0e6714f97f22179b90a codex-discord-bot.cmd at R4 failure boundary A: must be fenced!
    Remove-Item -LiteralPath (Join-Path $temp 'discord_launcher.log') -Force -ErrorAction SilentlyContinue
    $fencedR4A = & cmd.exe /d /c "`"$oldCmdPath`"" 2>&1 | Out-String
    $logR4A = if ([IO.File]::Exists((Join-Path $temp 'discord_launcher.log'))) {{ [IO.File]::ReadAllText((Join-Path $temp 'discord_launcher.log')) }} else {{ '' }}
    if ($LASTEXITCODE -ne 0 -or $fencedR4A -notmatch 'disabled by marker' -or $logR4A -notmatch 'direct_launch_disabled marker=') {{
        throw "Baseline codex-discord-bot.cmd was not fenced at R4 OnBeforeCommitWrite boundary: out=$fencedR4A log=$logR4A"
    }}

    # R4 Fault Injection B: Resume from cutover_failed_hold, simulate post-commit interruption before fence cleanup (-OnAfterCommitBeforeFenceRelease)
    # -> mcp-ownership-state.json MUST already be committed=true on disk while fences remain, and a subsequent Invoke-CdrMcpOwnershipCutover
    # must resume idempotently (ResumedFromCommitted=true) and remove the leftover fences without spawning a second external MCP!
    $postCommitInterrupted = $false
    try {{
        Invoke-CdrMcpOwnershipCutover `
            -RepoRoot $temp `
            -McpBinaryPath $mcpBin `
            -BotBinaryPath $psExe `
            -EmbeddedExitTimeoutMs 3000 `
            -ReadinessTimeoutMs 8000 `
            -OnAfterCommitBeforeFenceRelease {{ throw 'injected_post_commit_interruption' }} | Out-Null
    }} catch {{
        if ($_.Exception.Message -match 'injected_post_commit_interruption') {{
            $postCommitInterrupted = $true
        }} else {{
            throw
        }}
    }}
    $afterPostCommitState = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    if (-not $postCommitInterrupted -or
        $afterPostCommitState.state -cne 'committed' -or
        -not [bool]$afterPostCommitState.committed -or
        -not [IO.File]::Exists((Join-Path $temp '.codex_discord_runtime.cutover')) -or
        -not [IO.File]::Exists((Join-Path $temp '.codex_discord_bot.disabled'))) {{
        throw "R4 OnAfterCommitBeforeFenceRelease did not leave committed=true state prior to fence removal: $($afterPostCommitState | ConvertTo-Json -Compress)"
    }}
    # Execute exact baseline 56ab736f42b23d1a5359d0e6714f97f22179b90a codex-discord-bot.cmd at R4 failure boundary B: must remain fenced until cleanup completes!
    Remove-Item -LiteralPath (Join-Path $temp 'discord_launcher.log') -Force -ErrorAction SilentlyContinue
    $fencedR4B = & cmd.exe /d /c "`"$oldCmdPath`"" 2>&1 | Out-String
    $logR4B = if ([IO.File]::Exists((Join-Path $temp 'discord_launcher.log'))) {{ [IO.File]::ReadAllText((Join-Path $temp 'discord_launcher.log')) }} else {{ '' }}
    if ($LASTEXITCODE -ne 0 -or $fencedR4B -notmatch 'disabled by marker' -or $logR4B -notmatch 'direct_launch_disabled marker=') {{
        throw "Baseline codex-discord-bot.cmd was not fenced at R4 OnAfterCommitBeforeFenceRelease boundary: out=$fencedR4B log=$logR4B"
    }}
    $committedIdentityBeforeResume = [string]$afterPostCommitState.owner_identity

    # Resume Invoke-CdrMcpOwnershipCutover after post-commit interruption: must finish fence cleanup idempotently without re-spawning!
    $cutover = Invoke-CdrMcpOwnershipCutover `
        -RepoRoot $temp `
        -McpBinaryPath $mcpBin `
        -BotBinaryPath $psExe `
        -EmbeddedExitTimeoutMs 3000 `
        -ReadinessTimeoutMs 8000
    $mcpProc = $cutover.Process
    if (-not $cutover.Committed -or
        -not $cutover.ResumedFromCommitted -or
        $cutover.Mode -ne 'external' -or
        $cutover.ProcessIdentity -cne $committedIdentityBeforeResume -or
        [IO.File]::Exists((Join-Path $temp '.codex_discord_runtime.cutover')) -or
        [IO.File]::Exists((Join-Path $temp '.codex_discord_bot.disabled'))) {{
        throw "R4 idempotent resume from committed state failed: $($cutover | ConvertTo-Json -Compress)"
    }}
    if (-not [IO.File]::Exists((Join-Path $temp 'bot_data\disabled_fence_observed.txt')) -or
        ([IO.File]::ReadAllText((Join-Path $temp 'bot_data\disabled_fence_observed.txt')).Trim() -cne 'mcp_cutover_fence')) {{
        throw 'Invoke-CdrMcpOwnershipCutover must fence old CMD/helpers with .codex_discord_bot.disabled (mcp_cutover_fence) during handoff!'
    }}
    if (Test-CdrProcessMatchesIdentity -ProcessId $embeddedProc.Id -ExpectedIdentity $embeddedIdentity -ExpectedBinaryPath $psExe) {{
        throw 'Embedded owner must have exited before external cutover committed!'
    }}
    $stateJson = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    if ($stateJson.schema_version -ne 1 -or $stateJson.mode -ne 'external' -or $stateJson.state -ne 'committed' -or -not [bool]$stateJson.committed -or $stateJson.embedded_drain_runtime_id -cne 'healthy-emb-rt-1') {{
        throw "Ownership state file not committed to external with embedded_drain_runtime_id: $($stateJson | ConvertTo-Json -Compress)"
    }}

    # Case 3: Manual rollback to embedded MUST fail closed while external MCP is still running if -StopExternalFirst is not used
    $rollbackRefused = $false
    try {{
        Invoke-CdrMcpRollbackToEmbedded -RepoRoot $temp -McpBinaryPath $mcpBin | Out-Null
    }} catch {{
        $rollbackRefused = $true
    }}
    if (-not $rollbackRefused) {{
        throw 'Rollback to embedded should be refused while external MCP is still alive without -StopExternalFirst'
    }}

    # Case 4: Manual rollback WITH -StopExternalFirst fences MCP intent to stopped first, stops real external MCP gracefully, and switches mode to embedded
    $rollbackOk = Invoke-CdrMcpRollbackToEmbedded -RepoRoot $temp -McpBinaryPath $mcpBin -StopExternalFirst -GraceTimeoutMs 3000
    if (-not $rollbackOk.RolledBack -or $rollbackOk.Mode -ne 'embedded') {{
        throw "Expected clean rollback to embedded, got: $($rollbackOk | ConvertTo-Json -Compress)"
    }}
    if ((Get-CdrComponentIntent -RepoRoot $temp -Component 'mcp').DesiredState -ne 'stopped') {{
        throw 'Rollback to embedded must leave external MCP intent fenced to stopped'
    }}
    if (Test-CdrProcessMatchesIdentity -ProcessId $cutover.ProcessId -ExpectedIdentity $cutover.ProcessIdentity -ExpectedBinaryPath $mcpBin) {{
        throw 'External MCP process must be dead before embedded mode is restored'
    }}
    $afterState = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    if ($afterState.schema_version -ne 1 -or $afterState.mode -ne 'embedded' -or $afterState.state -ne 'committed' -or -not [bool]$afterState.committed) {{
        throw 'Ownership state file was not updated to committed embedded after verified rollback'
    }}
}} finally {{
    foreach ($p in @($embeddedProc, $mcpProc)) {{
        if ($null -ne $p) {{
            $id = Get-CdrProcessIdentity -Process $p
            if ($id) {{ Stop-CdrVerifiedProcess -ProcessId $p.Id -ExpectedIdentity $id | Out-Null }}
        }}
    }}
}}

# Case 5 (R1 + T11): Real cdr-runtime.exe embedded owner -> Invoke-CdrMcpOwnershipCutover -> real external cdr-remote-agent.exe
# Verify that the real embedded cdr-runtime.exe drains via .codex_discord_rust.drain.prepare + .codex_discord_rust.restart,
# writes bot_data/remote-mcp-restart.json, exits cleanly, and the newly launched external cdr-remote-agent.exe claims and restores
# the project binding (thread-t11 -> root) so a fresh ProjectSession activation (without working_directory) + TerminalExec succeeds!
Remove-Item -LiteralPath $drainIdPath -Force -ErrorAction SilentlyContinue
& '{soak_bin_env}' init-state-db (Join-Path $temp 'codex_home\state_5.sqlite')
if ($LASTEXITCODE -ne 0) {{ throw 'init-state-db failed in t11' }}
$env:CDR_DISCORD_HTTP_PROXY = '{discord_http}'
$env:CDR_DISCORD_GATEWAY_PROXY_URL = 'ws://{discord_ws}'
[IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=Bot.test.token`nDISCORD_ALLOWED_CHANNEL_IDS=42`nCDR_DISCORD_HTTP_PROXY={discord_http}`nCDR_DISCORD_GATEWAY_PROXY_URL=ws://{discord_ws}`nCODEX_HOME={codex_home_env}`nCODEX_EXE={soak_bin_env}`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_OWNERSHIP=embedded`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://{bridge_addr}`nCODEX_REMOTE_MCP_DEVICE_ID=device-t11-real`nCODEX_REMOTE_MCP_DEVICE_TOKEN=secret-t11`n")
Set-CdrComponentIntent -RepoRoot $temp -Component 'bot' -DesiredState 'running' -Action 'start-real-embedded' -Source 'test' | Out-Null
$realEmbeddedLaunch = Invoke-CdrJournaledProcessLaunch `
    -RepoRoot $temp -Component 'bot' -FilePath $botBin `
    -ArgumentList @('--env', (Join-Path $temp '.env')) `
    -WorkingDirectory $temp -TargetBinaryPath $botBin -Action 'start-real-embedded' -AllowClearManualStop

# Wait for the real cdr-runtime.exe to publish .codex_discord_rust.drain.identity (state=open) AND bind the project on the bridge
$boundAckPath = Join-Path $temp 'bot_data\embedded_bound_ack.txt'
$embReadyDeadline = [DateTime]::UtcNow.AddSeconds(12)
$realDrainParsed = $null
while ([DateTime]::UtcNow -lt $embReadyDeadline) {{
    if ([IO.File]::Exists($drainIdPath) -and [IO.File]::Exists($boundAckPath)) {{
        try {{
            $parsed = Read-CdrHealthyDrainIdentity -Path $drainIdPath
            if ($null -ne $parsed -and $parsed.State -ceq 'open' -and $parsed.Pid -eq $realEmbeddedLaunch.ProcessId) {{
                $realDrainParsed = $parsed
                break
            }}
        }} catch {{ }}
    }}
    Start-Sleep -Milliseconds 50
}}
if ($null -eq $realDrainParsed) {{
    Stop-CdrComponentRuntime -RepoRoot $temp -Component 'bot' -BinaryPath $botBin -GraceTimeoutMs 1000 -AllowForceKill | Out-Null
    throw 'Real embedded cdr-runtime.exe did not publish open drain.identity and bind project session within 12s'
}}

# Switch .env to external ownership and run Invoke-CdrMcpOwnershipCutover against the real embedded cdr-runtime.exe!
[IO.File]::WriteAllText((Join-Path $temp '.env'), "DISCORD_BOT_TOKEN=Bot.test.token`nDISCORD_ALLOWED_CHANNEL_IDS=42`nCDR_DISCORD_HTTP_PROXY={discord_http}`nCDR_DISCORD_GATEWAY_PROXY_URL=ws://{discord_ws}`nCODEX_HOME={codex_home_env}`nCODEX_EXE={soak_bin_env}`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_OWNERSHIP=external`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://{bridge_addr}`nCODEX_REMOTE_MCP_DEVICE_ID=device-t11-real`nCODEX_REMOTE_MCP_DEVICE_TOKEN=secret-t11`n")
Set-CdrComponentIntent -RepoRoot $temp -Component 'mcp' -DesiredState 'running' -Action 'real-cutover' -Source 'test' | Out-Null
$script:f1RealCutoverStages = New-Object System.Collections.Generic.List[string]
$realCutover = Invoke-CdrMcpOwnershipCutover `
    -RepoRoot $temp `
    -McpBinaryPath $mcpBin `
    -BotBinaryPath $botBin `
    -ExpectedEmbeddedIdentity $realEmbeddedLaunch.ProcessIdentity `
    -EmbeddedExitTimeoutMs 15000 `
    -ReadinessTimeoutMs 10000 `
    -OnBeforeDrainMarkerPublish {{
        param($targetPath, $tempPath, $text)
        $writerCmd = Get-Command Write-AtomicRestartMarker -ErrorAction Stop
        $callerFrame = (Get-PSCallStack)[1]
        if ([IO.Path]::GetFileName([string]$writerCmd.ScriptBlock.File) -ine 'codex-discord-rust-drain.ps1' -or $callerFrame.Command -cne 'Write-AtomicRestartMarker') {{
            throw "T11 F1 expected canonical Write-AtomicRestartMarker in codex-discord-rust-drain.ps1, got file=$($writerCmd.ScriptBlock.File) caller=$($callerFrame.Command)"
        }}
        $leaf = [IO.Path]::GetFileName($targetPath)
        if (-not [IO.File]::Exists($tempPath)) {{
            throw "F1 staging temp file missing for $leaf"
        }}
        if ([IO.File]::Exists($targetPath)) {{
            throw "F1 final marker $leaf must not exist before atomic Move/Replace"
        }}
        if ($leaf -ceq '.codex_discord_rust.restart') {{
            if ([IO.File]::Exists((Join-Path $temp '.codex_discord_rust.drain.prepare'))) {{
                throw 'F1 drain.prepare must not exist while staging restart marker'
            }}
            $script:f1RealCutoverStages.Add('restart_staged')
        }} elseif ($leaf -ceq '.codex_discord_rust.drain.prepare') {{
            $restartPublished = Join-Path $temp '.codex_discord_rust.restart'
            if (-not [IO.File]::Exists($restartPublished) -or ([IO.File]::ReadAllText($restartPublished, [Text.UTF8Encoding]::UTF8) -cne $text)) {{
                throw 'F1 completed .codex_discord_rust.restart must be published before .codex_discord_rust.drain.prepare'
            }}
            # Hold staging across >1 gateway_loop 250ms poll tick so the live embedded cdr-runtime.exe polls while prepare is staged
            Start-Sleep -Milliseconds 300
            $script:f1RealCutoverStages.Add('prepare_staged_after_restart')
        }}
    }}
if (($script:f1RealCutoverStages -join ',') -cne 'restart_staged,prepare_staged_after_restart') {{
    throw "Expected F1 atomic staging order restart_staged,prepare_staged_after_restart during real T11 cutover, got: $($script:f1RealCutoverStages -join ',')"
}}
if (-not $realCutover.Committed -or $realCutover.Mode -ne 'external') {{
    throw "Real embedded-to-external cutover did not commit: $($realCutover | ConvertTo-Json -Compress)"
}}
if (Test-CdrProcessMatchesIdentity -ProcessId $realEmbeddedLaunch.ProcessId -ExpectedIdentity $realEmbeddedLaunch.ProcessIdentity -ExpectedBinaryPath $botBin) {{
    throw 'Real embedded cdr-runtime.exe must have exited cleanly during cutover!'
}}
if ([IO.File]::Exists((Join-Path $temp 'bot_data\remote-mcp-restart.json'))) {{
    throw 'Expected external cdr-remote-agent.exe to claim and remove bot_data/remote-mcp-restart.json on startup!'
}}
"#,
        journal = journal_script.display(),
        temp = temp.path().display(),
        baseline_cmd_fixture = baseline_cmd_fixture.display(),
        discord_http = discord_double.http_addr,
        discord_ws = discord_double.ws_addr,
        codex_home_env = codex_home_env,
        soak_bin_env = soak_bin_env,
        bridge_addr = bridge_addr,
        bot_bin = bot_bin.display(),
        mcp_bin = mcp_bin.display()
    );
    run_pwsh(&script);

    // Verify that the external cdr-remote-agent.exe launched by Invoke-CdrMcpOwnershipCutover
    // restored the project binding (thread-t11 -> root) from the real embedded cdr-runtime.exe's remote-mcp-restart.json,
    // preserves the session-generation fence by requiring a fresh ProjectSession activation (without working_directory),
    // and executes a TerminalExec command on that freshly activated session!
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    cmd_req_tx
        .send(("after-real-embedded-cutover".into(), reply_tx))
        .expect("send terminal exec to cutover external mcp");
    let ws_reply = tokio::time::timeout(Duration::from_secs(8), reply_rx)
        .await
        .expect("cutover external mcp terminal exec timeout")
        .expect("cutover external mcp terminal exec reply");
    assert!(
        ws_reply.contains("CUTOVER_HANDOFF_RESTORED_OK:after-real-embedded-cutover"),
        "Expected external cdr-remote-agent.exe to restore remote-mcp-restart.json project binding from real embedded cdr-runtime.exe and execute TerminalExec after fresh ProjectSession activation, got: {ws_reply}"
    );

    let stop_mcp_script = format!(
        ". '{}'; Stop-CdrComponentRuntime -RepoRoot '{}' -Component 'mcp' -BinaryPath '{}' -GraceTimeoutMs 3000 -Source 't11-cleanup' -AllowForceKill | Out-Null; $soakPath = [IO.Path]::GetFullPath('{}'); Get-Process -Name 'cdr-offline-soak' -ErrorAction SilentlyContinue | ForEach-Object {{ try {{ if ($_.Path -and [IO.Path]::GetFullPath($_.Path) -ieq $soakPath) {{ $soakId = Get-CdrProcessIdentity -Process $_; if (-not [string]::IsNullOrWhiteSpace($soakId)) {{ Stop-CdrVerifiedProcess -ProcessId $_.Id -ExpectedIdentity $soakId -ExpectedBinaryPath $soakPath | Out-Null }} }} }} catch {{ }} }}",
        journal_script.display(),
        temp.path().display(),
        temp.path()
            .join("target")
            .join("release")
            .join("cdr-remote-agent.exe")
            .display(),
        local_soak_t03.display()
    );
    run_pwsh(&stop_mcp_script);
    drop(cmd_req_tx);
    bridge_task.abort();
    discord_double.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f1_atomic_cutover_drain_marker_publication_prevents_partial_read_quarantine() {
    use cdr_runtime::restart_readiness::drain::DrainGateError;
    use cdr_runtime::restart_readiness::drain_controller::{
        DrainProtocolError, DrainedTransition, RuntimeDrainController,
    };

    let ws = repo_root();
    let journal_script = ws.join("scripts").join("CdrLaunchJournal.ps1");
    let temp = tempfile::tempdir().expect("create f1 temp dir");

    // Obtain the current test process's exact <pid>|<dotnet_ticks> identity via CdrLaunchJournal.ps1
    let self_pid = std::process::id();
    let id_out = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &format!(
                ". '{}'; [Console]::Out.Write((Get-CdrProcessIdentity -ProcessId {}))",
                journal_script.display(),
                self_pid
            ),
        ])
        .output()
        .expect("query current process identity");
    assert!(id_out.status.success(), "Get-CdrProcessIdentity failed");
    let self_identity = String::from_utf8_lossy(&id_out.stdout).trim().to_string();
    assert!(
        self_identity.starts_with(&format!("{self_pid}|")),
        "unexpected self_identity: {self_identity}"
    );

    // 1. Verify original fail-closed runtime quarantine behavior remains intact when a partial/incomplete
    // .codex_discord_rust.drain.prepare is visible at the final path (as could happen before F1 atomic publication).
    let quarantine_dir = temp.path().join("quarantine_case");
    fs::create_dir_all(&quarantine_dir).expect("create quarantine_dir");
    let quarantine_ctrl =
        RuntimeDrainController::initialize(&quarantine_dir).expect("init quarantine controller");
    let partial_prepare = quarantine_dir.join(".codex_discord_rust.drain.prepare");
    fs::write(&partial_prepare, "version=1\nruntime_id=").expect("write partial prepare");
    let claim_err = quarantine_ctrl
        .claim_prepare()
        .expect_err("partial prepare must fail claim_prepare");
    assert!(
        matches!(claim_err, DrainProtocolError::Marker(_)),
        "expected DrainProtocolError::Marker on partial prepare, got: {claim_err:?}"
    );
    quarantine_ctrl
        .quarantine_untrusted_prepare()
        .expect("quarantine_untrusted_prepare must seal gate");
    assert!(
        quarantine_ctrl.admission_gate().is_sealed(),
        "admission gate must be sealed after quarantine_untrusted_prepare"
    );
    assert_eq!(
        quarantine_ctrl.admission_gate().try_enter().unwrap_err(),
        DrainGateError::Sealed
    );
    let late_valid_fence = format!(
        "version=1\nruntime_id={}\nprocess_identity={self_identity}\nnonce=cutover-late\n",
        quarantine_ctrl.runtime_id()
    );
    fs::write(&partial_prepare, &late_valid_fence).expect("write late valid prepare");
    let second_err = quarantine_ctrl
        .claim_prepare()
        .expect_err("quarantined gate must reject subsequent valid nonce with FenceMismatch");
    assert!(
        matches!(
            second_err,
            DrainProtocolError::Gate(DrainGateError::FenceMismatch)
        ),
        "expected FenceMismatch after untrusted-prepare quarantine, got: {second_err:?}"
    );

    // 2. Verify failure before publish in Invoke-CdrMcpOwnershipCutover (-OnBeforeDrainMarkerPublish throwing
    // while staging .codex_discord_rust.drain.prepare.tmp.<guid>) never exposes a final .codex_discord_rust.drain.prepare,
    // cleans up both the temp file and .codex_discord_rust.restart, and leaves RuntimeDrainController unsealed.
    let fail_dir = temp.path().join("fail_before_publish_case");
    fs::create_dir_all(fail_dir.join("bot_data")).expect("create fail_dir");
    let fail_ctrl = RuntimeDrainController::initialize(&fail_dir).expect("init fail controller");
    let current_exe = std::env::current_exe().expect("current test exe");
    let fail_ps = format!(
        r#"
$ErrorActionPreference = 'Stop'
. '{journal}'
$root = '{fail_dir}'
$fakeMcp = Join-Path $root 'bin\win-x64\cdr-remote-agent.exe'
New-Item -ItemType Directory -Path (Split-Path -Parent $fakeMcp) -Force | Out-Null
[IO.File]::WriteAllText($fakeMcp, 'stub-mcp')
[IO.File]::WriteAllText((Join-Path $root '.env'), "DISCORD_BOT_TOKEN=dummy`nDISCORD_ALLOWED_CHANNEL_IDS=1`nCODEX_REMOTE_MCP_ENABLED=1`nCODEX_REMOTE_MCP_OWNERSHIP=external`nCODEX_REMOTE_MCP_BRIDGE_URL=ws://127.0.0.1:9/unused`nCODEX_REMOTE_MCP_DEVICE_ID=dev-f1`nCODEX_REMOTE_MCP_DEVICE_TOKEN=tok-f1`n")
$threw = $false
try {{
    Invoke-CdrMcpOwnershipCutover `
        -RepoRoot $root `
        -McpBinaryPath $fakeMcp `
        -BotBinaryPath '{current_exe}' `
        -ExpectedEmbeddedIdentity '{self_identity}' `
        -EmbeddedExitTimeoutMs 2000 `
        -ReadinessTimeoutMs 1000 `
        -OnBeforeDrainMarkerPublish {{
            param($targetPath, $tempPath, $text)
            if ([IO.Path]::GetFileName($targetPath) -ceq '.codex_discord_rust.drain.prepare') {{
                if (-not [IO.File]::Exists((Join-Path $root '.codex_discord_rust.restart'))) {{
                    throw 'restart marker was not published before prepare staging'
                }}
                if ([IO.File]::Exists($targetPath)) {{
                    throw 'final drain.prepare must not exist during staging'
                }}
                if (-not [IO.File]::Exists($tempPath)) {{
                    throw 'temp drain.prepare file must exist during staging'
                }}
                throw 'injected_prepare_publish_failure'
            }}
        }} | Out-Null
}} catch {{
    if ($_.Exception.Message -match 'injected_prepare_publish_failure') {{
        $threw = $true
    }} else {{
        throw
    }}
}}
if (-not $threw) {{ throw 'Expected Invoke-CdrMcpOwnershipCutover to throw injected_prepare_publish_failure' }}
$st = Get-Content -LiteralPath (Join-Path $root '.cdr-maintenance\mcp-ownership-state.json') -Raw | ConvertFrom-Json
if ($st.state -cne 'cutover_failed_hold' -or [bool]$st.committed -or [bool]$st.embedded_owner_exited -or $st.reason -notmatch '^embedded_drain_publish_failed:injected_prepare_publish_failure') {{
    throw "Unexpected ownership state after pre-publish failure: $($st | ConvertTo-Json -Compress)"
}}
"#,
        journal = journal_script.display(),
        fail_dir = fail_dir.display(),
        current_exe = current_exe.display(),
        self_identity = self_identity
    );
    run_pwsh(&fail_ps);
    assert!(
        !fail_dir.join(".codex_discord_rust.drain.prepare").exists(),
        "final drain.prepare must not exist after failure before publish"
    );
    assert!(
        !fail_dir.join(".codex_discord_rust.restart").exists(),
        "restart marker must be cleaned up after failure before prepare publish"
    );
    let leftover_tmps: Vec<_> = fs::read_dir(&fail_dir)
        .expect("read fail_dir")
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
        .collect();
    assert!(
        leftover_tmps.is_empty(),
        "temporary marker files must be cleaned up on failure: {leftover_tmps:?}"
    );
    assert!(
        fail_ctrl
            .claim_prepare()
            .expect("claim_prepare after failed publish")
            .is_none(),
        "RuntimeDrainController must observe no prepare marker after failed publish"
    );
    assert!(
        !fail_ctrl.admission_gate().is_sealed() && fail_ctrl.admission_gate().try_enter().is_ok(),
        "AdmissionGate must remain open and unquarantined when publish fails before Move/Replace"
    );

    // 3. Deterministic 2-stage publication boundary test controlling RuntimeDrainController reader
    // across (Stage 1) staging .codex_discord_rust.restart.tmp.*, (Stage 2) after .codex_discord_rust.restart
    // is published while staging .codex_discord_rust.drain.prepare.tmp.*, and (Stage 3) after atomic
    // publication of .codex_discord_rust.drain.prepare completes.
    // Exercises BOTH entrypoint modes ("direct_journal" where only CdrLaunchJournal.ps1 is loaded initially,
    // and "preloaded_drain" where codex-discord-rust-drain.ps1 is already loaded before CdrLaunchJournal.ps1)
    // and proves both execute the single canonical Write-AtomicRestartMarker in codex-discord-rust-drain.ps1.
    let drain_script = ws.join("codex-discord-rust-drain.ps1");
    for (mode_idx, entrypoint_mode) in ["direct_journal", "preloaded_drain"]
        .into_iter()
        .enumerate()
    {
        let sync_dir = temp
            .path()
            .join(format!("sync_publication_{entrypoint_mode}"));
        fs::create_dir_all(&sync_dir).expect("create sync_dir");
        let sync_ctrl =
            RuntimeDrainController::initialize(&sync_dir).expect("init sync controller");
        let expected_nonce = format!("cutover-f1-deterministic-nonce-{mode_idx}");
        let expected_fence_text = format!(
            "version=1\nruntime_id={}\nprocess_identity={self_identity}\nnonce={expected_nonce}\n",
            sync_ctrl.runtime_id()
        );

        let stage1_ready = sync_dir.join("stage1_ready.txt");
        let stage1_continue = sync_dir.join("stage1_continue.txt");
        let stage2_ready = sync_dir.join("stage2_ready.txt");
        let stage2_continue = sync_dir.join("stage2_continue.txt");

        let preload_line = if entrypoint_mode == "preloaded_drain" {
            format!(". '{}'", drain_script.display())
        } else {
            String::new()
        };

        let writer_ps = format!(
            r"
$ErrorActionPreference = 'Stop'
{preload_line}
. '{journal}'
if ('{entrypoint_mode}' -ceq 'direct_journal') {{
    if ($null -ne (Get-Command Write-AtomicRestartMarker -ErrorAction SilentlyContinue)) {{
        throw 'CdrLaunchJournal.ps1 must not define or import Write-AtomicRestartMarker at top level before cutover publication'
    }}
}}
$root = '{sync_dir}'
$expectedDrainFile = [IO.Path]::GetFullPath('{drain_script}')
Publish-CdrCutoverDrainFenceMarkers `
    -RestartMarkerPath (Join-Path $root '.codex_discord_rust.restart') `
    -DrainPreparePath (Join-Path $root '.codex_discord_rust.drain.prepare') `
    -RuntimeId '{runtime_id}' `
    -ProcessIdentity '{self_identity}' `
    -Nonce '{expected_nonce}' `
    -OnBeforeMarkerPublish {{
        param($targetPath, $tempPath, $text)
        $writerCmd = Get-Command Write-AtomicRestartMarker -ErrorAction Stop
        $writerFile = [IO.Path]::GetFullPath([string]$writerCmd.ScriptBlock.File)
        $callerFrame = (Get-PSCallStack)[1]
        $callerFile = if ($callerFrame.ScriptName) {{ [IO.Path]::GetFullPath([string]$callerFrame.ScriptName) }} else {{ '' }}
        if ($writerFile -ine $expectedDrainFile -or $callerFrame.Command -cne 'Write-AtomicRestartMarker' -or $callerFile -ine $expectedDrainFile) {{
            throw ('Expected canonical Write-AtomicRestartMarker in ' + $expectedDrainFile + ', got writerFile=' + $writerFile + ' callerCmd=' + $callerFrame.Command + ' callerFile=' + $callerFile)
        }}
        $leaf = [IO.Path]::GetFileName($targetPath)
        if ($leaf -ceq '.codex_discord_rust.restart') {{
            [IO.File]::WriteAllText((Join-Path $root 'stage1_ready.txt'), $tempPath, [Text.UTF8Encoding]::new($false))
            $deadline = [DateTime]::UtcNow.AddSeconds(10)
            while (-not [IO.File]::Exists((Join-Path $root 'stage1_continue.txt')) -and [DateTime]::UtcNow -lt $deadline) {{
                Start-Sleep -Milliseconds 15
            }}
            if (-not [IO.File]::Exists((Join-Path $root 'stage1_continue.txt'))) {{
                throw 'Timed out waiting for stage1_continue.txt'
            }}
        }} elseif ($leaf -ceq '.codex_discord_rust.drain.prepare') {{
            [IO.File]::WriteAllText((Join-Path $root 'stage2_ready.txt'), $tempPath, [Text.UTF8Encoding]::new($false))
            $deadline = [DateTime]::UtcNow.AddSeconds(10)
            while (-not [IO.File]::Exists((Join-Path $root 'stage2_continue.txt')) -and [DateTime]::UtcNow -lt $deadline) {{
                Start-Sleep -Milliseconds 15
            }}
            if (-not [IO.File]::Exists((Join-Path $root 'stage2_continue.txt'))) {{
                throw 'Timed out waiting for stage2_continue.txt'
            }}
        }}
    }}
",
            preload_line = preload_line,
            entrypoint_mode = entrypoint_mode,
            journal = journal_script.display(),
            drain_script = drain_script.display(),
            sync_dir = sync_dir.display(),
            runtime_id = sync_ctrl.runtime_id(),
            self_identity = self_identity,
            expected_nonce = expected_nonce
        );

        let mut writer_child = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &writer_ps,
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn F1 PowerShell writer");

        // Wait for Stage 1 (staging .codex_discord_rust.restart.tmp.*)
        let s1_deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !stage1_ready.exists() {
            if let Some(status) = writer_child.try_wait().expect("poll writer child") {
                let out = writer_child
                    .wait_with_output()
                    .expect("writer child output");
                panic!(
                    "F1 writer ({entrypoint_mode}) exited early before stage1_ready ({status:?}):\nSTDOUT:\n{}\nSTDERR:\n{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            assert!(
                std::time::Instant::now() < s1_deadline,
                "timed out waiting for stage1_ready.txt ({entrypoint_mode})"
            );
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        let stage1_tmp_path = PathBuf::from(
            fs::read_to_string(&stage1_ready)
                .expect("read stage1_ready")
                .trim(),
        );
        assert!(
            stage1_tmp_path.exists(),
            "stage1 temporary restart marker must exist during staging ({entrypoint_mode})"
        );
        assert_eq!(
            fs::read_to_string(&stage1_tmp_path).expect("read stage1 temp"),
            expected_fence_text
        );
        assert!(
            !sync_dir.join(".codex_discord_rust.restart").exists(),
            "final .codex_discord_rust.restart must not exist while staging temp ({entrypoint_mode})"
        );
        assert!(
            !sync_dir.join(".codex_discord_rust.drain.prepare").exists(),
            "final .codex_discord_rust.drain.prepare must not exist during stage 1 ({entrypoint_mode})"
        );
        assert!(
            sync_ctrl
                .claim_prepare()
                .expect("claim_prepare at stage 1")
                .is_none(),
            "reader must observe no prepare marker at stage 1 ({entrypoint_mode})"
        );
        assert!(
            !sync_ctrl.admission_gate().is_sealed()
                && sync_ctrl.admission_gate().try_enter().is_ok(),
            "admission gate must remain open at stage 1 ({entrypoint_mode})"
        );
        fs::write(&stage1_continue, "ok").expect("signal stage1_continue");

        // Wait for Stage 2 (after .codex_discord_rust.restart is published, while staging .codex_discord_rust.drain.prepare.tmp.*)
        let s2_deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !stage2_ready.exists() {
            if let Some(status) = writer_child.try_wait().expect("poll writer child") {
                let out = writer_child
                    .wait_with_output()
                    .expect("writer child output");
                panic!(
                    "F1 writer ({entrypoint_mode}) exited early before stage2_ready ({status:?}):\nSTDOUT:\n{}\nSTDERR:\n{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            assert!(
                std::time::Instant::now() < s2_deadline,
                "timed out waiting for stage2_ready.txt ({entrypoint_mode})"
            );
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        let stage2_tmp_path = PathBuf::from(
            fs::read_to_string(&stage2_ready)
                .expect("read stage2_ready")
                .trim(),
        );
        assert!(
            !stage1_tmp_path.exists(),
            "stage1 temporary restart marker must be moved/removed once restart is published ({entrypoint_mode})"
        );
        assert_eq!(
            fs::read_to_string(sync_dir.join(".codex_discord_rust.restart"))
                .expect("read published restart marker at stage 2"),
            expected_fence_text,
            "completed .codex_discord_rust.restart must be published before .codex_discord_rust.drain.prepare ({entrypoint_mode})"
        );
        assert!(
            stage2_tmp_path.exists(),
            "stage2 temporary drain.prepare marker must exist during staging ({entrypoint_mode})"
        );
        assert_eq!(
            fs::read_to_string(&stage2_tmp_path).expect("read stage2 temp"),
            expected_fence_text
        );
        assert!(
            !sync_dir.join(".codex_discord_rust.drain.prepare").exists(),
            "final .codex_discord_rust.drain.prepare must not exist while staging temp ({entrypoint_mode})"
        );
        assert!(
            sync_ctrl
                .claim_prepare()
                .expect("claim_prepare at stage 2")
                .is_none(),
            "reader must observe no final prepare marker while prepare temp is staged ({entrypoint_mode})"
        );
        assert!(
            !sync_ctrl.admission_gate().is_sealed()
                && sync_ctrl.admission_gate().try_enter().is_ok(),
            "admission gate must remain open at stage 2 ({entrypoint_mode})"
        );
        fs::write(&stage2_continue, "ok").expect("signal stage2_continue");

        // Stage 3: Wait for writer completion and verify reader observes the complete valid fence, claims real nonce, and completes handshake
        let writer_out = writer_child
            .wait_with_output()
            .expect("wait for F1 writer completion");
        assert!(
            writer_out.status.success(),
            "F1 writer ({entrypoint_mode}) failed:\nSTDOUT:\n{}\nSTDERR:\n{}",
            String::from_utf8_lossy(&writer_out.stdout),
            String::from_utf8_lossy(&writer_out.stderr)
        );
        assert!(
            !stage2_tmp_path.exists(),
            "stage2 temporary drain.prepare marker must be moved/removed after publication ({entrypoint_mode})"
        );
        assert_eq!(
            fs::read_to_string(sync_dir.join(".codex_discord_rust.drain.prepare"))
                .expect("read published drain.prepare"),
            expected_fence_text
        );
        let claimed_key = sync_ctrl
            .claim_prepare()
            .expect("claim_prepare after atomic publication")
            .expect("prepare marker must be present after publication");
        assert_eq!(claimed_key.runtime_id(), sync_ctrl.runtime_id());
        assert_eq!(claimed_key.process_identity(), self_identity);
        assert_eq!(claimed_key.nonce(), expected_nonce);
        assert!(
            sync_ctrl.admission_gate().is_sealed(),
            "admission gate must be sealed with real nonce after claim_prepare ({entrypoint_mode})"
        );
        sync_ctrl
            .acknowledge(&claimed_key)
            .expect("acknowledge claimed drain fence");
        let transition = sync_ctrl.complete_drained_handshake(&claimed_key).await;
        assert_eq!(transition, DrainedTransition::Restart);
    }
}
