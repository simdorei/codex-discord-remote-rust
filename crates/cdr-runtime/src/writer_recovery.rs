//! Recover a verified external desktop writer without replaying old requests.
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cdr_store::queue::{self, QueueJobState};
use serde_json::{Value, json};

pub struct Target<'a> {
    pub root: &'a Path,
    pub codex_home: &'a Path,
    pub database: &'a Path,
    pub thread: &'a str,
    pub channel: i64,
    pub user: i64,
}

/// Full !recover: revoke old requests, then replace the desktop and bot hosts.
/// The bot itself is restarted, so return the durable handoff instead of waiting.
pub async fn run_tools(target: Target<'_>, dry_run: bool) -> Result<Value, String> {
    run_tools_checked(target, dry_run, &|| Ok(()), &|_| Ok(())).await
}

pub async fn run_tools_checked(
    target: Target<'_>,
    dry_run: bool,
    check: &(dyn Fn() -> Result<(), String> + Sync),
    cancellation_check: &(dyn Fn(&rusqlite::Connection) -> cdr_store::Result<()> + Sync),
) -> Result<Value, String> {
    let inspection = controller_checked(&target, "InspectTools", None, check).await?;
    if inspection.get("ThreadId").and_then(Value::as_str) != Some(target.thread)
        || inspection.get("State").and_then(Value::as_str) != Some("tools_recovery")
    {
        return Err("tool recovery inspection returned a different target or scope".into());
    }
    if dry_run {
        return Ok(json!({"phase":"preview_tools","inspection":inspection}));
    }
    let identity = inspection
        .get("RecoveryIdentity")
        .and_then(Value::as_str)
        .ok_or("missing tool host identities")?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs_f64();
    let cancelled = queue::cancel_for_recovery_checked(
        target.database,
        target.thread,
        target.channel,
        target.user,
        now,
        cancellation_check,
    )
    .map_err(|e| e.to_string())?;
    let dispatched = controller_checked(&target, "StartTools", Some(identity), check)
        .await
        .map_err(|e| {
            format!(
                "{} requests were cancelled and will not replay, but restart dispatch failed: {e}",
                cancelled.jobs.len()
            )
        })?;
    let receipt = validated_receipt(target.root, &dispatched)?;
    Ok(
        json!({"phase":"dispatched_tools","thread":target.thread,"cancelled":cancelled.jobs,
        "started_or_uncertain":cancelled.started_or_uncertain,"receipt":receipt}),
    )
}

pub async fn run(target: Target<'_>, dry_run: bool, wait: bool) -> Result<Value, String> {
    let inspection = controller(&target, "Inspect", None).await?;
    if inspection.get("ThreadId").and_then(Value::as_str) != Some(target.thread) {
        return Err("writer inspection returned a different thread; no recovery was sent".into());
    }
    let state = inspection.get("State").and_then(Value::as_str);
    if !matches!(state, Some("desktop_owned" | "unlocked")) {
        return Err("writer ownership is unverified; no recovery was sent".into());
    }
    if dry_run {
        return Ok(json!({"phase":"preview","inspection":inspection}));
    }
    let cancelled = cancel_unstarted(&target)?;
    if state == Some("unlocked") {
        return Ok(json!({"phase":"unlocked","thread":target.thread,"cancelled":cancelled}));
    }
    let identity = inspection
        .get("WriterIdentity")
        .and_then(Value::as_str)
        .ok_or("missing writer process identity")?;
    let dispatched = controller(&target, "Start", Some(identity)).await?;
    let receipt = validated_receipt(target.root, &dispatched)?;
    if !wait {
        return Ok(
            json!({"phase":"dispatched","thread":target.thread,"cancelled":cancelled,"receipt":receipt}),
        );
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(50);
    while tokio::time::Instant::now() < deadline {
        if let Ok(bytes) = tokio::fs::read(&receipt).await
            && let Ok(result) = serde_json::from_slice::<Value>(&bytes)
            && matches!(
                result.get("Phase").and_then(Value::as_str),
                Some("completed" | "reacquired" | "failed")
            )
        {
            return Ok(
                json!({"phase":result["Phase"],"thread":target.thread,"cancelled":cancelled,"receipt":receipt,"result":result}),
            );
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(json!({"phase":"unverified","thread":target.thread,"cancelled":cancelled,"receipt":receipt}))
}

fn cancel_unstarted(target: &Target<'_>) -> Result<Vec<String>, String> {
    let jobs = queue::list_filtered(target.database, Some(target.thread), None)
        .map_err(|e| e.to_string())?;
    if jobs.iter().any(|job| {
        job.state != QueueJobState::Quarantined
            && (job.state != QueueJobState::Pending
                || job.channel_id != target.channel
                || job.owner_user_id != Some(target.user))
    }) {
        return Err("recovery preparation refused: running, uncertain, or another sender's requests remain; no desktop restart was sent".into());
    }
    let mut cancelled = Vec::new();
    for _ in 0..128 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs_f64();
        let job = queue::cancel_latest_pending(
            target.database,
            target.thread,
            target.channel,
            target.user,
            now,
        )
        .map_err(|e| {
            format!(
                "recovery preparation stopped after {} cancellations: {e}; desktop not restarted",
                cancelled.len()
            )
        })?;
        let Some(job) = job else {
            let remaining = queue::list_filtered(target.database, Some(target.thread), None)
                .map_err(|e| e.to_string())?;
            let intakes = cdr_store::prompt_intake::list_prompt_intakes(target.database)
                .map_err(|e| e.to_string())?;
            if remaining
                .iter()
                .any(|job| job.state != QueueJobState::Quarantined)
                || intakes
                    .iter()
                    .any(|job| job.target_thread_id == target.thread)
            {
                return Err(
                    "requests changed during recovery preparation; no desktop restart was sent"
                        .into(),
                );
            }
            return Ok(cancelled);
        };
        cancelled.push(job);
    }
    Err("recovery cancellation limit reached; desktop not restarted".into())
}

fn validated_receipt(root: &Path, value: &Value) -> Result<PathBuf, String> {
    let operation = value
        .get("Operation")
        .and_then(Value::as_str)
        .ok_or("missing recovery operation")?;
    if operation.len() != 32
        || !operation
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("invalid recovery operation identity".into());
    }
    let expected = root
        .join("maintenance_backups/desktop-recovery")
        .join(operation)
        .join("receipt.json");
    if value
        .get("ReceiptPath")
        .and_then(Value::as_str)
        .map(Path::new)
        != Some(expected.as_path())
    {
        return Err("recovery receipt escaped its operation directory".into());
    }
    Ok(expected)
}

async fn controller(
    target: &Target<'_>,
    mode: &str,
    identity: Option<&str>,
) -> Result<Value, String> {
    controller_checked(target, mode, identity, &|| Ok(())).await
}

async fn controller_checked(
    target: &Target<'_>,
    mode: &str,
    identity: Option<&str>,
    check: &(dyn Fn() -> Result<(), String> + Sync),
) -> Result<Value, String> {
    if !cfg!(windows) {
        return Err("desktop writer recovery is available on Windows only".into());
    }
    let script = target.root.join("scripts/Invoke-CdrDesktopRecovery.ps1");
    if !script.is_file() {
        return Err("desktop recovery controller is missing".into());
    }
    let mut command = tokio::process::Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
        ])
        .arg(script)
        .arg("-Mode")
        .arg(mode)
        .arg("-RepoRoot")
        .arg(target.root)
        .arg("-CodexHome")
        .arg(target.codex_home)
        .arg("-ThreadId")
        .arg(target.thread)
        .current_dir(target.root)
        .kill_on_drop(true);
    if let Some(identity) = identity {
        command
            .arg(if mode == "StartTools" {
                "-ExpectedRecoveryIdentity"
            } else {
                "-ExpectedWriterIdentity"
            })
            .arg(identity);
    }
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    check()?;
    let output = tokio::time::timeout(Duration::from_secs(20), command.output()).await
        .map_err(|_| "desktop recovery controller timed out; check saved recovery receipts before retrying")?
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "desktop recovery controller refused: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("invalid desktop recovery receipt: {e}"))
}

