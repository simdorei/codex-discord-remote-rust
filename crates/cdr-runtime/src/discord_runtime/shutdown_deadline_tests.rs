use std::time::Duration;

use tokio::sync::oneshot;

#[cfg(windows)]
use std::io::{BufRead, Write};
#[cfg(windows)]
use std::process::{Command, Stdio};

#[cfg(windows)]
use cdr_app_server::{AppServerClient, AppServerConfig};

use super::complete_before_shutdown_deadline;

#[cfg(windows)]
const REAL_ABORT_ENV: &str = "CDR_TEST_REAL_SHUTDOWN_ABORT";
#[cfg(windows)]
const ABORT_PARENT_ENV: &str = "CDR_HARD_EXPIRY_PARENT_PROBE";
#[cfg(windows)]
const APP_SERVER_HELPER_ENV: &str = "CDR_HARD_EXPIRY_APP_SERVER_HELPER";
#[cfg(windows)]
const PID_FILE_ENV: &str = "CDR_HARD_EXPIRY_PID_FILE";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[should_panic(expected = "fatal runtime shutdown timeout: app-server-test")]
async fn wsu_13_abort_resistant_close_wait_escalates_at_its_hard_deadline() {
    let (started, observed) = oneshot::channel();
    let forwarder = tokio::spawn(async move {
        started.send(()).expect("test receives readiness");
        std::thread::sleep(Duration::from_millis(250));
    });
    observed.await.expect("abort-resistant task starts");

    complete_before_shutdown_deadline(
        tokio::time::Instant::now() + Duration::from_millis(25),
        "app-server-test",
        async move { forwarder.await.expect("forwarder eventually exits") },
    )
    .await;
}

#[cfg(windows)]
#[test]
#[ignore = "spawned by the hard-expiry app-server containment test"]
fn wsu_stubborn_app_server_helper_probe() {
    if std::env::var_os(APP_SERVER_HELPER_ENV).is_none() {
        return;
    }
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

#[cfg(windows)]
#[test]
#[ignore = "spawned by the hard-expiry app-server containment test"]
fn wsu_hard_expiry_parent_probe() {
    if std::env::var_os(ABORT_PARENT_ENV).is_none() {
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
            "discord_runtime::shutdown_deadline::tests::wsu_stubborn_app_server_helper_probe"
                .to_owned(),
            "--ignored".to_owned(),
        ];
        config
            .environment
            .insert(APP_SERVER_HELPER_ENV.to_owned(), "1".to_owned());
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

        complete_before_shutdown_deadline(
            tokio::time::Instant::now() + Duration::from_millis(25),
            "app-server-containment-probe",
            std::future::pending::<()>(),
        )
        .await;
    });
}

#[cfg(windows)]
#[test]
fn wsu_16_hard_expiry_removes_runtime_and_app_server_processes() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let pid_file = temporary.path().join("hard-expiry-pids.txt");
    let status = Command::new(std::env::current_exe().expect("current test executable"))
        .args([
            "--exact",
            "discord_runtime::shutdown_deadline::tests::wsu_hard_expiry_parent_probe",
            "--ignored",
            "--nocapture",
        ])
        .env(REAL_ABORT_ENV, "1")
        .env(ABORT_PARENT_ENV, "1")
        .env(PID_FILE_ENV, &pid_file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run hard-expiry probe");
    assert!(!status.success(), "hard-expiry probe must abort");

    let pids = std::fs::read_to_string(&pid_file).expect("read PID evidence");
    let mut pids = pids.lines().map(|line| line.parse::<u32>().expect("PID"));
    let runtime_pid = pids.next().expect("runtime PID");
    let app_server_pid = pids.next().expect("app-server PID");
    assert!(wait_for_exit(runtime_pid, Duration::from_secs(1)));
    let app_server_exited = wait_for_exit(app_server_pid, Duration::from_secs(1));
    if !app_server_exited {
        terminate_for_cleanup(app_server_pid);
    }
    assert!(
        app_server_exited,
        "hard expiry orphaned app-server PID {app_server_pid}"
    );
}

#[cfg(windows)]
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
            .expect("query exact probe PID");
        if status.success() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
fn terminate_for_cleanup(process_id: u32) {
    let status = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            &format!("Stop-Process -Id {process_id} -Force -ErrorAction SilentlyContinue"),
        ])
        .status()
        .expect("terminate exact disposable probe PID");
    assert!(status.success(), "failed to clean up probe PID");
}
