#![cfg(windows)]

use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use cdr_app_server::{AppServerClient, AppServerConfig};

const PARENT_PROBE_ENV: &str = "CDR_APP_SERVER_ABORT_PARENT_PROBE";
const CHILD_PROBE_ENV: &str = "CDR_APP_SERVER_STUBBORN_CHILD_PROBE";
const GRANDCHILD_PROBE_ENV: &str = "CDR_APP_SERVER_STUBBORN_GRANDCHILD_PROBE";
const PID_FILE_ENV: &str = "CDR_APP_SERVER_ABORT_PID_FILE";
const GRANDCHILD_PID_FILE_ENV: &str = "CDR_APP_SERVER_GRANDCHILD_PID_FILE";

#[test]
#[ignore = "spawned by the Windows abort-containment regression test"]
fn app_server_stubborn_child_probe() {
    if std::env::var_os(CHILD_PROBE_ENV).is_none() {
        return;
    }

    let grandchild_pid_file =
        std::env::var_os(GRANDCHILD_PID_FILE_ENV).expect("grandchild PID file is configured");
    let grandchild = Command::new(std::env::current_exe().expect("current test executable"))
        .args([
            "--exact",
            "app_server_stubborn_grandchild_probe",
            "--ignored",
            "--nocapture",
        ])
        .env(GRANDCHILD_PROBE_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn immediate stubborn grandchild");
    let mut evidence =
        std::fs::File::create(grandchild_pid_file).expect("create grandchild PID evidence");
    writeln!(evidence, "{}", grandchild.id()).expect("write grandchild PID evidence");
    evidence.sync_all().expect("flush grandchild PID evidence");
    drop(grandchild);

    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let message: serde_json::Value = serde_json::from_str(&line).expect("JSON-RPC request");
        if message.get("method").and_then(serde_json::Value::as_str) == Some("initialized") {
            break;
        }
        let Some(id) = message.get("id") else {
            continue;
        };
        serde_json::to_writer(&mut stdout, &serde_json::json!({"id": id, "result": {}}))
            .expect("write initialize response");
        writeln!(stdout).expect("terminate initialize response");
        stdout.flush().expect("flush initialize response");
    }
    std::thread::sleep(Duration::from_secs(30));
}

#[test]
#[ignore = "spawned by the Windows abort-containment regression test"]
fn app_server_stubborn_grandchild_probe() {
    if std::env::var_os(GRANDCHILD_PROBE_ENV).is_some() {
        std::thread::sleep(Duration::from_secs(30));
    }
}

#[test]
#[ignore = "spawned by the Windows abort-containment regression test"]
fn app_server_abort_parent_probe() {
    if std::env::var_os(PARENT_PROBE_ENV).is_none() {
        return;
    }

    let pid_file = std::env::var_os(PID_FILE_ENV).expect("PID file is configured");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("probe runtime");
    runtime.block_on(async move {
        let mut config =
            AppServerConfig::new(std::env::current_exe().expect("current test executable"));
        config.arguments = vec![
            "--exact".to_owned(),
            "app_server_stubborn_child_probe".to_owned(),
            "--ignored".to_owned(),
        ];
        config
            .environment
            .insert(CHILD_PROBE_ENV.to_owned(), "1".to_owned());
        let client = AppServerClient::start(config)
            .await
            .expect("start stubborn app-server helper");
        let child_pid = client
            .lifecycle_snapshot()
            .process_id
            .expect("app-server PID");
        let mut file = std::fs::File::create(pid_file).expect("create PID evidence");
        writeln!(file, "{}\n{child_pid}", std::process::id()).expect("write PID evidence");
        file.sync_all().expect("flush PID evidence");
        std::process::abort();
    });
}

#[test]
fn app_server_process_tree_dies_when_runtime_owner_aborts() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let pid_file = temporary.path().join("abort-pids.txt");
    let grandchild_pid_file = temporary.path().join("grandchild-pid.txt");
    let status = Command::new(std::env::current_exe().expect("current test executable"))
        .args([
            "--exact",
            "app_server_abort_parent_probe",
            "--ignored",
            "--nocapture",
        ])
        .env(PARENT_PROBE_ENV, "1")
        .env(PID_FILE_ENV, &pid_file)
        .env(GRANDCHILD_PID_FILE_ENV, &grandchild_pid_file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run aborting owner probe");
    assert!(
        !status.success(),
        "probe must terminate through process abort"
    );

    let pids = std::fs::read_to_string(&pid_file).expect("read PID evidence");
    let mut pids = pids.lines().map(|line| line.parse::<u32>().expect("PID"));
    let parent_pid = pids.next().expect("parent PID");
    let child_pid = pids.next().expect("child PID");
    let grandchild_pid = std::fs::read_to_string(&grandchild_pid_file)
        .expect("read grandchild PID evidence")
        .trim()
        .parse::<u32>()
        .expect("grandchild PID");
    assert!(wait_for_exit(parent_pid, Duration::from_secs(1)));
    let child_exited = wait_for_exit(child_pid, Duration::from_secs(1));
    let grandchild_exited = wait_for_exit(grandchild_pid, Duration::from_secs(1));
    if !child_exited {
        terminate_for_cleanup(child_pid);
    }
    if !grandchild_exited {
        terminate_for_cleanup(grandchild_pid);
    }
    assert!(
        child_exited,
        "app-server helper survived its aborted runtime owner (PID {child_pid})"
    );
    assert!(
        grandchild_exited,
        "immediate app-server grandchild survived its aborted runtime owner (PID {grandchild_pid})"
    );
}

fn wait_for_exit(process_id: u32, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let status = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "if (Get-Process -Id {process_id} -ErrorAction SilentlyContinue) {{ exit 1 }} else {{ exit 0 }}"
                ),
            ])
            .status()
            .expect("query exact test-helper PID");
        if status.success() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn terminate_for_cleanup(process_id: u32) {
    let status = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            &format!("Stop-Process -Id {process_id} -Force -ErrorAction SilentlyContinue"),
        ])
        .status()
        .expect("terminate exact disposable helper PID");
    assert!(status.success(), "failed to clean up test-helper PID");
}
