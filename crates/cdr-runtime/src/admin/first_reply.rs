//! Exact-job, read-only preview. This command never grants replay/delivery authority.
use super::args::Args;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};

pub(super) fn run(args: &Args, root: &Path) -> Result<String, String> {
    let path = root.join(args.required("--database")?);
    inspect(&path, args.required("--job-id")?)
        .map(|report| report.to_string())
        .map_err(|error| format!("Cannot inspect first reply: {error}"))
}

fn inspect(path: &Path, job: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.busy_timeout(Duration::from_secs(3))?;
    db.execute_batch("PRAGMA query_only=ON; BEGIN")?;
    let ingress_count: i64 = db.query_row(
        "SELECT count(*) FROM discord_ingress_journal WHERE owner_id=?",
        [job],
        |r| r.get(0),
    )?;
    let outbox_count: i64 = db.query_row(
        "SELECT count(*) FROM codex_delivery_outbox WHERE job_id=?",
        [job],
        |r| r.get(0),
    )?;
    let mut report = json!({
        "read_only":true,"job_id":job,"ingress_count":ingress_count,"outbox_count":outbox_count,
        "replay_authorized":false,"delivery_authorized":false
    });
    if ingress_count != 1 || outbox_count != 1 {
        report["reason"] = json!("exact ingress/outbox identity is not unique; preserved");
        return Ok(report);
    }
    let owner = db.query_row(
        "SELECT event_id,channel_id,target_thread_id,outcome_json,confirmation_delivered FROM discord_ingress_journal WHERE owner_id=?",
        [job], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<i64>>(4)?)),
    )?;
    let final_row = db.query_row(
        "SELECT target_thread_id,turn_id,channel_id,content FROM codex_delivery_outbox WHERE job_id=?",
        [job], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?)),
    )?;
    let evidence: Value =
        serde_json::from_str(owner.3.as_deref().filter(|v| !v.is_empty()).unwrap_or("{}"))?;
    let prepared = &evidence["new_verification"];
    let mut statement =
        db.prepare("SELECT codex_thread_id FROM mirror_threads WHERE discord_thread_id=? LIMIT 2")?;
    let mapping = statement
        .query_map([final_row.2], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let identity_matches = owner.2.as_deref() == Some(final_row.0.as_str())
        && prepared["thread_id"].as_str() == Some(final_row.0.as_str())
        && prepared["channel_id"].as_i64() == Some(final_row.2)
        && mapping.as_slice() == [final_row.0.as_str()];
    let key = json!([
        owner.1,
        "message/reply/v1",
        format!("inbound-message/{}/action-result", owner.0),
        0
    ])
    .to_string();
    let receipt = db.query_row(
        "SELECT message_id,retryable,blocked_reason FROM codex_delivery_receipts WHERE receipt_key=?", params![key],
        |r| Ok((r.get::<_,Option<i64>>(0)?,r.get::<_,Option<i64>>(1)?,r.get::<_,Option<String>>(2)?)),
    ).optional()?;
    let receipt_state = match receipt {
        None => "absent",
        Some((Some(id), _, _)) if id != 0 => "confirmed",
        Some((_, _, Some(reason))) if !reason.is_empty() => "rejected_blocked",
        Some((_, Some(retryable), _)) if retryable != 0 => "definite_rejection",
        Some(_) => "unknown",
    };
    report["thread_id"] = json!(final_row.0);
    report["turn_id"] = json!(final_row.1);
    report["destination_matches"] = json!(identity_matches);
    report["normal_ack_receipt"] = json!(receipt_state);
    report["final_sha256"] = json!(format!("{:x}", Sha256::digest(final_row.3.as_bytes())));
    report["normal_ack_confirmed_flag"] = json!(owner.4.is_some_and(|v| v != 0));
    report["reason"] = json!(
        "Historical non-attempt and current exclusive delivery ownership are not proven by this preview; preserved"
    );
    Ok(report)
}
