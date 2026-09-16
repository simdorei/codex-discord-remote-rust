use crate::Result;
use rusqlite::{Connection, params};

pub(super) fn reason(
    connection: &Connection,
    channel: i64,
    target: Option<&str>,
) -> Result<Option<&'static str>> {
    reason_except(connection, channel, target, None)
}

pub(super) fn reason_except(
    connection: &Connection,
    channel: i64,
    target: Option<&str>,
    confirmation: Option<&str>,
) -> Result<Option<&'static str>> {
    reason_for_schema(connection, channel, target, confirmation, false)
}

pub(super) fn reason_for_schema(
    connection: &Connection,
    channel: i64,
    target: Option<&str>,
    confirmation: Option<&str>,
    allow_pre_commentary_schema: bool,
) -> Result<Option<&'static str>> {
    reason_with_exclusions(
        connection,
        channel,
        target,
        confirmation,
        allow_pre_commentary_schema,
        &[],
    )
}

pub(super) fn reason_with_exclusions(
    connection: &Connection,
    channel: i64,
    target: Option<&str>,
    confirmation: Option<&str>,
    allow_pre_commentary_schema: bool,
    excluded: &[String],
) -> Result<Option<&'static str>> {
    for (reason, table, extra) in [
        ("queued requests", "codex_turn_queue", ""),
        ("prompt intake", "codex_prompt_intakes", ""),
        (
            "ingress",
            "discord_ingress_journal",
            // An acknowledged prompt handoff stays `owned` permanently. Its
            // intake/queue/delivery machinery, checked independently here, owns
            // any unfinished work. CASE is deliberate: nullable/malformed owner
            // fields must not disappear through SQL's three-valued NOT logic.
            "AND CASE
                WHEN state='completed' AND confirmation_delivered=1 THEN 0
                WHEN state='owned' AND confirmation_delivered=1
                    AND owner_kind='prompt' AND length(trim(owner_id))>0 THEN 0
                ELSE 1 END",
        ),
        ("undelivered result", "codex_delivery_outbox", ""),
        ("undelivered progress", "codex_commentary_outbox", ""),
        (
            "busy choice",
            "busy_choices",
            "AND claimed_at IS NULL AND expires_at > unixepoch()",
        ),
    ] {
        // This optional feature's table was added after the installed baseline.
        // No table means no rows, not permission to ignore query/column errors.
        if allow_pre_commentary_schema && table == "codex_commentary_outbox" {
            let exists: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='codex_commentary_outbox')",
                [], |row| row.get(0),
            )?;
            if !exists {
                continue;
            }
        }
        let mut sql = format!(
            "SELECT EXISTS(SELECT 1 FROM {table} WHERE (channel_id=?1 OR (?2 IS NOT NULL AND target_thread_id=?2)) {extra})"
        );
        let exists = if table == "discord_ingress_journal" {
            sql.pop();
            sql.push_str(" AND (?3 IS NULL OR ingress_id!=?3) AND ingress_id NOT IN (SELECT value FROM json_each(?4)))");
            connection.query_row(
                &sql,
                params![
                    channel,
                    target,
                    confirmation,
                    serde_json::to_string(excluded)?
                ],
                |r| r.get::<_, bool>(0),
            )?
        } else {
            connection.query_row(&sql, params![channel, target], |r| r.get::<_, bool>(0))?
        };
        if exists {
            return Ok(Some(reason));
        }
    }
    if connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_goal_progress
        WHERE channel=?1 OR (?2 IS NOT NULL AND thread=?2))",
        params![channel, target],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(Some("undelivered goal progress"));
    }
    let mut statement = connection
        .prepare("SELECT receipt_key FROM codex_delivery_receipts WHERE message_id IS NULL")?;
    for key in statement.query_map([], |r| r.get::<_, String>(0))? {
        let channel_id = serde_json::from_str::<serde_json::Value>(&key?)
            .ok()
            .and_then(|v| {
                v.as_array()
                    .and_then(|a| a.first())
                    .and_then(serde_json::Value::as_i64)
            });
        if channel_id.is_none() {
            return Ok(Some(
                "unattributable delivery receipt (invalid or missing channel identity)",
            ));
        }
        if channel_id == Some(channel) {
            return Ok(Some(
                "unsettled delivery receipt (unknown, retryable or blocked)",
            ));
        }
    }
    Ok(None)
}
