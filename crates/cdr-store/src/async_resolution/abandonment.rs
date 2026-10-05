//! Append-only storage foundation for exact no-replay abandonment.
//!
//! Exact persisted user consent may atomically abandon one held Pending job.
//! This never releases a policy or issues an RPC; the runtime route is a separate gate.
use rusqlite::{Connection, OptionalExtension, params};

use crate::{Result, StoreError};

mod api;
mod binding;
mod decision;
mod identity;
mod proposal;
pub mod readiness;
mod routing;
mod snapshot;
pub use api::{
    Decision, DecisionInput, DecisionReceipt, DeliveredProposal, Proposal, ProposalInput,
};
pub use binding::delivered_proposal;
pub use decision::{decision_status, record_decision};
pub use proposal::{bind_delivery, propose};
pub use routing::{DecisionRouteInput, authorize_decision, command_target};

pub const COMPONENT: &str = "recovery_abandonment";
pub const FORMAT_VERSION: i64 = 1;
const SCHEMA: &str = include_str!("abandonment/schema.sql");

fn invalid(reason: &str) -> StoreError {
    StoreError::Integrity(format!("recovery abandonment storage held: {reason}"))
}

fn normalized_sql(value: &str) -> String {
    // Match the fixed built-in DDL, not general SQL semantic equivalence.
    // Only SQLite's optional creation prefix and outer terminator are elided;
    // every literal, quoted identifier and body byte remains significant.
    let value = value.trim();
    let value = value.strip_suffix(';').unwrap_or(value).trim_end();
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

fn definitions() -> impl Iterator<Item = &'static str> {
    SCHEMA
        .split("-- object --")
        .map(str::trim)
        .filter(|part| !part.is_empty())
}

fn identity(statement: &str) -> Result<(&str, &str)> {
    let mut words = statement.split_whitespace();
    if words.next() != Some("CREATE") {
        return Err(invalid("invalid built-in schema definition"));
    }
    let kind = match words.next() {
        Some("TABLE") => "table",
        Some("TRIGGER") => "trigger",
        _ => return Err(invalid("unsupported built-in schema object")),
    };
    if [words.next(), words.next(), words.next()] != [Some("IF"), Some("NOT"), Some("EXISTS")] {
        return Err(invalid("invalid built-in object prefix"));
    }
    let name = words
        .next()
        .ok_or_else(|| invalid("missing built-in object name"))?;
    Ok((kind, name))
}

fn object_count(db: &Connection) -> Result<i64> {
    Ok(db.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE name GLOB 'cdr_recovery_abandonment_*'",
        [],
        |row| row.get(0),
    )?)
}

fn catalog_matches(db: &Connection) -> Result<bool> {
    let mut count = 0_i64;
    for definition in definitions() {
        let (kind, name) = identity(definition)?;
        let actual: Option<String> = db
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type=? AND name=?",
                params![kind, name],
                |row| row.get(0),
            )
            .optional()?;
        if actual.is_none_or(|sql| normalized_sql(&sql) != normalized_sql(definition)) {
            return Ok(false);
        }
        count += 1;
    }
    Ok(object_count(db)? == count)
}

pub(crate) fn schema_current(db: &Connection) -> Result<bool> {
    if !catalog_matches(db)? {
        return Ok(false);
    }
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_runtime_capability_requirements
         WHERE component=? AND format_version=?)",
        params![COMPONENT, FORMAT_VERSION],
        |row| row.get(0),
    )?)
}

pub(crate) fn migrate_schema(db: &Connection) -> Result<()> {
    db.execute_batch(SCHEMA)?;
    db.execute(
        "INSERT OR IGNORE INTO cdr_runtime_capability_requirements(component,format_version)
         VALUES(?,?)",
        params![COMPONENT, FORMAT_VERSION],
    )?;
    check_compatibility_in(db, FORMAT_VERSION)
}

/// Read-only format and structural compatibility, never application authority.
/// A pristine legacy database may lack this feature. A partial or changed family
/// is rejected without migration, including empty malformed tables and triggers.
pub fn check_compatibility_in(db: &Connection, required: i64) -> Result<()> {
    if required == 0 && object_count(db)? == 0 {
        return Ok(());
    }
    if required != FORMAT_VERSION || !schema_current(db)? {
        return Err(invalid("unsupported or incomplete abandonment capability"));
    }
    db.prepare(
        "SELECT id,format_version,revision,job_id,target_thread_id,owner_user_id,
        channel_id,application_id,seal_json,seal_sha256
        FROM cdr_recovery_abandonment_proposals LIMIT 0",
    )?;
    db.prepare(
        "SELECT proposal_id,revision,message_id,body_sha256
        FROM cdr_recovery_abandonment_deliveries LIMIT 0",
    )?;
    db.prepare(
        "SELECT proposal_id,revision,ingress_id,interaction_id,decision,recorded_at_bits
        FROM cdr_recovery_abandonment_decisions LIMIT 0",
    )?;
    let malformed: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_recovery_abandonment_proposals WHERE format_version!=1 OR revision<1)
         OR EXISTS(SELECT 1 FROM cdr_recovery_abandonment_deliveries d WHERE NOT EXISTS(
            SELECT 1 FROM cdr_recovery_abandonment_proposals p WHERE p.id=d.proposal_id AND p.revision=d.revision))
         OR EXISTS(SELECT 1 FROM cdr_recovery_abandonment_decisions d WHERE NOT EXISTS(
            SELECT 1 FROM cdr_recovery_abandonment_proposals p WHERE p.id=d.proposal_id AND p.revision=d.revision))",
        [], |row| row.get(0),
    )?;
    if malformed {
        return Err(invalid(
            "stored abandonment evidence has no supported original proposal",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::normalized_sql;

    #[test]
    fn catalog_prefix_and_outer_terminator_preserve_the_body() {
        for kind in ["TABLE", "TRIGGER"] {
            let body = r#""Mixed Name" ('Keep Case', 'Keep Space', 'ifnotexists')"#;
            let expected = format!("CREATE {kind} {body}");
            assert_eq!(
                normalized_sql(&format!(" \nCREATE {kind} IF NOT EXISTS {body};\n ")),
                expected,
            );
            assert_eq!(normalized_sql(&expected), expected);
        }
    }

    #[test]
    fn literal_and_quoted_identifier_contents_are_not_folded() {
        for (original, changed) in [
            ("'abandon_only'", "'abandon_ only'"),
            ("'abandon_only'", "'ABANDON_ONLY'"),
            ("'abandon_only'", "'abandon_ifnotexistsonly'"),
            (r#""Mixed Name""#, r#""mixed name""#),
            ("'Keep ''Quoted'' Value'", "'keep ''quoted'' value'"),
        ] {
            assert_ne!(normalized_sql(original), normalized_sql(changed));
        }
    }

    #[test]
    fn noncanonical_layout_is_not_claimed_equivalent() {
        assert_ne!(
            normalized_sql("CREATE TABLE t (v TEXT)"),
            normalized_sql("CREATE TABLE t ( v TEXT )"),
        );
    }
}
