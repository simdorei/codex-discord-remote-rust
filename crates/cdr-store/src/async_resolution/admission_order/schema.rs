use super::{COMPONENT, FORMAT_VERSION, invalid};
use crate::Result;
use rusqlite::{Connection, OptionalExtension, params};

const SCHEMA: &str = include_str!("schema.sql");

fn normalized(value: &str) -> String {
    let value = value
        .trim()
        .strip_suffix(';')
        .unwrap_or(value.trim())
        .trim_end();
    for (prefix, replacement) in [
        ("CREATE TABLE IF NOT EXISTS ", "CREATE TABLE "),
        ("CREATE TRIGGER IF NOT EXISTS ", "CREATE TRIGGER "),
    ] {
        if let Some(body) = value.strip_prefix(prefix) {
            return format!("{replacement}{body}");
        }
    }
    value.to_owned()
}

fn object_count(db: &Connection) -> Result<i64> {
    Ok(db.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE name GLOB 'cdr_recovery_ingress_order*'",
        [],
        |row| row.get(0),
    )?)
}

fn requirement(db: &Connection) -> Result<Option<i64>> {
    Ok(db
        .query_row(
            "SELECT format_version FROM cdr_runtime_capability_requirements WHERE component=?",
            [COMPONENT],
            |row| row.get(0),
        )
        .optional()?)
}

fn catalog_matches(db: &Connection) -> Result<bool> {
    let mut count = 0_i64;
    for definition in SCHEMA
        .split("-- object --")
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        let mut words = definition.split_whitespace();
        if words.next() != Some("CREATE") {
            return Err(invalid("invalid built-in DDL"));
        }
        let kind = match words.next() {
            Some("TABLE") => "table",
            Some("TRIGGER") => "trigger",
            _ => return Err(invalid("unsupported built-in DDL")),
        };
        if [words.next(), words.next(), words.next()] != [Some("IF"), Some("NOT"), Some("EXISTS")] {
            return Err(invalid("invalid built-in object prefix"));
        }
        let name = words
            .next()
            .ok_or_else(|| invalid("missing built-in object name"))?;
        let actual: Option<String> = db
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type=? AND name=?",
                params![kind, name],
                |row| row.get(0),
            )
            .optional()?;
        if actual.is_none_or(|sql| normalized(&sql) != normalized(definition)) {
            return Ok(false);
        }
        count += 1;
    }
    Ok(object_count(db)? == count)
}

pub(crate) fn schema_current(db: &Connection) -> Result<bool> {
    Ok(catalog_matches(db)? && requirement(db)? == Some(FORMAT_VERSION))
}

pub(crate) fn migrate_schema(db: &Connection) -> Result<()> {
    // A partial/missing ledger with a persisted capability is lost history,
    // not permission to reset its monotonic boundary or repair it in place.
    if object_count(db)? != 0 || requirement(db)?.is_some() {
        return check_compatibility_in(db, FORMAT_VERSION);
    }
    db.execute_batch(SCHEMA)?;
    // Existing source rows get legacy markers, never invented fresh authority.
    // The ID-only legacy set is retained too, even after ordinary pruning.
    db.execute_batch(
        "INSERT INTO cdr_recovery_ingress_order(ingress_id,kind,event_id,origin)
         SELECT ingress_id,kind,event_id,'legacy' FROM discord_ingress_journal ORDER BY rowid;
         INSERT INTO cdr_recovery_ingress_order(ingress_id,kind,event_id,origin)
         SELECT NULL,'message',p.message_id,'legacy' FROM discord_processed_messages p
         WHERE NOT EXISTS(SELECT 1 FROM cdr_recovery_ingress_order o
             WHERE o.kind='message' AND o.event_id=p.message_id) ORDER BY p.message_id;",
    )?;
    let complete: bool = db.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM discord_ingress_journal j WHERE NOT EXISTS(
            SELECT 1 FROM cdr_recovery_ingress_order o WHERE o.ingress_id IS j.ingress_id
            AND o.kind=j.kind AND o.event_id IS j.event_id AND o.origin='legacy'))
         AND NOT EXISTS(SELECT 1 FROM discord_processed_messages p WHERE NOT EXISTS(
            SELECT 1 FROM cdr_recovery_ingress_order o WHERE o.kind='message' AND o.event_id=p.message_id))",
        [], |row|row.get(0),
    )?;
    if !complete {
        return Err(invalid("legacy admission inventory was not retained"));
    }
    if db.execute(
        "INSERT INTO cdr_runtime_capability_requirements(component,format_version) VALUES(?,?)",
        params![COMPONENT, FORMAT_VERSION],
    )? != 1
    {
        return Err(invalid("required capability was not retained"));
    }
    check_compatibility_in(db, FORMAT_VERSION)
}

/// Read-only storage compatibility. This neither grants a release nor checks
/// operating readiness. A genuinely absent legacy family is supported as absent.
pub fn check_compatibility_in(db: &Connection, required: i64) -> Result<()> {
    if required == 0 && object_count(db)? == 0 {
        return Ok(());
    }
    if required != FORMAT_VERSION || !schema_current(db)? {
        return Err(invalid("unsupported or incomplete durable admission order"));
    }
    Ok(())
}
