//! Durable state for automatic ordinary-quota/Reserve switching.
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::Result;

pub mod alignment;
pub mod start_notice;
pub mod transition_notice;
pub mod usage_fence;

pub const HOLD_PREFIX: &str = "[cdr-rust:auto-reserve-hold:v1] ";

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Policy {
    pub thread_id: String,
    pub mode: String,
    pub state: String,
    pub account_id: Option<String>,
    pub process_id: Option<i64>,
    pub generation: Option<i64>,
    pub previous_model: Option<String>,
    pub previous_effort: Option<String>,
    pub previous_effort_present: bool,
    pub previous_tier: Option<String>,
    pub applied_model: Option<String>,
    pub applied_effort: Option<String>,
    pub applied_tier: Option<String>,
    pub revision: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EpisodeClaim {
    pub revision: i64,
    pub usage_failure: Option<usage_fence::Claim>,
}

pub fn migrate_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS codex_reserve_policy (
            thread_id TEXT PRIMARY KEY,
            mode TEXT NOT NULL CHECK(mode IN ('auto','on','off','manual')),
            state TEXT NOT NULL CHECK(state IN ('ordinary','entering','reserve','restoring','held','unknown')),
            account_id TEXT,
            process_id INTEGER,
            generation INTEGER,
            previous_model TEXT,
            previous_effort TEXT,
            previous_effort_present INTEGER NOT NULL DEFAULT 0,
            previous_tier TEXT,
            applied_model TEXT,
            applied_effort TEXT,
            applied_tier TEXT,
            revision INTEGER NOT NULL DEFAULT 0,
            updated_at REAL NOT NULL DEFAULT (unixepoch())
        );
        CREATE INDEX IF NOT EXISTS codex_reserve_policy_recovery
            ON codex_reserve_policy(state, updated_at);",
    )?;
    for column in [
        "previous_effort_present",
        "applied_model",
        "applied_effort",
        "applied_tier",
        "usage_failure_state",
        "usage_failure_revision",
        "usage_failure_id",
        "usage_failure_reason",
        "usage_failure_resolution_reason",
        "usage_failure_updated_at",
    ] {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('codex_reserve_policy') WHERE name=?1)",
            [column],
            |row| row.get(0),
        )?;
        if !exists {
            let definition = match column {
                "previous_effort_present" | "usage_failure_id" => "INTEGER NOT NULL DEFAULT 0",
                "usage_failure_revision" => "INTEGER",
                "usage_failure_updated_at" => "REAL",
                _ => "TEXT",
            };
            connection.execute(
                &format!("ALTER TABLE codex_reserve_policy ADD COLUMN {column} {definition}"),
                [],
            )?;
        }
    }
    start_notice::migrate(connection)?;
    transition_notice::migrate(connection)
}

pub fn schema_current(connection: &Connection) -> Result<bool> {
    let current: bool = connection.query_row(
        "SELECT (SELECT COUNT(*) FROM pragma_table_info('codex_reserve_policy')
         WHERE name IN ('thread_id','mode','state','account_id','process_id','generation',
         'previous_model','previous_effort','previous_effort_present','previous_tier',
         'applied_model','applied_effort','applied_tier','revision','updated_at',
         'usage_failure_state','usage_failure_revision','usage_failure_reason',
         'usage_failure_id','usage_failure_resolution_reason','usage_failure_updated_at'))=21
         AND EXISTS(SELECT 1 FROM sqlite_schema WHERE type='index' AND name='codex_reserve_policy_recovery')",
        [],
        |row| row.get(0),
    )?;
    Ok(current
        && start_notice::schema_current(connection)?
        && transition_notice::schema_current(connection)?)
}

pub fn get(path: &Path, thread_id: &str) -> Result<Option<Policy>> {
    let connection = crate::schema::open_initialized(path)?;
    get_connection(&connection, thread_id)
}

pub fn ensure(path: &Path, thread_id: &str) -> Result<Policy> {
    let connection = crate::schema::open_initialized(path)?;
    connection.execute(
        "INSERT INTO codex_reserve_policy(thread_id,mode,state) VALUES(?1,'auto','ordinary') ON CONFLICT(thread_id) DO NOTHING",
        [thread_id],
    )?;
    get_connection(&connection, thread_id)?.ok_or_else(|| {
        crate::StoreError::Integrity("Reserve policy insert was not observable".into())
    })
}

