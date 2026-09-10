#![cfg(windows)]

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

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

fn wait_for_json(child: &mut Child, path: &Path, label: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(bytes) = fs::read(path)
            && let Ok(value) = serde_json::from_slice(&bytes)
        {
            return value;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("ownership host exited before {label}: {status}");
        }
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_output(mut child: Child) -> Output {
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let pid = child.id().to_string();
            let _ = Command::new("taskkill")
                .args(["/PID", &pid, "/T", "/F"])
                .output();
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "ownership host did not exit\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().unwrap()
}

fn exact_process_alive(pid: u64, start_ticks: u64) -> bool {
    let script = format!(
        "$p=Get-Process -Id {pid} -ErrorAction SilentlyContinue; \
        if($null -ne $p -and [long]$p.StartTime.ToUniversalTime().Ticks -eq {start_ticks}){{exit 0}};exit 7"
    );
    Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .unwrap()
        .success()
}

#[rustfmt::skip]
fn run_fault_scenario(fault_setup: &str, alive_at_ready: bool, registration_errors: &[&str], stop_error_prefixes: &[&str]) {
    let temp = tempfile::tempdir().unwrap();
    let harness = temp.path().join("guarded-harness.exe");
    let ready = temp.path().join("owner.ready.json");
    let release = temp.path().join("owner.release");
    let stopped = temp.path().join("owner.stopped.json");
    let close_release = temp.path().join("guard-close.release");
    let closed = temp.path().join("guard.closed.json");
    fs::write(&harness, b"immutable while child lives").unwrap();

    let provenance = repo_root().join("scripts/CodexDiscordSoak.Provenance.psm1");
    let ownership = repo_root().join("scripts/CodexDiscordSoak.ProcessOwnership.psm1");
    let host_script = format!(
        "$ErrorActionPreference='Stop';Import-Module {} -Force;Import-Module {} -Force; \
         $c=New-CodexSoakHarnessProvenance -RequestedPath {};$o=New-CodexSoakProcessOwner;$p=$null;$pid0=$null;$ticks=$null;$assignCleanup=@();$guardClosed=$false; \
         try{{$null=Open-CodexSoakHarnessGuard $c;$p=[Diagnostics.Process]::new();$p.StartInfo.FileName='powershell.exe'; \
         $p.StartInfo.Arguments='-NoProfile -NonInteractive -Command \"Start-Sleep -Seconds 30\"';$p.StartInfo.UseShellExecute=$false; \
         if(-not $p.Start()){{throw 'child start failed'}};{} \
         [IO.File]::WriteAllText({},([ordered]@{{pid=$pid0;start_ticks=$ticks;setup=$assign;registration_cleanup_errors=@($assignCleanup);owner_exit_confirmed=$o.ExitConfirmed;close_state=$close}}|ConvertTo-Json -Compress)); \
         $d=[datetime]::UtcNow.AddSeconds(10);while(-not(Test-Path -LiteralPath {})){{if([datetime]::UtcNow -ge $d){{throw 'release timeout'}};Start-Sleep -Milliseconds 20}}; \
         $r=Stop-CodexSoakOwnedProcess $o $pid0 $ticks 5000;if(-not $r.exit_confirmed){{throw 'exact child exit unconfirmed'}}; \
         $null=Complete-CodexSoakHarnessProvenance $c;[IO.File]::WriteAllText({},($r|ConvertTo-Json -Compress)); \
         $d=[datetime]::UtcNow.AddSeconds(10);while(-not(Test-Path -LiteralPath {})){{if([datetime]::UtcNow -ge $d){{throw 'guard close timeout'}};Start-Sleep -Milliseconds 20}}; \
         Close-CodexSoakHarnessGuard $c;$guardClosed=$true;[IO.File]::WriteAllText({},'{{\"guard_closed\":true}}');Close-CodexSoakProcessOwner $o \
         }}finally{{if($null -ne $o){{try{{$null=Stop-CodexSoakOwnedProcess $o $pid0 $ticks 5000}}catch{{}};try{{Close-CodexSoakProcessOwner $o}}catch{{}}}}; \
         if($null -ne $c -and $o.ExitConfirmed -and -not $guardClosed){{Close-CodexSoakHarnessGuard $c}};if($null -ne $p){{$p.Dispose()}}}}",
        ps_literal(&provenance), ps_literal(&ownership), ps_literal(&harness), fault_setup,
        ps_literal(&ready), ps_literal(&release), ps_literal(&stopped),
        ps_literal(&close_release), ps_literal(&closed));
    let host = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &host_script])
        .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut host = ChildGuard::new(host);
    let identity = wait_for_json(host.child_mut(), &ready, "owned child identity");
    let pid = identity["pid"].as_u64().unwrap();
    let ticks = identity["start_ticks"].as_u64().unwrap();
    assert_eq!(exact_process_alive(pid, ticks), alive_at_ready, "exact child state at Register return");
    assert_eq!(identity["owner_exit_confirmed"], !alive_at_ready);
    assert_eq!(identity["registration_cleanup_errors"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect::<Vec<_>>(), registration_errors);
    let denied = OpenOptions::new().write(true).open(&harness).unwrap_err();
    assert!(matches!(denied.raw_os_error(), Some(5 | 32 | 33)), "guard returned {denied}");

    fs::write(&release, b"release").unwrap();
    let result = wait_for_json(host.child_mut(), &stopped, "confirmed exact child exit");
    assert_eq!(result["exit_confirmed"], true);
    let actual_errors = result["cleanup_errors"].as_array().unwrap();
    assert_eq!(actual_errors.len(), stop_error_prefixes.len());
    for (actual, expected) in actual_errors.iter().zip(stop_error_prefixes) {
        assert!(actual.as_str().unwrap().starts_with(expected), "unexpected cleanup error: {actual}");
    }
    assert!(!exact_process_alive(pid, ticks), "exact child remained after exit confirmation");
    let denied = OpenOptions::new().write(true).open(&harness).unwrap_err();
    assert!(matches!(denied.raw_os_error(), Some(5 | 32 | 33)), "post-exit guard returned {denied}");
    fs::write(&close_release, b"close").unwrap();
    assert_eq!(wait_for_json(host.child_mut(), &closed, "guard close")["guard_closed"], true);
    drop(OpenOptions::new().write(true).open(&harness).unwrap());
    let output = wait_for_output(host.take());
    assert!(output.status.success(), "ownership host failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}

#[test]
fn assignment_failure_returns_only_after_exact_child_exit() {
    run_fault_scenario(
        "$o.JobHandle.Dispose();$assign=$null;try{$null=Register-CodexSoakOwnedProcess $o $p}catch{$assign=$_.Exception.Message;$assignCleanup=@($_.Exception.Data['CodexSoakCleanupErrors'])};if($null -eq $assign){throw 'forced assignment failure was accepted'};$pid0=[int]$o.Pid;$ticks=[long]$o.StartTicks;$close='Register rollback barrier returned';",
        false,
        &["Owned process Job handle was unavailable during termination"],
        &[],
    );
}

#[test]
fn native_wait_failure_uses_original_exact_process_barrier() {
    run_fault_scenario(
        "$reg=Register-CodexSoakOwnedProcess $o $p;$pid0=[int]$reg.pid;$ticks=[long]$reg.start_ticks;$o.ProcessHandle.Dispose();$assign='native wait handle invalidated';$close=$null;try{Close-CodexSoakProcessOwner $o}catch{$close=$_.Exception.Message};if($null -eq $close){throw 'live owner closed early'};",
        true,
        &[],
        &[
            "initial_exact_wait_failed: ",
            "bounded_exact_wait_failed: ",
            "direct_exact_terminate_failed: ",
            "final_exact_wait_failed: ",
            "Using retained original Process for the fail-closed exit barrier",
        ],
    );
}

#[test]
fn wrapper_orders_owned_exit_before_snapshot_and_guard_close() {
    let script = fs::read_to_string(repo_root().join("codex-discord-rust-soak.ps1")).unwrap();
    let created = script
        .find("New-CodexSoakProcessOwner")
        .expect("termination owner creation");
    let started = script.find("$child.Start()").expect("child start");
    let registered = script
        .find("Register-CodexSoakOwnedProcess")
        .expect("exact handle registration");
    let stopped = script
        .find("Stop-CodexSoakOwnedProcess")
        .expect("owned termination");
    let snapshot = script
        .find("Complete-CodexSoakHarnessProvenance")
        .expect("post snapshot");
    let guard_close = script
        .find("Close-CodexSoakHarnessGuard")
        .expect("guard close");
    assert!(created < started && started < registered);
    assert!(registered < stopped && stopped < snapshot && snapshot < guard_close);
    assert_eq!(
        script.match_indices("Close-CodexSoakHarnessGuard").count(),
        1
    );
    assert!(
        script.contains("$null -ne $provenanceContext -and (-not $childStarted -or $childStopped)")
    );
    assert!(script.contains("$failure.Data['CodexSoakCleanupErrors']"));
    let pre_registration = &script[started..registered];
    assert!(!pre_registration.contains("$child.Id"));
    assert!(!pre_registration.contains("$child.StartTime"));
}

#[test]
fn registration_retains_original_process_before_fallible_native_capture() {
    let module =
        fs::read_to_string(repo_root().join("scripts/CodexDiscordSoak.ProcessOwnership.psm1"))
            .unwrap();
    let start = module
        .find("function Register-CodexSoakOwnedProcess")
        .unwrap();
    let end = module[start..]
        .find("function Stop-CodexSoakOwnedProcess")
        .map(|offset| start + offset)
        .unwrap();
    let registration = &module[start..end];
    let retained = registration.find("$Owner.Process = $Process").unwrap();
    let duplicated = registration
        .find("DuplicateProcessHandle($Process.Handle)")
        .unwrap();
    let rollback_wait = registration.find("$Process.WaitForExit()").unwrap();
    assert!(retained < duplicated && duplicated < rollback_wait);
    assert!(registration.contains("Stop-CodexSoakOwnedProcess $Owner"));
    assert!(!registration.contains("if ($null -eq $Owner.ProcessHandle)"));
    assert!(registration.contains("while (-not $Owner.ExitConfirmed)"));
}
