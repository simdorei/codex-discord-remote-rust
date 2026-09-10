//! Operator-assisted recovery: attach verified on-disk lineage to an existing
//! unresolved handoff. Never creates a fork, edits a rollout, or submits a turn.
use rusqlite::{Connection, OpenFlags};
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[6] != "--apply" {
        return Err(
            "usage: restore_known_fork DB HANDOFF SOURCE TARGET ROLLOUT BACKUP --apply".into(),
        );
    }
    let [db, handoff, source, target, rollout, backup, _] = args.as_slice() else {
        unreachable!()
    };
    let line = BufReader::new(File::open(rollout)?)
        .lines()
        .next()
        .ok_or("empty rollout")??;
    let metadata: serde_json::Value = serde_json::from_str(&line)?;
    if metadata["type"] != "session_meta"
        || metadata["payload"]["id"] != *target
        || metadata["payload"]["forked_from_id"] != *source
    {
        return Err("rollout lineage does not match the selected source and target".into());
    }
    let connection = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let row: (String, Option<String>, Option<String>, f64) = connection.query_row(
        "SELECT source_thread_id,observed_target_thread_id,target_thread_id,created_at
         FROM codex_thread_fork_handoffs WHERE handoff_id=?",
        [handoff],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    if row.0 != *source || row.1.is_some() || row.2.is_some() {
        return Err("handoff changed or already has a target; no write performed".into());
    }
    let created = chrono::DateTime::parse_from_rfc3339(
        metadata["payload"]["timestamp"]
            .as_str()
            .ok_or("missing lineage timestamp")?,
    )?;
    let created_time: std::time::SystemTime = created.into();
    let delta = created_time
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs_f64()
        - row.3;
    if !(0.0..=10.0).contains(&delta) {
        return Err("candidate outside operator recovery window".into());
    }
    let inflight: i64 = connection.query_row(
        "SELECT count(*) FROM codex_turn_queue WHERE target_thread_id IN (?,?) AND state IN ('starting','running')",
        [source,target], |r| r.get(0))?;
    if inflight != 0 {
        return Err("inflight job requires review; no write performed".into());
    }
    if Path::new(backup).exists() {
        return Err("backup path already exists".into());
    }
    let mut snapshot = Connection::open(backup)?;
    rusqlite::backup::Backup::new(&connection, &mut snapshot)?.run_to_completion(
        128,
        std::time::Duration::from_millis(10),
        None,
    )?;
    let integrity: String = snapshot.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if integrity != "ok" {
        return Err("backup integrity check failed".into());
    }
    cdr_store::queue::stage_app_server_fork_target(Path::new(db), handoff, target)?;
    println!(
        "verified_existing_target_staged handoff={handoff} target={target}; backup_integrity=ok; no fork/start sent"
    );
    Ok(())
}
