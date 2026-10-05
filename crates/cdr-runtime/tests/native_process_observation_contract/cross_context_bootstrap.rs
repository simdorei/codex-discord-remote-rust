use std::{
    fs::{self, File},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn directory() -> (String, PathBuf) {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let path = std::env::temp_dir().join(format!("cdr-cross-{nonce}"));
    fs::create_dir(&path).expect("unique diagnostic directory");
    (nonce, path)
}

fn builder(directory: &Path, nonce: &str) -> Command {
    let windows = std::env::var_os("SystemRoot").expect("Windows system root");
    let mut command =
        Command::new(Path::new(&windows).join("System32/WindowsPowerShell/v1.0/powershell.exe"));
    command
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(fixtures().join("native_process_cross_context.ps1"))
        .arg("-RunDirectory")
        .arg(directory)
        .arg("-Nonce")
        .arg(nonce);
    command
}

fn supervised(
    mut command: Command,
    directory: &Path,
    stage: &str,
    deadline: Instant,
) -> Result<Output, String> {
    if Instant::now() >= deadline {
        return Err(format!(
            "{stage}: absolute supervisor deadline before spawn; safe_for_follow_up=false"
        ));
    }
    let stdout = directory.join(format!("{stage}.stdout"));
    let stderr = directory.join(format!("{stage}.stderr"));
    command
        .stdin(Stdio::null())
        .stdout(File::create_new(&stdout).map_err(|error| error.to_string())?)
        .stderr(File::create_new(&stderr).map_err(|error| error.to_string())?)
        .creation_flags(0x0800_0000);
    if Instant::now() >= deadline {
        return Err(format!(
            "{stage}: deadline at spawn boundary; safe_for_follow_up=false"
        ));
    }
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let pid = child.id();
    let mut initial_wait_error = None;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(Output {
                    status,
                    stdout: fs::read(&stdout).map_err(|error| error.to_string())?,
                    stderr: fs::read(&stderr).map_err(|error| error.to_string())?,
                });
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => break,
            Err(error) => {
                initial_wait_error = Some(error.to_string());
                break;
            }
        }
    }
    let kill_error = child.kill().err().map(|error| error.to_string());
    let cleanup_deadline = Instant::now() + Duration::from_secs(2);
    let mut exit_confirmed = false;
    let mut wait_error = None;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                exit_confirmed = true;
                break;
            }
            Ok(None) if Instant::now() < cleanup_deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => break,
            Err(error) => {
                wait_error = Some(error.to_string());
                break;
            }
        }
    }
    let evidence = serde_json::json!({
        "stage": stage, "pid": pid, "forced": true, "exit_confirmed": exit_confirmed,
        "kill_error": kill_error, "initial_wait_error": initial_wait_error, "wait_error": wait_error, "safe_for_follow_up": false,
        "scope": "exact direct child only; never evidence of WMI peer exit"
    });
    eprintln!("NATIVE_CROSS_BOOTSTRAP {evidence}");
    Err(evidence.to_string())
}

pub(super) struct Built {
    directory: PathBuf,
    nonce: String,
    metadata: serde_json::Value,
    deadline: Instant,
}

impl Built {
    pub(super) fn new() -> Self {
        // Single outer lifetime, including the build. It is never reset for A.
        let deadline = Instant::now() + Duration::from_secs(65);
        let (nonce, directory) = directory();
        let output = supervised(builder(&directory, &nonce), &directory, "build", deadline)
            .expect("bounded diagnostic build");
        assert!(
            output.status.success(),
            "build: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let metadata: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("build metadata");
        assert_eq!(
            metadata["image"].as_str().map(Path::new),
            Some(directory.join("native_process_cross_context.exe").as_path())
        );
        eprintln!("NATIVE_CROSS_BOOTSTRAP_BUILD {metadata}");
        Self {
            directory,
            nonce,
            metadata,
            deadline,
        }
    }

    pub(super) fn execute(&self, mode: &str, expired: bool) -> Output {
        let expiry = if expired {
            "1"
        } else {
            self.metadata["expiry"].as_str().unwrap()
        };
        let stage = if expired { "expired" } else { mode };
        let mut command = Command::new(self.directory.join("native_process_cross_context.exe"));
        command
            .arg(mode)
            .arg(&self.directory)
            .arg(&self.nonce)
            .arg(self.metadata["bundle"].as_str().unwrap())
            .arg(expiry)
            .arg(self.metadata["image_pin"].as_str().unwrap())
            .arg(fixtures());
        supervised(command, &self.directory, stage, self.deadline)
            .expect("bounded compiled diagnostic")
    }
}

pub(super) fn assert_stalled_compiler_is_reaped() {
    let (nonce, directory) = directory();
    let mut command = builder(&directory, &nonce);
    command.arg("-InjectBuildStall");
    let failure = supervised(
        command,
        &directory,
        "build-stall",
        Instant::now() + Duration::from_secs(2),
    )
    .expect_err("injected pre-compilation stall must reach the supervisor");
    let evidence: serde_json::Value =
        serde_json::from_str(&failure).expect("structured cleanup evidence");
    assert_eq!(evidence["exit_confirmed"], true);
    assert_eq!(evidence["forced"], true);
    assert_eq!(evidence["safe_for_follow_up"], false);
    assert_eq!(
        fs::read_to_string(directory.join("before-compilation.marker")).unwrap(),
        "before Add-Type"
    );
    assert!(
        !directory.join("native_process_cross_context.exe").exists(),
        "no A/B executable or WMI dispatch during stalled build"
    );
}
