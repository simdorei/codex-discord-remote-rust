//! Durable notices for successful automatic Reserve transitions.
use super::Policy;
use crate::{Result, StoreError, mapping::mirrored_thread_id_in, schema::open_initialized};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::path::Path;

pub const DOMAIN: &str = "reserve/transition/v1";

#[derive(Clone, Debug)]
pub struct TransitionNotice {
    pub notice_id: String,
    pub thread_id: String,
    pub channel_id: Option<i64>,
    pub content: String,
}

pub(super) fn migrate(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS codex_reserve_transition_notices (
            notice_id TEXT PRIMARY KEY,
            target_thread_id TEXT NOT NULL,
            channel_id INTEGER,
            policy_revision INTEGER NOT NULL,
            transition_state TEXT NOT NULL CHECK(transition_state IN ('reserve','ordinary')),
            content TEXT NOT NULL,
            created_at REAL NOT NULL DEFAULT(unixepoch())
        );
        CREATE INDEX IF NOT EXISTS codex_reserve_transition_notices_pending
            ON codex_reserve_transition_notices(created_at, notice_id);",
    )?;
    Ok(())
}

pub(super) fn schema_current(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT COUNT(*)=7 FROM pragma_table_info('codex_reserve_transition_notices')
         WHERE name IN ('notice_id','target_thread_id','channel_id','policy_revision',
                        'transition_state','content','created_at')
         AND EXISTS(SELECT 1 FROM sqlite_schema
                    WHERE type='index' AND name='codex_reserve_transition_notices_pending')",
        [],
        |row| row.get(0),
    )?)
}

pub(crate) fn stage_in(connection: &Connection, policy: &Policy) -> Result<()> {
    let channel_id: Option<i64> = connection
        .query_row(
            "SELECT discord_thread_id FROM mirror_threads WHERE codex_thread_id=?1",
            [&policy.thread_id],
            |row| row.get(0),
        )
        .optional()?;
    let notice_id = format!("{}:{}:{}", policy.thread_id, policy.revision, policy.state);
    let content = content_for(policy)?;
    connection.execute(
        "INSERT OR IGNORE INTO codex_reserve_transition_notices
         (notice_id,target_thread_id,channel_id,policy_revision,transition_state,content)
         VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            notice_id,
            policy.thread_id,
            channel_id,
            policy.revision,
            policy.state,
            content
        ],
    )?;
    Ok(())
}

pub fn pending(path: &Path) -> Result<Vec<TransitionNotice>> {
    let connection = open_initialized(path)?;
    let mut statement = connection.prepare(
        "SELECT notice_id,target_thread_id,channel_id,content
         FROM codex_reserve_transition_notices ORDER BY created_at,notice_id",
    )?;
    Ok(statement
        .query_map([], |row| {
            Ok(TransitionNotice {
                notice_id: row.get(0)?,
                thread_id: row.get(1)?,
                channel_id: row.get(2)?,
                content: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn complete(path: &Path, notice_id: &str) -> Result<()> {
    open_initialized(path)?.execute(
        "DELETE FROM codex_reserve_transition_notices WHERE notice_id=?1",
        [notice_id],
    )?;
    Ok(())
}

pub(crate) fn validate_claim_in(connection: &Connection, key: &str, hash: &str) -> Result<()> {
    let Ok((channel, domain, notice_id, chunk)) =
        serde_json::from_str::<(i64, String, String, usize)>(key)
    else {
        return Ok(());
    };
    if domain != DOMAIN {
        return Ok(());
    }
    if chunk != 0 {
        return Err(StoreError::Integrity(
            "Reserve transition notice has an invalid chunk".into(),
        ));
    }
    let Some((thread, stored_channel, content)) = connection
        .query_row(
            "SELECT target_thread_id,channel_id,content
             FROM codex_reserve_transition_notices WHERE notice_id=?1",
            [notice_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?
    else {
        return Err(StoreError::Integrity(
            "Reserve transition notice is not pending".into(),
        ));
    };
    if stored_channel != Some(channel) || hex::encode(Sha256::digest(content.as_bytes())) != hash {
        return Err(StoreError::Integrity(
            "Reserve transition notice destination or content changed".into(),
        ));
    }
    if mirrored_thread_id_in(connection, Some(channel))?.as_deref() != Some(thread.as_str()) {
        return Err(StoreError::Integrity(
            "Reserve transition notice room was remapped".into(),
        ));
    }
    crate::dead_generation::ensure_target_available(connection, &thread)?;
    Ok(())
}

fn content_for(policy: &Policy) -> Result<String> {
    let legacy_unrecorded = "not recorded (legacy episode API)";
    let model = match policy.state.as_str() {
        "reserve" => policy.applied_model.as_deref().unwrap_or(legacy_unrecorded),
        "ordinary" => policy.previous_model.as_deref().ok_or_else(|| {
            StoreError::Integrity("Reserve transition notice model is missing".into())
        })?,
        _ => legacy_unrecorded,
    };
    let effort = match policy.state.as_str() {
        "reserve" if policy.applied_model.is_none() => legacy_unrecorded,
        "reserve" => policy.applied_effort.as_deref().unwrap_or("null (기본값)"),
        "ordinary" => {
            if !policy.previous_effort_present {
                return Err(StoreError::Integrity(
                    "Reserve transition notice effort presence is missing".into(),
                ));
            }
            policy.previous_effort.as_deref().unwrap_or("null (기본값)")
        }
        _ => legacy_unrecorded,
    };
    let tier = match policy.state.as_str() {
        "reserve" if policy.applied_model.is_none() => legacy_unrecorded,
        "reserve" => policy.applied_tier.as_deref().unwrap_or("null (기본값)"),
        "ordinary" => policy.previous_tier.as_deref().unwrap_or("null (기본값)"),
        _ => legacy_unrecorded,
    };
    let label = if policy.state == "reserve" {
        "자동 Reserve 전환 확인"
    } else {
        "일반 설정 자동 복귀 확인"
    };
    Ok(format!(
        "In progress\n{label}\nmodel: {model}\neffort: {effort}\nservice tier: {tier}\npolicy revision: {}\n이전 실패 요청은 자동 재실행하지 않습니다.",
        policy.revision
    ))
}
