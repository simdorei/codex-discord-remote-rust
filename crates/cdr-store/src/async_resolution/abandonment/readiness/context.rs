//! Snapshot current facts without reinterpreting old request-specific holds.
use super::super::{api::Proposal, identity, invalid, snapshot};
use crate::{Result, async_resolution as resolution};
use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Serialize)]
pub(super) struct Policy {
    pub(super) origin_job: String,
    pub(super) original_turn: String,
    registration: Value,
}

pub(super) fn policy_in(db: &Connection, proposal: &Proposal) -> Result<Policy> {
    let (version, kind, review, turn, origin, pending): (
        i64,
        String,
        String,
        String,
        String,
        String,
    ) = db.query_row(
        "SELECT format_version,policy,proposal_sha256,original_turn_id,origin_job_id,pending_job_id
         FROM cdr_async_recovery_policies WHERE thread_id=?",
        [&proposal.thread_id],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        },
    )?;
    if version != resolution::RECOVERY_POLICY_FORMAT_VERSION
        || kind != "publishing_recovery"
        || review != resolution::REVIEWED_PROPOSAL_SHA256
        || pending != proposal.job_id
        || origin == pending
        || origin.trim().is_empty()
        || turn.trim().is_empty()
        || origin.len() > 128
        || turn.len() > 128
    {
        return Err(invalid(
            "registered policy does not identify the exact disposed successor",
        ));
    }
    Ok(Policy {
        registration: json!([version, kind, review, turn, origin, pending]),
        origin_job: origin,
        original_turn: turn,
    })
}

fn capabilities(db: &Connection) -> Result<()> {
    if !resolution::schema_current(db)? {
        return Err(invalid("original execution schema is unavailable"));
    }
    let values = db.prepare("SELECT component,format_version FROM cdr_runtime_capability_requirements ORDER BY component LIMIT 65")?
        .query_map([], |r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let expected = [
        ("async_resolution", resolution::FORMAT_VERSION),
        (
            resolution::RECOVERY_POLICY_COMPONENT,
            resolution::RECOVERY_POLICY_FORMAT_VERSION,
        ),
        (
            resolution::publication::COMPONENT,
            resolution::publication::FORMAT_VERSION,
        ),
        (
            resolution::abandonment::COMPONENT,
            resolution::abandonment::FORMAT_VERSION,
        ),
        (
            resolution::admission_order::COMPONENT,
            resolution::admission_order::FORMAT_VERSION,
        ),
    ];
    if values.len() != expected.len()
        || expected.iter().any(|(component, version)| {
            !values
                .iter()
                .any(|(name, actual)| name.as_str() == *component && actual == version)
        })
    {
        return Err(invalid(
            "current runtime capability is unknown or unsupported",
        ));
    }
    resolution::publication::check_compatibility_in(db, resolution::publication::FORMAT_VERSION)?;
    resolution::admission_order::check_compatibility_in(
        db,
        resolution::admission_order::FORMAT_VERSION,
    )
}

fn barriers(db: &Connection, proposal: &Proposal) -> Result<()> {
    let mapped: bool = db.query_row(
        "SELECT count(*)=1 AND MAX(codex_thread_id=?1 AND discord_thread_id=?2)
         FROM mirror_threads WHERE codex_thread_id=?1 OR discord_thread_id=?2",
        params![proposal.thread_id, proposal.channel_id],
        |r| r.get(0),
    )?;
    let blocked: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_turn_queue WHERE target_thread_id=?1
            AND (state!='pending' OR turn_id IS NOT NULL OR goal_waiting!=0))
         OR EXISTS(SELECT 1 FROM codex_archive_fences WHERE target_thread_id=?1)
         OR EXISTS(SELECT 1 FROM cdr_cleanup_fences WHERE target_thread_id=?1 OR channel_id=?2)
         OR EXISTS(SELECT 1 FROM codex_dead_generation_holds WHERE target_thread_id=?1)
         OR EXISTS(SELECT 1 FROM codex_mutation_attempts WHERE state='prepared' AND (scoped=0 OR target_thread_id=?1))
         OR EXISTS(SELECT 1 FROM cdr_async_questions WHERE thread_id=?1 AND state IN ('open','dispatching'))",
        params![proposal.thread_id,proposal.channel_id], |r|r.get(0),
    )?;
    if !mapped || blocked || resolution::lifecycle::admission_held_in(db, &proposal.thread_id)? {
        return Err(invalid(
            "mapping, original execution or independent lifecycle evidence remains unresolved",
        ));
    }
    crate::ingress::stop::control::require_unheld_in(db, &proposal.thread_id)
}

