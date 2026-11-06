#![cfg(windows)]
#![allow(clippy::too_many_lines)]

use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use cdr_windows_native::{CapturedWindowProcess, query_live_process_metadata};
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
                    ready_conn.store(false, Ordering::SeqCst);
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

fn ensure_runtime_bins() -> (PathBuf, PathBuf) {
    static BUILT_RUNTIME_BINS: std::sync::OnceLock<(PathBuf, PathBuf)> = std::sync::OnceLock::new();
    BUILT_RUNTIME_BINS
        .get_or_init(|| {
            let mcp_bin = PathBuf::from(env!("CARGO_BIN_EXE_cdr-remote-agent"));
            let runtime_bin = mcp_bin.with_file_name("cdr-runtime.exe");
            let soak_bin = mcp_bin.with_file_name("cdr-offline-soak.exe");
            if !runtime_bin.exists() || !soak_bin.exists() {
                let status = Command::new("cargo")
                    .args([
                        "build",
                        "--locked",
                        "-p",
                        "cdr-runtime",
                        "--bin",
                        "cdr-runtime",
                        "--bin",
                        "cdr-offline-soak",
                    ])
                    .current_dir(repo_root())
                    .status()
                    .expect("build cdr-runtime bins");
                assert!(status.success(), "cargo build cdr-runtime bins failed");
            }
            assert!(
                runtime_bin.exists(),
                "cdr-runtime.exe missing at {}",
                runtime_bin.display()
            );
            assert!(
                soak_bin.exists(),
                "cdr-offline-soak.exe missing at {}",
                soak_bin.display()
            );
            (runtime_bin, soak_bin)
        })
        .clone()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn copy_control_fixture(root: &Path) -> PathBuf {
    let ws = repo_root();
    let scripts_dir = root.join("scripts");
    let bin_dir = root.join("bin").join("win-x64");
    let bot_data = root.join("bot_data");
    fs::create_dir_all(&scripts_dir).expect("create scripts");
    fs::create_dir_all(&bin_dir).expect("create bin");
    fs::create_dir_all(&bot_data).expect("create bot_data");

    for file in [
        "codex-discord-rust-control.ps1",
        "codex-discord-rust-watchdog.ps1",
        "codex-discord-rust-restart.ps1",
        "codex-discord-rust-drain.ps1",
    ] {
        fs::copy(ws.join(file), root.join(file)).expect("copy root script");
    }
    for entry in fs::read_dir(ws.join("scripts")).expect("read scripts") {
        let entry = entry.expect("entry");
        if entry.path().is_file() {
            fs::copy(entry.path(), scripts_dir.join(entry.file_name())).expect("copy script");
        }
    }

    let ps_exe =
        PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()))
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    fs::write(
        root.join(".env"),
        "DISCORD_BOT_TOKEN=dummy\nDISCORD_ALLOWED_CHANNEL_IDS=1\n",
    )
    .expect("write .env");
    ps_exe
}

struct DetachedProcessGuard {
    pid: u32,
    expected_identity: String,
    expected_exe: PathBuf,
    local_soak_exe: PathBuf,
    journal_script: PathBuf,
}

impl DetachedProcessGuard {
    fn new(
        pid: u32,
        expected_identity: String,
        expected_exe: PathBuf,
        local_soak_exe: PathBuf,
        journal_script: PathBuf,
    ) -> Self {
        Self {
            pid,
            expected_identity,
            expected_exe,
            local_soak_exe,
            journal_script,
        }
    }
}

