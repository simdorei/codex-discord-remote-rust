use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;

use cdr_store::mapping::upsert_thread;
use cdr_store::mirror::update_cursor;
use rusqlite::Connection;
use serde_json::json;

use super::SoakResult;

pub(super) const MIRROR_THREAD: &str = "thread-a";

pub(super) fn seed_state(state_db: &Path, mirror_db: &Path, rollout: &Path) -> SoakResult<()> {
    fs::write(rollout, [])?;
    let connection = Connection::open(state_db)?;
    connection.execute_batch(
        "CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, \
         updated_at INTEGER, rollout_path TEXT, model TEXT, reasoning_effort TEXT, \
         tokens_used INTEGER, archived INTEGER, archived_at INTEGER, source TEXT, \
         thread_source TEXT);",
    )?;
    connection.execute(
        "INSERT INTO threads VALUES (?1, 'Offline soak', 'C:/offline', 1, ?2, \
         'fake', 'high', 0, 0, 0, 'offline', 'offline')",
        [MIRROR_THREAD, rollout.to_string_lossy().as_ref()],
    )?;
    upsert_thread(
        mirror_db,
        MIRROR_THREAD,
        "offline",
        "Offline soak",
        100,
        200,
        1.0,
    )?;
    update_cursor(
        mirror_db,
        MIRROR_THREAD,
        rollout.to_string_lossy().as_ref(),
        0,
        1.0,
    )?;
    Ok(())
}

pub(super) fn append_repeated_assistant_shapes(
    rollout: &Path,
    cycle: u64,
    seed: u64,
    origin_turn_id: &str,
) -> SoakResult<()> {
    let text = format!("offline-update-{seed}-{cycle}");
    let values = [
        json!({
            "timestamp": format!("{cycle}-1"),
            "type": "event_msg",
            "payload": {"type": "agent_message", "message": text}
        }),
        json!({
            "timestamp": format!("{cycle}-2"),
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "assistant",
                "phase": "commentary",
                "content": [{"type": "output_text", "text": text}]
            }
        }),
        json!({
            "timestamp": format!("{cycle}-3"),
            "type": "event_msg",
            "payload": {
                "type": "task_complete",
                "turn_id": origin_turn_id,
                "last_agent_message": text
            }
        }),
    ];
    let file = OpenOptions::new().append(true).open(rollout)?;
    let mut writer = BufWriter::new(file);
    for value in values {
        serde_json::to_writer(&mut writer, &value)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

pub(super) fn compact_rollout(rollout: &Path, mirror_db: &Path, cycle: u64) -> SoakResult<()> {
    fs::write(rollout, [])?;
    update_cursor(
        mirror_db,
        MIRROR_THREAD,
        rollout.to_string_lossy().as_ref(),
        0,
        f64::from(u32::try_from(cycle)?),
    )?;
    Ok(())
}