pub fn message(report: &Value) -> String {
    let count = report
        .get("cancelled")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let status = match report.get("phase").and_then(Value::as_str) {
        Some("dispatched_tools") => {
            "이전 요청의 취소 기록을 저장했습니다. 데스크톱 앱과 봇·앱서버를 재시작합니다. 다른 앱 채팅과 작업도 중단될 수 있습니다. 이전 요청은 자동 재실행하지 않습니다. 아직 재시작 완료가 확인된 상태는 아닙니다."
        }
        Some("restarted") => {
            "데스크톱 앱과 봇·앱서버 재시작을 확인했습니다. 개별 도구 연결은 새 도구 호출에서 확인해야 합니다."
        }
        Some("completed") => {
            "데스크톱 앱을 재시작했고 해당 채팅의 점유 해제를 확인했습니다. 새 요청을 보낼 수 있습니다."
        }
        Some("unlocked") => "해당 채팅을 점유한 프로세스가 없습니다.",
        Some("reacquired") => {
            "앱은 재시작됐지만 해당 채팅의 점유를 다시 가져갔습니다. 복구 완료로 처리하지 않았습니다."
        }
        Some("failed") => "데스크톱 복구가 완료되지 않았습니다. 기록된 오류를 확인하세요.",
        _ => "복구 결과가 아직 확인되지 않았습니다. 기록을 확인하기 전 재실행하지 마세요.",
    };
    format!(
        "{status}\n이전 요청 취소 기록: {count}개\n복구 기록: {}\n{}",
        report
            .get("receipt")
            .and_then(Value::as_str)
            .unwrap_or("없음"),
        report
            .pointer("/result/Error")
            .and_then(Value::as_str)
            .unwrap_or("")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipt_must_stay_with_the_exact_operation() {
        let root = Path::new("root");
        let op = "a".repeat(32);
        let expected = root
            .join("maintenance_backups/desktop-recovery")
            .join(&op)
            .join("receipt.json");
        assert_eq!(
            validated_receipt(root, &json!({"Operation":op,"ReceiptPath":expected})).unwrap(),
            expected
        );
        for value in [
            json!({"Operation":"../escape","ReceiptPath":"elsewhere"}),
            json!({"Operation":op,"ReceiptPath":"elsewhere"}),
        ] {
            assert!(validated_receipt(root, &value).is_err());
        }
    }
    #[test]
    fn unverified_or_reacquired_is_never_reported_as_recovered() {
        for phase in ["dispatched", "unverified", "failed", "reacquired"] {
            assert!(!message(&json!({"phase":phase})).contains("해제를 확인했습니다"));
        }
    }
}

#[cfg(all(test, windows))]
#[path = "writer_recovery/admitted_handoff_tests.rs"]
mod admitted_handoff_tests;
