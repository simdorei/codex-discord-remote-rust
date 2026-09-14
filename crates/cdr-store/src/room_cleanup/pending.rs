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
    // Old installations predate this table. Missing table means no async records;
    // a malformed present table/query still fails closed.
    if connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='cdr_async_questions')", [], |r|r.get::<_,bool>(0))? && connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_async_questions WHERE (channel_id=?1 OR thread_id=?2) AND state IN ('observed','open','dispatching'))",
        params![channel,target], |r|r.get::<_,bool>(0))? {
        return Ok(Some("unanswered or unconfirmed async question"));
    }
    if connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='cdr_async_question_inbox')", [], |r|r.get::<_,bool>(0))? && connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_async_question_inbox WHERE (candidate_channel_id=?1 OR thread_id=?2) AND state='waiting')",
        params![channel,target], |r|r.get::<_,bool>(0))? {
        return Ok(Some("unbound async question awaiting original ownership"));
    }
    for (reason, table, extra) in [
        ("queued requests", "codex_turn_queue", ""),
        ("prompt intake", "codex_prompt_intakes", ""),
        (
            "ingress",
            "discord_ingress_journal",
            "AND (state!='completed' OR confirmation_delivered=0)",
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
            sql.push_str(" AND (?3 IS NULL OR ingress_id!=?3))");
            connection.query_row(&sql, params![channel, target, confirmation], |r| {
                r.get::<_, bool>(0)
            })?
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