pub fn set_mode(path: &Path, thread_id: &str, mode: &str) -> Result<Policy> {
    let mut connection = crate::schema::open_initialized(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT INTO codex_reserve_policy(thread_id,mode,state,revision) VALUES(?1,?2,'ordinary',1)
         ON CONFLICT(thread_id) DO UPDATE SET mode=excluded.mode,
         state=CASE WHEN codex_reserve_policy.mode IN ('auto','on','off')
             AND excluded.mode IN ('auto','on','off')
             AND codex_reserve_policy.state <> 'ordinary'
             THEN codex_reserve_policy.state ELSE 'ordinary' END,
         account_id=CASE WHEN codex_reserve_policy.mode IN ('auto','on','off')
             AND excluded.mode IN ('auto','on','off')
             AND codex_reserve_policy.state <> 'ordinary'
             THEN codex_reserve_policy.account_id ELSE NULL END,
         process_id=CASE WHEN codex_reserve_policy.mode IN ('auto','on','off')
             AND excluded.mode IN ('auto','on','off')
             AND codex_reserve_policy.state <> 'ordinary'
             THEN codex_reserve_policy.process_id ELSE NULL END,
         generation=CASE WHEN codex_reserve_policy.mode IN ('auto','on','off')
             AND excluded.mode IN ('auto','on','off')
             AND codex_reserve_policy.state <> 'ordinary'
             THEN codex_reserve_policy.generation ELSE NULL END,
         previous_model=CASE WHEN codex_reserve_policy.mode IN ('auto','on','off')
             AND excluded.mode IN ('auto','on','off')
             AND codex_reserve_policy.state <> 'ordinary'
             THEN codex_reserve_policy.previous_model ELSE NULL END,
         previous_effort=CASE WHEN codex_reserve_policy.mode IN ('auto','on','off')
             AND excluded.mode IN ('auto','on','off')
             AND codex_reserve_policy.state <> 'ordinary'
             THEN codex_reserve_policy.previous_effort ELSE NULL END,
         previous_tier=CASE WHEN codex_reserve_policy.mode IN ('auto','on','off')
             AND excluded.mode IN ('auto','on','off')
             AND codex_reserve_policy.state <> 'ordinary'
             THEN codex_reserve_policy.previous_tier ELSE NULL END,
         previous_effort_present=CASE WHEN codex_reserve_policy.mode IN ('auto','on','off')
             AND excluded.mode IN ('auto','on','off')
             AND codex_reserve_policy.state <> 'ordinary'
             THEN codex_reserve_policy.previous_effort_present ELSE 0 END,
         applied_model=CASE WHEN codex_reserve_policy.mode IN ('auto','on','off')
             AND excluded.mode IN ('auto','on','off')
             AND codex_reserve_policy.state <> 'ordinary'
             THEN codex_reserve_policy.applied_model ELSE NULL END,
         applied_effort=CASE WHEN codex_reserve_policy.mode IN ('auto','on','off')
             AND excluded.mode IN ('auto','on','off')
             AND codex_reserve_policy.state <> 'ordinary'
             THEN codex_reserve_policy.applied_effort ELSE NULL END,
         applied_tier=CASE WHEN codex_reserve_policy.mode IN ('auto','on','off')
             AND excluded.mode IN ('auto','on','off')
             AND codex_reserve_policy.state <> 'ordinary'
             THEN codex_reserve_policy.applied_tier ELSE NULL END,
         revision=codex_reserve_policy.revision+1,
         updated_at=unixepoch()",
        params![thread_id, mode],
    )?;
    if mode == "manual" {
        usage_fence::resolve_manual_in(&tx, thread_id)?;
    }
    tx.commit()?;
    get_connection(&connection, thread_id)?.ok_or_else(|| {
        crate::StoreError::Integrity("Reserve policy mode update was not observable".into())
    })
}

pub fn begin_episode(
    path: &Path,
    thread_id: &str,
    account_id: &str,
    process_id: Option<i64>,
    generation: i64,
    previous: (&str, Option<&str>, Option<&str>),
) -> Result<bool> {
    let Some(policy) = get(path, thread_id)? else {
        return Ok(false);
    };
    Ok(begin_episode_claim(
        path,
        thread_id,
        policy.revision,
        EpisodeIdentity {
            account_id,
            process_id,
            generation,
        },
        previous,
        previous.1.is_some(),
        (None, None, None),
    )?
    .is_some())
}

#[derive(Clone, Copy, Debug)]
pub struct EpisodeIdentity<'a> {
    pub account_id: &'a str,
    pub process_id: Option<i64>,
    pub generation: i64,
}

