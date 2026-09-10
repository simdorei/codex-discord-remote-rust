use crate::Result;
use rusqlite::Connection;

const GUARDED: [(&str, &str); 7] = [
    (
        "codex_goal_progress",
        "target_thread_id=NEW.thread OR NEW.channel IN (SELECT value FROM json_each(channels))",
    ),
    (
        "codex_commentary_outbox",
        "target_thread_id=NEW.target_thread_id OR NEW.channel_id IN (SELECT value FROM json_each(channels))",
    ),
    (
        "codex_turn_queue",
        "target_thread_id=NEW.target_thread_id OR NEW.channel_id IN (SELECT value FROM json_each(channels))",
    ),
    (
        "codex_prompt_intakes",
        "target_thread_id=NEW.target_thread_id OR NEW.channel_id IN (SELECT value FROM json_each(channels))",
    ),
    (
        "codex_delivery_outbox",
        "target_thread_id=NEW.target_thread_id OR NEW.channel_id IN (SELECT value FROM json_each(channels))",
    ),
    (
        "busy_choices",
        "target_thread_id=NEW.target_thread_id OR NEW.channel_id IN (SELECT value FROM json_each(channels))",
    ),
    (
        "mirror_threads",
        "target_thread_id=NEW.codex_thread_id OR NEW.discord_thread_id IN (SELECT value FROM json_each(channels))",
    ),
];

pub(super) fn migrate(connection: &Connection) -> Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS cdr_archive_fences(target_thread_id TEXT PRIMARY KEY, token TEXT NOT NULL, channels TEXT NOT NULL CHECK(json_valid(channels)),phase TEXT NOT NULL CHECK(phase IN ('deleting','deleted')),created_at REAL NOT NULL);")?;
    for (table, predicate) in GUARDED {
        for operation in ["INSERT", "UPDATE"] {
            connection.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS cdr_archive_{table}_{operation} BEFORE {operation} ON {table} WHEN EXISTS(SELECT 1 FROM cdr_archive_fences WHERE {predicate}) BEGIN SELECT RAISE(ABORT,'archive deletion fenced; operation not executed'); END;"))?;
        }
    }
    connection.execute_batch("CREATE TRIGGER IF NOT EXISTS cdr_archive_ingress_save AFTER INSERT ON discord_ingress_journal WHEN EXISTS(SELECT 1 FROM cdr_archive_fences WHERE target_thread_id=NEW.target_thread_id OR NEW.channel_id IN (SELECT value FROM json_each(channels))) BEGIN UPDATE discord_ingress_journal SET state='held',phase='archive_fenced',hold_reason='archive deletion in progress or completed; original request saved without execution' WHERE ingress_id=NEW.ingress_id; END;
    CREATE TRIGGER IF NOT EXISTS cdr_archive_ingress_execute BEFORE UPDATE ON discord_ingress_journal WHEN NEW.state IN ('staged','acknowledged','executing','owned') AND EXISTS(SELECT 1 FROM cdr_archive_fences WHERE target_thread_id=NEW.target_thread_id OR NEW.channel_id IN (SELECT value FROM json_each(channels))) BEGIN SELECT RAISE(ABORT,'archive deletion fenced; ingress cannot execute'); END;
    CREATE TRIGGER IF NOT EXISTS cdr_archive_receipt BEFORE INSERT ON codex_delivery_receipts WHEN EXISTS(SELECT 1 FROM cdr_archive_fences WHERE CASE WHEN json_valid(NEW.receipt_key) THEN CASE WHEN json_type(NEW.receipt_key,'$[0]')='integer' THEN json_extract(NEW.receipt_key,'$[0]') IN (SELECT value FROM json_each(channels)) ELSE 1 END ELSE 1 END) BEGIN SELECT RAISE(ABORT,'archive deletion fenced; delivery not attempted'); END;")?;
    Ok(())
}

pub(super) fn current(connection: &Connection) -> Result<bool> {
    if !connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='cdr_archive_fences')",[],|r|r.get::<_,bool>(0))? {
        return Ok(false);
    }
    for (table, _) in GUARDED {
        for operation in ["INSERT", "UPDATE"] {
            if !super::schema::has_trigger(connection, &format!("cdr_archive_{table}_{operation}"))?
            {
                return Ok(false);
            }
        }
    }
    for name in [
        "cdr_archive_ingress_save",
        "cdr_archive_ingress_execute",
        "cdr_archive_receipt",
    ] {
        if !super::schema::has_trigger(connection, name)? {
            return Ok(false);
        }
    }
    Ok(true)
}
