//! Stop originals before a durable prompt job exists; never invent a job ID.
use rusqlite::{Connection, params};
use serde_json::json;

use super::{REASON, StopScope, StoredIngress, get_in, refused};
use crate::Result;

pub(super) fn snapshot(
    db: &Connection,
    scope: StopScope<'_>,
    already: usize,
) -> Result<Vec<StoredIngress>> {
    let keys = ids(db, scope.target)?;
    if already.saturating_add(keys.len()) > 128 {
        return Err(refused());
    }
    keys.iter()
        .map(|key| {
            let original = get_in(db, key)?.ok_or_else(refused)?;
            if original.channel_id != scope.channel
                || original.owner_user_id != scope.owner
                || original.event_id.is_none_or(|event| event <= 0)
                || original.owner_kind.is_some()
                || original.owner_id.is_some()
            {
                return Err(refused());
            }
            Ok(original)
        })
        .collect()
}

pub(super) fn hold(
    db: &Connection,
    originals: &[StoredIngress],
    operation: &str,
) -> Result<Vec<StoredIngress>> {
    originals
        .iter()
        .map(|original| {
            let mut wanted = original.clone();
            let mut outcome = original.outcome.clone().unwrap_or_else(|| json!({}));
            if !outcome.is_object() {
                return Err(refused());
            }
            if let Some(previous) = outcome.get("stop_hold") {
                if original.state != "held"
                    || previous["kind"] != "stop_original_ingress"
                    || previous["ingress_id"] != original.ingress_id
                    || previous["target"].as_str() != original.target_thread_id.as_deref()
                    || previous["channel"] != original.channel_id
                    || previous["owner"] != original.owner_user_id
                    || previous["event_id"].as_i64() != original.event_id
                {
                    return Err(refused());
                }
                return Ok(wanted);
            }
            let evidence = json!({
                "kind":"stop_original_ingress","ingress_id":original.ingress_id,
                "target":original.target_thread_id,"channel":original.channel_id,
                "owner":original.owner_user_id,"event_id":original.event_id,
                "operation_id":operation
            });
            outcome["stop_hold"] = evidence.clone();
            wanted.outcome = Some(outcome);
            wanted.state = "held".into();
            if wanted.hold_reason.is_empty() {
                wanted.hold_reason = REASON.into();
            }
            // Keep the original envelope, phase, timestamps and prior JSON evidence.
            // json_set adds only the stop key rather than round-tripping old numbers.
            if db.execute(
                "UPDATE discord_ingress_journal SET state='held',hold_reason=?,
             outcome_json=json_set(COALESCE(outcome_json,'{}'),'$.stop_hold',json(?))
             WHERE ingress_id=? AND owner_id IS NULL",
                params![
                    wanted.hold_reason,
                    evidence.to_string(),
                    original.ingress_id
                ],
            )? != 1
                || get_in(db, &original.ingress_id)?.as_ref() != Some(&wanted)
            {
                return Err(refused());
            }
            Ok(wanted)
        })
        .collect()
}

pub(super) fn verify(db: &Connection, target: &str, held: &[StoredIngress]) -> Result<()> {
    let keys: Vec<_> = held
        .iter()
        .map(|record| record.ingress_id.as_str())
        .collect();
    if ids(db, target)? != keys {
        return Err(refused());
    }
    for original in held {
        if get_in(db, &original.ingress_id)?.as_ref() != Some(original) {
            return Err(refused());
        }
    }
    Ok(())
}

pub(crate) fn require_unheld_key_in(db: &Connection, key: &str) -> Result<()> {
    let held: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM discord_ingress_journal WHERE ingress_id=?
         AND json_type(outcome_json,'$.stop_hold') IS NOT NULL)",
        [key],
        |row| row.get(0),
    )?;
    if held {
        return Err(refused());
    }
    Ok(())
}

pub(crate) fn require_unheld_origin_in(db: &Connection, event: Option<i64>) -> Result<()> {
    let Some(event) = event else {
        return Ok(());
    };
    let held: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM discord_ingress_journal WHERE event_id=?
         AND json_type(outcome_json,'$.stop_hold') IS NOT NULL)",
        [event],
        |row| row.get(0),
    )?;
    if held {
        return Err(refused());
    }
    Ok(())
}

fn ids(db: &Connection, target: &str) -> Result<Vec<String>> {
    Ok(db.prepare(
        "SELECT ingress_id FROM discord_ingress_journal
         WHERE target_thread_id=? AND owner_id IS NULL
         AND state IN ('staged','acknowledged','executing','held')
         AND json_type(payload_json,'$.version')='integer'
         AND json_extract(payload_json,'$.version')=1 AND (
             (kind='message' AND (json_type(payload_json,'$.plan.Execute.Ask.prompt')='text'
                 OR json_type(payload_json,'$.plan.Execute.Interview.prompt')='text'))
             OR (kind='interaction' AND json_extract(payload_json,'$.work.Slash.name') IN ('ask','interview')
                 AND json_type(payload_json,'$.work.Slash.values.prompt.String')='text'))
         ORDER BY ingress_id LIMIT 129",
    )?.query_map([target], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?)
}