pub(super) fn capture_in(db: &Connection, proposal: &Proposal) -> Result<Value> {
    capabilities(db)?;
    barriers(db, proposal)?;
    let mut facts = serde_json::Map::new();
    let mut budget = 0;
    for (key,sql) in [
        ("mapping","SELECT * FROM mirror_threads WHERE codex_thread_id=?1 OR discord_thread_id=?2 ORDER BY codex_thread_id LIMIT 3"),
        ("pending","SELECT * FROM codex_turn_queue WHERE target_thread_id=?1 ORDER BY created_at,job_id LIMIT 129"),
        ("intakes","SELECT * FROM codex_prompt_intakes WHERE target_thread_id=?1 ORDER BY rowid LIMIT 129"),
        ("obligations","SELECT * FROM cdr_async_execution_obligations WHERE thread_id=?1 ORDER BY question_id LIMIT 129"),
        ("questions","SELECT * FROM cdr_async_questions WHERE thread_id=?1 ORDER BY id LIMIT 129"),
        ("settlements","SELECT * FROM cdr_async_terminal_settlements WHERE question_id IN
            (SELECT question_id FROM cdr_async_execution_obligations WHERE thread_id=?1) ORDER BY question_id LIMIT 129"),
        ("handoffs","SELECT * FROM cdr_async_execution_handoffs WHERE question_id IN
            (SELECT question_id FROM cdr_async_execution_obligations WHERE thread_id=?1) ORDER BY question_id,revision LIMIT 129"),
        ("candidates","SELECT * FROM cdr_async_terminal_candidates WHERE question_id IN
            (SELECT question_id FROM cdr_async_execution_obligations WHERE thread_id=?1) ORDER BY question_id,revision,kind,evidence_sha256 LIMIT 129"),
        ("controls","SELECT * FROM cdr_stop_controls WHERE target_thread_id=?1 ORDER BY sequence LIMIT 129"),
        ("request_holds","SELECT * FROM cdr_execution_holds WHERE target_thread_id=?1 ORDER BY job_id LIMIT 129"),
        ("cancellations","SELECT * FROM codex_request_cancellations WHERE target_thread_id=?1 ORDER BY job_id LIMIT 129"),
        ("ingress","SELECT * FROM discord_ingress_journal WHERE (target_thread_id=?1 OR target_thread_id IS NULL)
            AND state!='completed' ORDER BY ingress_id LIMIT 129"),
        ("order","SELECT o.* FROM cdr_recovery_ingress_order o JOIN discord_ingress_journal j ON j.ingress_id=o.ingress_id
            WHERE j.target_thread_id=?1 AND j.state!='completed' ORDER BY o.sequence LIMIT 129"),
    ] {
        let mut statement = db.prepare(sql)?;
        statement.raw_bind_parameter(1, &proposal.thread_id)?;
        if statement.parameter_count() == 2 { statement.raw_bind_parameter(2, proposal.channel_id)?; }
        facts.insert(key.into(), snapshot::rows(&mut statement, &mut budget)?);
    }
    let mut caps = db
        .prepare("SELECT * FROM cdr_runtime_capability_requirements ORDER BY component LIMIT 65")?;
    facts.insert(
        "capabilities".into(),
        snapshot::rows(&mut caps, &mut budget)?,
    );
    facts.insert(
        "stop_origin".into(),
        crate::ingress::stop::revision::capture_in(db, Some(&proposal.thread_id))?,
    );
    facts.insert("runtime".into(), identity::runtime_in(db)?);
    Ok(Value::Object(facts))
}
