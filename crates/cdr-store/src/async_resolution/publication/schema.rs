use rusqlite::{Connection, OptionalExtension, params};

use super::{COMPONENT, FORMAT_VERSION, invalid};
use crate::Result;

pub(crate) fn migrate_schema(db: &Connection) -> Result<()> {
    db.execute_batch(include_str!("schema.sql"))?;
    db.execute(
        "INSERT OR IGNORE INTO cdr_runtime_capability_requirements(component,format_version)
         VALUES(?,?)",
        params![COMPONENT, FORMAT_VERSION],
    )?;
    // RAISE(IGNORE) must not leave the new ledger without a launch requirement.
    db.query_row(
        "SELECT format_version FROM cdr_runtime_capability_requirements
         WHERE component=? AND format_version>=?",
        params![COMPONENT, FORMAT_VERSION],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(())
}

pub(crate) fn schema_current(db: &Connection) -> Result<bool> {
    let present: bool = db.query_row(
        "SELECT count(*)=10 FROM sqlite_schema WHERE name IN (
         'cdr_recovery_publication_proposals','cdr_recovery_publication_deliveries',
         'cdr_recovery_publication_decisions','cdr_recovery_publication_job_revision',
         'cdr_recovery_publication_proposal_immutable','cdr_recovery_publication_proposal_no_delete',
         'cdr_recovery_publication_delivery_immutable','cdr_recovery_publication_delivery_no_delete',
         'cdr_recovery_publication_decision_immutable','cdr_recovery_publication_decision_no_delete')",
        [], |r| r.get(0),
    )?;
    if !present {
        return Ok(false);
    }
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_runtime_capability_requirements
         WHERE component=? AND format_version>=?)",
        params![COMPONENT, FORMAT_VERSION],
        |r| r.get(0),
    )?)
}

/// Read-only launch check. An empty legacy DB without this ledger is compatible;
/// a partial or unsupported ledger is not silently repaired by the probe.
pub fn check_compatibility_in(db: &Connection, required: i64) -> Result<()> {
    let count: i64 = db.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name IN (
         'cdr_recovery_publication_proposals','cdr_recovery_publication_deliveries',
         'cdr_recovery_publication_decisions')",
        [],
        |r| r.get(0),
    )?;
    if required == 0 && count == 0 {
        return Ok(());
    }
    let persisted: Option<i64> = db
        .query_row(
            "SELECT format_version FROM cdr_runtime_capability_requirements WHERE component=?",
            [COMPONENT],
            |r| r.get(0),
        )
        .optional()?;
    if required != FORMAT_VERSION || persisted != Some(required) || !schema_current(db)? {
        return Err(invalid(
            "unsupported or incomplete consent ledger capability",
        ));
    }
    let unsupported: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_recovery_publication_proposals
         WHERE format_version!=? OR revision<1)",
        [FORMAT_VERSION],
        |r| r.get(0),
    )?;
    if unsupported {
        return Err(invalid("unsupported stored consent proposal"));
    }
    // Validate the consumed column contract, including empty tables.
    db.prepare(
        "SELECT id,format_version,revision,job_id,target_thread_id,
        owner_user_id,channel_id,application_id,seal_json,seal_sha256
        FROM cdr_recovery_publication_proposals LIMIT 0",
    )?;
    db.prepare(
        "SELECT proposal_id,revision,message_id,body_sha256
        FROM cdr_recovery_publication_deliveries LIMIT 0",
    )?;
    db.prepare(
        "SELECT proposal_id,revision,ingress_id,interaction_id,decision,recorded_at_bits
        FROM cdr_recovery_publication_decisions LIMIT 0",
    )?;
    Ok(())
}