pub fn begin_episode_claim(
    path: &Path,
    thread_id: &str,
    expected_revision: i64,
    identity: EpisodeIdentity<'_>,
    previous: (&str, Option<&str>, Option<&str>),
    previous_effort_present: bool,
    applied: (Option<&str>, Option<&str>, Option<&str>),
) -> Result<Option<EpisodeClaim>> {
    let EpisodeIdentity {
        account_id,
        process_id,
        generation,
    } = identity;
    let mut connection = crate::schema::open_initialized(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let usage_failure = usage_fence::claim_in(&tx, thread_id)?;
    let changed = tx.execute(
        "UPDATE codex_reserve_policy SET state='entering',account_id=?2,process_id=?3,
         generation=?4,previous_model=?5,previous_effort=?6,previous_effort_present=?7,
         previous_tier=?8,applied_model=?9,applied_effort=?10,applied_tier=?11,
         revision=revision+1,updated_at=unixepoch()
         WHERE thread_id=?1 AND revision=?12 AND mode IN ('auto','on') AND state='ordinary'
         AND NOT EXISTS (SELECT 1 FROM codex_archive_fences WHERE target_thread_id=?1)",
        params![
            thread_id,
            account_id,
            process_id,
            generation,
            previous.0,
            previous.1,
            previous_effort_present,
            previous.2,
            applied.0,
            applied.1,
            applied.2,
            expected_revision
        ],
    )?;
    tx.commit()?;
    Ok((changed == 1).then_some(EpisodeClaim {
        revision: expected_revision + 1,
        usage_failure,
    }))
}

pub fn finish_episode(path: &Path, thread_id: &str, state: &str) -> Result<bool> {
    let Some(policy) = get(path, thread_id)? else {
        return Ok(false);
    };
    finish_episode_claim(
        path,
        thread_id,
        EpisodeClaim {
            revision: policy.revision,
            usage_failure: None,
        },
        if state == "reserve" {
            "entering"
        } else {
            "restoring"
        },
        state,
    )
}

pub fn finish_episode_claim(
    path: &Path,
    thread_id: &str,
    claim: EpisodeClaim,
    expected_state: &str,
    state: &str,
) -> Result<bool> {
    let mut connection = crate::schema::open_initialized(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = tx.execute(
        "UPDATE codex_reserve_policy SET state=?2,revision=revision+1,updated_at=unixepoch()
         WHERE thread_id=?1 AND revision=?3 AND state=?4 AND mode IN ('auto','on')
         AND ((?4='entering' AND ?2='reserve') OR (?4='restoring' AND ?2='ordinary'))",
        params![thread_id, state, claim.revision, expected_state],
    )? == 1;
    if !changed {
        tx.commit()?;
        return Ok(false);
    }
    let policy = get_connection(&tx, thread_id)?.ok_or_else(|| {
        crate::StoreError::Integrity("Reserve policy completion was not observable".into())
    })?;
    transition_notice::stage_in(&tx, &policy)?;
    usage_fence::resolve_episode_in(
        &tx,
        thread_id,
        claim.usage_failure,
        policy.revision,
        "automatic Reserve transition confirmed",
    )?;
    tx.commit()?;
    Ok(true)
}

pub fn stage_usage_failure(path: &Path, thread_id: &str, reason: &str) -> Result<()> {
    let mut connection = crate::schema::open_initialized(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    stage_usage_failure_in(&tx, thread_id, reason)?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn stage_usage_failure_in(
    connection: &Connection,
    thread_id: &str,
    reason: &str,
) -> Result<()> {
    usage_fence::stage_in(connection, thread_id, reason)
}

pub fn usage_failure_unresolved(path: &Path, thread_id: &str) -> Result<bool> {
    usage_fence::unresolved(path, thread_id)
}

pub fn usage_failure_claim(path: &Path, thread_id: &str) -> Result<Option<usage_fence::Claim>> {
    let connection = crate::schema::open_initialized(path)?;
    usage_fence::claim_in(&connection, thread_id)
}

pub fn resolve_usage_failure(
    path: &Path,
    thread_id: &str,
    expected_fence_id: i64,
    expected_policy_revision: i64,
    reason: &str,
) -> Result<bool> {
    let mut connection = crate::schema::open_initialized(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = usage_fence::resolve_in(
        &tx,
        thread_id,
        expected_fence_id,
        expected_policy_revision,
        expected_policy_revision,
        reason,
    )?;
    tx.commit()?;
    Ok(changed)
}

pub fn resolve_usage_failure_claim(
    path: &Path,
    thread_id: &str,
    claim: usage_fence::Claim,
    reason: &str,
) -> Result<bool> {
    let Some(failure_policy_revision) = claim.failure_policy_revision else {
        return Ok(false);
    };
    let mut connection = crate::schema::open_initialized(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = usage_fence::resolve_in(
        &tx,
        thread_id,
        claim.fence_id,
        failure_policy_revision,
        claim.policy_revision,
        reason,
    )?;
    tx.commit()?;
    Ok(changed)
}

pub fn begin_restore(
    path: &Path,
    thread_id: &str,
    account_id: &str,
    process_id: Option<i64>,
    generation: i64,
) -> Result<bool> {
    let Some(policy) = get(path, thread_id)? else {
        return Ok(false);
    };
    Ok(begin_restore_claim(
        path,
        thread_id,
        policy.revision,
        account_id,
        process_id,
        generation,
    )?
    .is_some())
}

/// Begin a NEW restore attempt after fresh account and exact idle-settings validation.
/// The confirmed Reserve episode may be inherited; its old process/generation are
/// evidence, not a ban on recovery. Revision/mode/state/account/archive CAS remains.
pub fn begin_restore_claim(
    path: &Path,
    thread_id: &str,
    expected_revision: i64,
    account_id: &str,
    process_id: Option<i64>,
    generation: i64,
) -> Result<Option<EpisodeClaim>> {
    let mut connection = crate::schema::open_initialized(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let usage_failure = usage_fence::claim_in(&tx, thread_id)?;
    let changed = tx.execute(
        "UPDATE codex_reserve_policy SET state='restoring',account_id=?2,process_id=?3,
         generation=?4,revision=revision+1,updated_at=unixepoch()
         WHERE thread_id=?1 AND revision=?5 AND mode IN ('auto','on') AND state='reserve'
         AND account_id=?2
         AND NOT EXISTS (SELECT 1 FROM codex_archive_fences WHERE target_thread_id=?1)",
        params![
            thread_id,
            account_id,
            process_id,
            generation,
            expected_revision
        ],
    )?;
    tx.commit()?;
    Ok((changed == 1).then_some(EpisodeClaim {
        revision: expected_revision + 1,
        usage_failure,
    }))
}

pub fn mark_unknown(path: &Path, thread_id: &str, reason: &str) -> Result<()> {
    let connection = crate::schema::open_initialized(path)?;
    connection.execute(
        "UPDATE codex_reserve_policy SET state='unknown',revision=revision+1,updated_at=unixepoch()
         WHERE thread_id=?1",
        [thread_id],
    )?;
    eprintln!("reserve_auto_policy_unknown thread_id={thread_id} reason={reason}");
    Ok(())
}

pub fn mark_unknown_claim(
    path: &Path,
    thread_id: &str,
    expected_revision: i64,
    reason: &str,
) -> Result<bool> {
    let connection = crate::schema::open_initialized(path)?;
    let changed = connection.execute(
        "UPDATE codex_reserve_policy SET state='unknown',revision=revision+1,updated_at=unixepoch()
         WHERE thread_id=?1 AND revision=?2 AND state IN ('ordinary','entering','restoring','reserve','held','unknown')",
        params![thread_id, expected_revision],
    )?;
    eprintln!("reserve_auto_policy_unknown thread_id={thread_id} reason={reason}");
    Ok(changed == 1)
}

pub fn recovery_candidates(path: &Path) -> Result<Vec<String>> {
    recovery_candidates_after(path, None)
}

/// Keyset paging: skipped/busy/slow targets cannot monopolize every poll.
pub fn recovery_candidates_after(path: &Path, after: Option<&str>) -> Result<Vec<String>> {
    let connection = crate::schema::open_initialized(path)?;
    let mut statement = connection.prepare(
        "SELECT thread_id FROM codex_reserve_policy WHERE mode IN ('auto','on')
         AND state='reserve' AND (?1 IS NULL OR thread_id > ?1)
         ORDER BY thread_id LIMIT 128",
    )?;
    Ok(statement
        .query_map([after], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?)
}

fn get_connection(connection: &Connection, thread_id: &str) -> Result<Option<Policy>> {
    Ok(connection
        .query_row(
            "SELECT thread_id,mode,state,account_id,process_id,generation,previous_model,
             previous_effort,previous_effort_present,previous_tier,applied_model,
             applied_effort,applied_tier,revision FROM codex_reserve_policy WHERE thread_id=?1",
            [thread_id],
            |row| {
                Ok(Policy {
                    thread_id: row.get(0)?,
                    mode: row.get(1)?,
                    state: row.get(2)?,
                    account_id: row.get(3)?,
                    process_id: row.get(4)?,
                    generation: row.get(5)?,
                    previous_model: row.get(6)?,
                    previous_effort: row.get(7)?,
                    previous_effort_present: row.get(8)?,
                    previous_tier: row.get(9)?,
                    applied_model: row.get(10)?,
                    applied_effort: row.get(11)?,
                    applied_tier: row.get(12)?,
                    revision: row.get(13)?,
                })
            },
        )
        .optional()?)
}
