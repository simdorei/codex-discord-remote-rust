//! Offline rehearsal template: replace example paths and IDs only after review.
use rusqlite::{Connection, OpenFlags, types::Value};
use std::{error::Error, path::PathBuf};

fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn rows(db: &Connection, table: &str, columns: &[String]) -> rusqlite::Result<Vec<String>> {
    let sql = format!(
        "SELECT {} FROM {}",
        columns
            .iter()
            .map(|v| quoted(v))
            .collect::<Vec<_>>()
            .join(","),
        quoted(table)
    );
    let mut statement = db.prepare(&sql)?;
    let mut result = statement
        .query_map([], |row| {
            let values = (0..columns.len())
                .map(|index| row.get::<_, Value>(index))
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(format!("{values:?}"))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    result.sort();
    Ok(result)
}

fn main() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from("C:/example/codex-discord-remote-rust");
    let source = Connection::open_with_flags(
        root.join("discord_mirror.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let output = root.join(".codex-discord-backups/mirror-migration-rehearsal-20260909.sqlite");
    if output.exists() {
        return Err("Existing rehearsal preserved; refusing overwrite".into());
    }
    source.backup(rusqlite::MAIN_DB, &output, None)?;
    drop(source);
    let before = Connection::open_with_flags(&output, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let tables = before
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut evidence = Vec::new();
    for table in &tables {
        let columns = before
            .prepare(&format!("PRAGMA table_info({})", quoted(table)))?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        evidence.push((
            table.clone(),
            columns.clone(),
            rows(&before, table, &columns)?,
        ));
    }
    drop(before);
    let migrated = cdr_store::schema::open_initialized(&output)?;
    cdr_store::schema::assert_integrity(&migrated)?;
    let mut total_rows = 0;
    for (table, columns, original) in evidence {
        if rows(&migrated, &table, &columns)? != original {
            return Err(
                format!("Existing table contents changed during migration: {table}").into(),
            );
        }
        total_rows += original.len();
    }
    drop(migrated);
    let pending = cdr_store::room_cleanup::pending_reason(
        &output,
        100,
        Some("00000000-0000-0000-0000-000000000000"),
    )?;
    println!(
        "migration_integrity=ok; preserved_tables={}; preserved_rows={total_rows}; target_pending_reason={pending:?}",
        tables.len()
    );
    println!(
        "production_opened_read_only=true; rehearsal={}",
        output.display()
    );
    Ok(())
}
