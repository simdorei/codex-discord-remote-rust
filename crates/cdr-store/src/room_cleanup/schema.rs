use crate::Result;
use rusqlite::Connection;

const GUARDED: [(&str, &str); 8] = [
    (
        "codex_goal_progress",
        "channel_id=NEW.channel OR (phase='deleting' AND target_thread_id=NEW.thread)",
    ),
    (
        "codex_commentary_outbox",
        "channel_id=NEW.channel_id OR (phase='deleting' AND target_thread_id=NEW.target_thread_id)",
    ),
    (
        "codex_turn_queue",
        "channel_id=NEW.channel_id OR (phase='deleting' AND target_thread_id=NEW.target_thread_id)",
    ),
    (
        "codex_prompt_intakes",
        "channel_id=NEW.channel_id OR (phase='deleting' AND target_thread_id=NEW.target_thread_id)",
    ),
    (
        "codex_delivery_outbox",
        "channel_id=NEW.channel_id OR (phase='deleting' AND target_thread_id=NEW.target_thread_id)",
    ),
    (
        "busy_choices",
        "channel_id=NEW.channel_id OR (phase='deleting' AND target_thread_id=NEW.target_thread_id)",
    ),
    (
        "mirror_threads",
        "channel_id=NEW.discord_thread_id OR channel_id=NEW.discord_channel_id OR (phase='deleting' AND target_thread_id=NEW.codex_thread_id)",
    ),
    ("mirror_projects", "channel_id=NEW.discord_channel_id"),
];

pub(crate) fn migrate_schema(connection: &Connection) -> Result<()> {
    super::archive_schema::migrate(connection)?;
    super::archived_rejections::migrate(connection)?;
    connection.execute_batch("CREATE TABLE IF NOT EXISTS cdr_cleanup_fences (channel_id INTEGER PRIMARY KEY CHECK(channel_id>0),target_thread_id TEXT,token TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('deleting','deleted')),created_at REAL NOT NULL);")?;
    for (table, predicate) in GUARDED {
        for operation in ["INSERT", "UPDATE"] {
            connection.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS cdr_cleanup_{table}_{operation} BEFORE {operation} ON {table} WHEN EXISTS(SELECT 1 FROM cdr_cleanup_fences WHERE {predicate}) BEGIN SELECT RAISE(ABORT,'room cleanup fence active; operation not executed'); END;"))?;
        }
    }
    connection.execute_batch("CREATE TRIGGER IF NOT EXISTS cdr_cleanup_ingress_save AFTER INSERT ON discord_ingress_journal WHEN EXISTS(SELECT 1 FROM cdr_cleanup_fences WHERE channel_id=NEW.channel_id OR (phase='deleting' AND target_thread_id=NEW.target_thread_id)) BEGIN UPDATE discord_ingress_journal SET state='held',phase='cleanup_fenced',hold_reason='room cleanup in progress or completed; original request saved without execution' WHERE ingress_id=NEW.ingress_id; END;
    CREATE TRIGGER IF NOT EXISTS cdr_cleanup_ingress_execute BEFORE UPDATE ON discord_ingress_journal WHEN NEW.state IN ('staged','acknowledged','executing','owned') AND EXISTS(SELECT 1 FROM cdr_cleanup_fences WHERE channel_id=NEW.channel_id OR (phase='deleting' AND target_thread_id=NEW.target_thread_id)) BEGIN SELECT RAISE(ABORT,'room cleanup fence active; ingress cannot execute'); END;
    CREATE TRIGGER IF NOT EXISTS cdr_cleanup_receipt BEFORE INSERT ON codex_delivery_receipts WHEN EXISTS(SELECT 1 FROM cdr_cleanup_fences WHERE CASE WHEN json_valid(NEW.receipt_key) THEN CASE WHEN json_type(NEW.receipt_key,'$[0]')='integer' THEN channel_id=json_extract(NEW.receipt_key,'$[0]') ELSE 1 END ELSE 1 END) BEGIN SELECT RAISE(ABORT,'room cleanup fence active; delivery not attempted'); END;")?;
    Ok(())
}

pub(crate) fn schema_current(connection: &Connection) -> Result<bool> {
    if !super::archive_schema::current(connection)?
        || !super::archived_rejections::schema_current(connection)?
    {
        return Ok(false);
    }
    let exists:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='cdr_cleanup_fences')",[],|r|r.get(0))?;
    if !exists {
        return Ok(false);
    }
    for (table, _) in GUARDED {
        for operation in ["INSERT", "UPDATE"] {
            if !has_trigger(connection, &format!("cdr_cleanup_{table}_{operation}"))? {
                return Ok(false);
            }
        }
    }
    for name in [
        "cdr_cleanup_ingress_save",
        "cdr_cleanup_ingress_execute",
        "cdr_cleanup_receipt",
    ] {
        if !has_trigger(connection, name)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn has_trigger(connection: &Connection, name: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='trigger' AND name=?)",
        [name],
        |r| r.get(0),
    )?)
}
