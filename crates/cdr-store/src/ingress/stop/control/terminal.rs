use crate::Result;
use rusqlite::{Connection, params};
use serde_json::Value;

/// Only an exact terminal notification from the accepting resident can settle.
/// Interrupt ACK, missing queue rows, process replacement and wall time cannot.
pub(crate) fn record_terminal_in(
    db: &Connection,
    thread: &str,
    turn: &str,
    generation: i64,
    resident: &str,
    payload: &str,
) -> Result<()> {
    let Ok(value) = serde_json::from_str::<Value>(payload) else {
        return Ok(());
    };
    if value["threadId"] != thread
        || value["turn"]["id"] != turn
        || !matches!(
            value["turn"]["status"].as_str(),
            Some("completed" | "interrupted" | "failed")
        )
    {
        return Ok(());
    }
    db.execute("UPDATE cdr_stop_controls SET terminal_json=?1,
        phase=CASE WHEN json_extract(record_json,'$.can_settle')=1
            AND NOT EXISTS(SELECT 1 FROM json_each(record_json,'$.jobs') j
                WHERE NOT EXISTS(SELECT 1 FROM cdr_execution_holds h
                    WHERE h.job_id=json_extract(j.value,'$.job_id')
                    AND h.target_thread_id=cdr_stop_controls.target_thread_id))
            THEN 'settled' ELSE 'unknown' END
        WHERE target_thread_id=?2 AND turn_id=?3 AND generation=?4 AND resident_owner=?5
        AND EXISTS(SELECT 1 FROM codex_turn_queue q WHERE q.target_thread_id=?2 AND q.turn_id=?3
            AND COALESCE(q.turn_observation_generation,q.app_server_generation)=?4 AND q.state='running')",
        params![payload,thread,turn,generation,resident])?;
    Ok(())
}
