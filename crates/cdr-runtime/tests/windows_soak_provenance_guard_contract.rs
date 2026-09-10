#![cfg(windows)]

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ps_literal(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }

    fn child_mut(&mut self) -> &mut Child {
        self.0.as_mut().unwrap()
    }

    fn take(&mut self) -> Child {
        self.0.take().unwrap()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let pid = child.id().to_string();
            let _ = Command::new("taskkill")
                .args(["/PID", &pid, "/T", "/F"])
                .status();
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn wait_for_signal(child: &mut Child, path: &Path, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("guard host exited before {label}: {status}");
        }
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        thread::sleep(Duration::from_millis(20));
    }
}

#[rustfmt::skip]
fn wait_for_output(mut child: Child) -> Output {
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let pid = child.id().to_string();
            let _ = Command::new("taskkill").args(["/PID", &pid, "/T", "/F"]).output();
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("guard host did not exit\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().unwrap()
}

fn sharing_error(operation: &str) -> i32 {
    let script = format!(
        "try {{ {operation}; exit 0 }} catch {{ $e=$_.Exception; while($null -ne $e.InnerException){{$e=$e.InnerException}}; [Console]::Out.WriteLine($e.HResult -band 0xffff); exit 7 }}"
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(7),
        "operation was not refused by guard\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

#[test]
fn explicit_guard_blocks_independent_mutation_until_closed() {
    let temp = tempfile::tempdir().unwrap();
    let harness = temp.path().join("guarded-harness.exe");
    let ready = temp.path().join("guard.ready");
    let release = temp.path().join("guard.release");
    let closed = temp.path().join("guard.closed");
    let finish = temp.path().join("guard.finish");
    fs::write(&harness, b"original").unwrap();

    let module = repo_root().join("scripts/CodexDiscordSoak.Provenance.psm1");
    let host_script = format!(
        "$ErrorActionPreference='Stop'; Import-Module {} -Force; \
         $c=New-CodexSoakHarnessProvenance -RequestedPath {}; try {{ \
         $null=Open-CodexSoakHarnessGuard $c; [IO.File]::WriteAllText({},'ready'); \
         $d=[datetime]::UtcNow.AddSeconds(10); while(-not(Test-Path -LiteralPath {})){{if([datetime]::UtcNow -ge $d){{throw 'release timeout'}}; Start-Sleep -Milliseconds 20}}; \
         Close-CodexSoakHarnessGuard $c; [IO.File]::WriteAllText({},'closed'); \
         $d=[datetime]::UtcNow.AddSeconds(10); while(-not(Test-Path -LiteralPath {})){{if([datetime]::UtcNow -ge $d){{throw 'finish timeout'}}; Start-Sleep -Milliseconds 20}} \
         }} finally {{ Close-CodexSoakHarnessGuard $c }}",
        ps_literal(&module),
        ps_literal(&harness),
        ps_literal(&ready),
        ps_literal(&release),
        ps_literal(&closed),
        ps_literal(&finish)
    );
    let host = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &host_script])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut host = ChildGuard::new(host);
    wait_for_signal(host.child_mut(), &ready, "guard open");

    let write = format!(
        "$s=[IO.File]::Open({},'Open','Write','ReadWrite');$s.Dispose()",
        ps_literal(&harness)
    );
    let delete = format!("[IO.File]::Delete({})", ps_literal(&harness));
    for (name, code) in [
        ("write", sharing_error(&write)),
        ("delete", sharing_error(&delete)),
    ] {
        assert!(
            matches!(code, 5 | 32 | 33),
            "{name} returned non-sharing error {code}"
        );
    }
    assert_eq!(fs::read(&harness).unwrap(), b"original");

    fs::write(&release, b"release").unwrap();
    wait_for_signal(host.child_mut(), &closed, "guard close");
    let stream = OpenOptions::new().write(true).open(&harness).unwrap();
    drop(stream);
    assert!(
        host.child_mut().try_wait().unwrap().is_none(),
        "module host exited before loader-lock check"
    );
    fs::remove_file(&harness).unwrap();
    fs::write(&finish, b"finish").unwrap();
    let output = wait_for_output(host.take());
    assert!(
        output.status.success(),
        "guard host failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