impl Drop for DetachedProcessGuard {
    fn drop(&mut self) {
        if self.pid == 0 || self.expected_identity.is_empty() {
            return;
        }
        let cmd = format!(
            ". '{}'; \
             Stop-CdrVerifiedProcess -ProcessId {} -ExpectedIdentity '{}' -ExpectedBinaryPath '{}' | Out-Null; \
             $soakPath = [IO.Path]::GetFullPath('{}'); \
             Get-Process -Name 'cdr-offline-soak' -ErrorAction SilentlyContinue | ForEach-Object {{ \
                 try {{ \
                     if ($_.Path -and [IO.Path]::GetFullPath($_.Path) -ieq $soakPath) {{ \
                         $soakId = Get-CdrProcessIdentity -Process $_; \
                         if (-not [string]::IsNullOrWhiteSpace($soakId)) {{ \
                             Stop-CdrVerifiedProcess -ProcessId $_.Id -ExpectedIdentity $soakId -ExpectedBinaryPath $soakPath | Out-Null \
                         }} \
                     }} \
                 }} catch {{ }} \
             }}",
            self.journal_script.display(),
            self.pid,
            self.expected_identity,
            self.expected_exe.display(),
            self.local_soak_exe.display()
        );
        let _ = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &cmd,
            ])
            .output();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn t04_bot_start_and_restart_from_mcp_captured_window_process_survive_normal_exit_cancel_and_timeout_while_ordinary_children_die()
 {
    let (runtime_bin, soak_bin) = ensure_runtime_bins();
    for mode in ["normal_exit", "cancelled", "timed_out"] {
        let discord_double = DiscordRuntimeDouble::spawn().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let ps_exe = copy_control_fixture(temp.path());
        let journal_script = temp.path().join("scripts").join("CdrLaunchJournal.ps1");
        let ordinary_pid_file = temp.path().join("ordinary_child.pid");

        let release_dir = temp.path().join("target").join("release");
        fs::create_dir_all(&release_dir).expect("create target/release");
        let real_bot_exe = release_dir.join("cdr-runtime.exe");
        fs::copy(&runtime_bin, &real_bot_exe).expect("copy real cdr-runtime.exe");
        let local_soak_exe = temp.path().join("cdr-offline-soak.exe");
        fs::copy(&soak_bin, &local_soak_exe).expect("copy cdr-offline-soak.exe");

        let state_db_path = temp.path().join("codex_home").join("state_5.sqlite");
        let init_status = Command::new(&local_soak_exe)
            .arg("init-state-db")
            .arg(&state_db_path)
            .status()
            .expect("run init-state-db");
        assert!(init_status.success(), "init-state-db failed in t04");

        let soak_bin_env = local_soak_exe.to_string_lossy().replace('\\', "/");
        let codex_home_env = temp
            .path()
            .join("codex_home")
            .to_string_lossy()
            .replace('\\', "/");
        let env_path = temp.path().join(".env");
        fs::write(
            &env_path,
            format!(
                "DISCORD_BOT_TOKEN=Bot.test.token\n\
                 DISCORD_ALLOWED_CHANNEL_IDS=42\n\
                 CDR_DISCORD_HTTP_PROXY={}\n\
                 CDR_DISCORD_GATEWAY_PROXY_URL=ws://{}\n\
                 CODEX_HOME={codex_home_env}\n\
                 CODEX_EXE={soak_bin_env}\n\
                 CODEX_REMOTE_MCP_ENABLED=0\n",
                discord_double.http_addr, discord_double.ws_addr
            ),
        )
        .expect("write .env for real cdr-runtime.exe");

        // Pre-launch an initial real cdr-runtime.exe before the terminal command runs so the terminal command exercises
        // stopping the existing real bot and starting/restarting a replacement real cdr-runtime.exe through Invoke-CdrAppLaunch!
        let pre_launch_script = temp.path().join("pre_launch.ps1");
        fs::write(
            &pre_launch_script,
            format!(
                "$ErrorActionPreference = 'Stop'\n\
                 . '{}'\n\
                 $env:CDR_DISCORD_HTTP_PROXY = '{}'\n\
                 $env:CDR_DISCORD_GATEWAY_PROXY_URL = 'ws://{}'\n\
                 $res = Invoke-CdrAppLaunch -RepoRoot '{}' -BotBinaryPath '{}' -BotArguments @('--env','{}') -SkipTray\n\
                 if ($res.BotState -ne 'started') {{ throw \"Pre-launch failed: $($res | ConvertTo-Json -Compress)\" }}\n\
                 $resp = Invoke-RestMethod -Method Get -Uri 'http://{}/__test/inject_help' -TimeoutSec 25\n\
                 if (-not $resp.ok -or [int]$resp.help_replies -ne 1) {{ throw \"Pre-launched real cdr-runtime.exe failed to process Discord !help: $($resp | ConvertTo-Json -Compress)\" }}\n",
                journal_script.display(),
                discord_double.http_addr,
                discord_double.ws_addr,
                temp.path().display(),
                real_bot_exe.display(),
                env_path.display(),
                discord_double.http_addr
            ),
        )
        .expect("write pre_launch.ps1");
        let pre_out = Command::new(&ps_exe)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &pre_launch_script.display().to_string(),
            ])
            .output()
            .expect("run pre_launch_script");
        assert!(
            pre_out.status.success(),
            "mode={mode} pre-launch failed: {}",
            String::from_utf8_lossy(&pre_out.stderr)
        );
        let journal_path = temp
            .path()
            .join(".cdr-maintenance")
            .join("runtime-launch-journal.json");
        let pre_journal: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&journal_path).expect("read pre journal"))
                .expect("parse pre journal");
        let pre_pid = u32::try_from(pre_journal["ProcessId"].as_u64().unwrap_or(0)).unwrap_or(0);
        let pre_identity = pre_journal["ProcessIdentity"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let _pre_guard = DetachedProcessGuard::new(
            pre_pid,
            pre_identity.clone(),
            real_bot_exe.clone(),
            local_soak_exe.clone(),
            journal_script.clone(),
        );

        let tail_wait = if mode == "normal_exit" {
            "Start-Sleep -Milliseconds 100"
        } else {
            "Start-Sleep -Seconds 30"
        };
        let terminal_script = temp.path().join("mcp_terminal_cmd.ps1");
        let terminal_err_log = temp.path().join("terminal_err.log");
        fs::write(
            &terminal_script,
            format!(
                "$ErrorActionPreference = 'Stop'\n\
                 try {{\n\
                   . '{}'\n\
                   $env:CDR_DISCORD_HTTP_PROXY = '{}'\n\
                   $env:CDR_DISCORD_GATEWAY_PROXY_URL = 'ws://{}'\n\
                   $ordinary = Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 60') -WindowStyle Hidden -PassThru\n\
                   [IO.File]::WriteAllText('{}', [string]$ordinary.Id)\n\
                   # Stop prior real cdr-runtime.exe gracefully and restart via supported public entrypoint Invoke-CdrAppLaunch\n\
                   $stopRes = Stop-CdrComponentRuntime -RepoRoot '{}' -Component 'bot' -BinaryPath '{}' -GraceTimeoutMs 18000 -Source 'mcp-terminal-restart'\n\
                   if (-not $stopRes.Stopped) {{ throw \"Prior real cdr-runtime.exe did not stop gracefully during terminal restart: $($stopRes | ConvertTo-Json -Compress)\" }}\n\
                   Set-CdrComponentIntent -RepoRoot '{}' -Component 'bot' -DesiredState 'running' -Action 'mcp-terminal-restart' -Source 'mcp-terminal' | Out-Null\n\
                   $appRes = Invoke-CdrAppLaunch `\n\
                     -RepoRoot '{}' `\n\
                     -BotBinaryPath '{}' `\n\
                     -BotArguments @('--env','{}') `\n\
                     -SkipTray\n\
                   if ($appRes.BotState -ne 'started') {{ throw \"Invoke-CdrAppLaunch inside terminal failed: $($appRes | ConvertTo-Json -Compress)\" }}\n\
                   {}\n\
                 }} catch {{\n\
                   $botErr = if ([IO.File]::Exists((Join-Path '{}' 'codex_discord_rust.error.log'))) {{ Get-Content -LiteralPath (Join-Path '{}' 'codex_discord_rust.error.log') -Raw -ErrorAction SilentlyContinue }} else {{ '' }}\n\
                   [IO.File]::WriteAllText('{}', $_.Exception.Message + \"`n\" + $_.ScriptStackTrace + \"`nBOT_ERR:`n\" + $botErr)\n\
                   throw\n\
                 }}\n",
                journal_script.display(),
                discord_double.http_addr,
                discord_double.ws_addr,
                ordinary_pid_file.display(),
                temp.path().display(),
                real_bot_exe.display(),
                temp.path().display(),
                temp.path().display(),
                real_bot_exe.display(),
                env_path.display(),
                tail_wait,
                temp.path().display(),
                temp.path().display(),
                terminal_err_log.display()
            ),
        )
        .expect("write terminal_script");

        let args = vec![
            "-NoProfile".to_owned(),
            "-ExecutionPolicy".to_owned(),
            "Bypass".to_owned(),
            "-File".to_owned(),
            terminal_script.display().to_string(),
        ];
        let mut env_map: HashMap<String, String> = std::env::vars().collect();
        env_map.insert(
            "CDR_DISCORD_HTTP_PROXY".to_owned(),
            discord_double.http_addr.to_string(),
        );
        env_map.insert(
            "CDR_DISCORD_GATEWAY_PROXY_URL".to_owned(),
            format!("ws://{}", discord_double.ws_addr),
        );
        let mut captured = CapturedWindowProcess::launch(&ps_exe, &args, temp.path(), &env_map)
            .expect("launch CapturedWindowProcess");

        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        let mut launched_pid = 0_u32;
        let mut launched_identity = String::new();
        while std::time::Instant::now() < deadline {
            if let Ok(raw) = fs::read_to_string(&journal_path)
                && let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw)
                && json["Phase"].as_str() == Some("launched")
            {
                let pid = u32::try_from(json["ProcessId"].as_u64().unwrap_or(0)).unwrap_or(0);
                let id = json["ProcessIdentity"].as_str().unwrap_or("").to_owned();
                if pid > 0 && !id.is_empty() && id != pre_identity {
                    launched_pid = pid;
                    launched_identity = id;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let _launched_guard = DetachedProcessGuard::new(
            launched_pid,
            launched_identity.clone(),
            real_bot_exe.clone(),
            local_soak_exe.clone(),
            journal_script.clone(),
        );
        assert!(
            launched_pid > 0 && !launched_identity.is_empty(),
            "mode={mode}: replacement real cdr-runtime.exe launch journal did not reach launched state (err={})",
            fs::read_to_string(&terminal_err_log).unwrap_or_default()
        );
        let ordinary_pid: u32 = fs::read_to_string(&ordinary_pid_file)
            .expect("read ordinary pid")
            .trim()
            .parse()
            .expect("parse ordinary pid");

        match mode {
            "normal_exit" => {
                let wait_deadline = std::time::Instant::now() + Duration::from_secs(10);
                let mut normal_exit_code = None;
                while std::time::Instant::now() < wait_deadline {
                    if let Some(code) = captured.try_wait().expect("try_wait") {
                        normal_exit_code = Some(code);
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                assert_eq!(
                    normal_exit_code,
                    Some(0),
                    "mode={mode}: caller process in CapturedWindowProcess must exit normally with exit code 0 before job cleanup (err={})",
                    fs::read_to_string(&terminal_err_log).unwrap_or_default()
                );
                captured
                    .terminate_tree(Duration::from_secs(2))
                    .expect("cleanup job after normal exit");
            }
            "cancelled" => {
                captured.start_kill().expect("start_kill on cancel");
                captured
                    .terminate_tree(Duration::from_secs(2))
                    .expect("terminate_tree on cancel");
            }
            "timed_out" => {
                tokio::time::sleep(Duration::from_millis(200)).await;
                captured
                    .terminate_tree(Duration::from_secs(2))
                    .expect("terminate_tree on timeout");
            }
            _ => unreachable!(),
        }
        drop(captured);
        tokio::time::sleep(Duration::from_millis(250)).await;

        // 1. Ordinary child process inside CapturedWindowProcess MUST be dead after job close
        let ordinary_live = query_live_process_metadata(ordinary_pid).expect("query ordinary");
        assert!(
            ordinary_live.is_none(),
            "mode={mode}: ordinary terminal child process {ordinary_pid} should have been terminated with the job"
        );
        let pre_live = query_live_process_metadata(pre_pid).expect("query pre bot");
        assert!(
            pre_live.is_none(),
            "mode={mode}: prior real cdr-runtime.exe {pre_pid} should have exited cleanly before replacement"
        );

        // 2. Detached real cdr-runtime.exe process MUST still be alive with exact process_identity!
        let bot_live = query_live_process_metadata(launched_pid)
            .expect("query bot")
            .unwrap_or_else(|| {
                panic!("mode={mode}: detached real cdr-runtime.exe process {launched_pid} died when MCP terminal job closed!")
            });
        assert_eq!(bot_live.process_identity, launched_identity);

        // 3. Verify positive SessionId and UserSid match between caller and detached child
        let sec_check_script = temp.path().join("sec_check.ps1");
        fs::write(
            &sec_check_script,
            format!(
                "$ErrorActionPreference = 'Stop'\n\
                 . '{}'\n\
                 $meSec = Get-CdrProcessSecurityContext -ProcessId $PID\n\
                 $childSec = Get-CdrProcessSecurityContext -ProcessId {}\n\
                 if ($meSec.SessionId -ne $childSec.SessionId -or [string]::IsNullOrWhiteSpace($childSec.UserSid) -or $meSec.UserSid -cne $childSec.UserSid) {{\n\
                   throw \"Security context mismatch: meSession=$($meSec.SessionId) childSession=$($childSec.SessionId) sidMatched=$($meSec.UserSid -ceq $childSec.UserSid)\"\n\
                 }}\n\
                 $resp = Invoke-RestMethod -Method Get -Uri 'http://{}/__test/inject_help' -TimeoutSec 25\n\
                 if (-not $resp.ok -or [int]$resp.help_replies -ne 2) {{\n\
                   throw \"Detached real cdr-runtime.exe failed to process Discord !help after terminal job close (mode={mode}): $($resp | ConvertTo-Json -Compress)\"\n\
                 }}\n",
                journal_script.display(),
                launched_pid,
                discord_double.http_addr
            ),
        )
        .expect("write sec_check.ps1");
        let sec_out = Command::new(&ps_exe)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &sec_check_script.display().to_string(),
            ])
            .output()
            .expect("verify security context and real Discord !help after job close");
        assert!(
            sec_out.status.success(),
            "mode={mode} post-terminal security context + live Discord !help check failed: {}",
            String::from_utf8_lossy(&sec_out.stderr)
        );
        discord_double.abort();
    }
}
