//! Compare the actual persisted ingress, never caller-supplied actor headers.
use super::{
    api::{Decision, StoredProposal},
    invalid,
    snapshot::Target,
};
use crate::Result;
use rusqlite::{Connection, params};
use serde_json::{Value, json};

struct Ingress {
    version: i64,
    kind: String,
    event: Option<i64>,
    application: Option<i64>,
    channel: i64,
    owner: i64,
    message: Option<i64>,
    payload: Value,
    runtime: Option<String>,
    state: String,
    phase: String,
    target: Option<String>,
    owner_kind: Option<String>,
    owner_id: Option<String>,
    created_at: f64,
}

fn read_in(db: &Connection, id: &str) -> Result<Ingress> {
    let (mut value, encoded) = db.query_row(
        "SELECT version,kind,event_id,application_id,channel_id,owner_user_id,source_message_id,
         runtime_id,state,phase,target_thread_id,owner_kind,owner_id,created_at,payload_json
         FROM discord_ingress_journal WHERE ingress_id=? AND length(CAST(payload_json AS BLOB))<=131072",
        [id], |r| Ok((Ingress {
            version:r.get(0)?, kind:r.get(1)?, event:r.get(2)?, application:r.get(3)?,
            channel:r.get(4)?, owner:r.get(5)?, message:r.get(6)?, payload:Value::Null,
            runtime:r.get(7)?, state:r.get(8)?, phase:r.get(9)?, target:r.get(10)?,
            owner_kind:r.get(11)?, owner_id:r.get(12)?, created_at:r.get(13)?,
        }, r.get::<_,String>(14)?)),
    )?;
    value.payload = serde_json::from_str(&encoded)?;
    if value.version != 1 || !value.created_at.is_finite() || value.created_at < 0.0 {
        return Err(invalid("unsupported saved ingress"));
    }
    Ok(value)
}

fn executing(value: &Ingress) -> Result<()> {
    if value.state != "executing"
        || value.phase != "processing"
        || value.owner_kind.is_some()
        || value.owner_id.is_some()
    {
        return Err(invalid("ingress has no current unowned processing custody"));
    }
    Ok(())
}

pub(super) fn runtime_in(db: &Connection) -> Result<Value> {
    let app: String = db.query_row(
        "SELECT runtime_id FROM codex_app_server_runtime WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let wire: String = db.query_row(
        "SELECT runtime_id FROM codex_mutation_runtime WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    if app.trim().is_empty() || wire.trim().is_empty() {
        return Err(invalid("runtime identity is unavailable"));
    }
    Ok(json!({"app":app,"wire":wire}))
}

pub(super) fn message_in(
    db: &Connection,
    target: &Target,
    id: &str,
    require_executing: bool,
) -> Result<Value> {
    let saved = read_in(db, id)?;
    let content = saved.payload["content"]
        .as_str()
        .ok_or_else(|| invalid("proposal command is missing"))?;
    let words: Vec<_> = content.split_whitespace().collect();
    let runtime = runtime_in(db)?;
    if saved.kind != "message"
        || saved.application.is_some()
        || saved.event.is_none_or(|v| v <= 0)
        || saved.message != saved.event
        || saved.channel != target.channel
        || saved.owner != target.owner
        || saved.target.as_deref() != Some(&target.thread)
        || saved.runtime.as_deref() != runtime["app"].as_str()
        || saved.payload["version"] != 1
        || saved.payload["author_is_bot"] != false
        || words != ["!discard-request", target.job.as_str()]
        || saved.payload["plan"] != json!({"Execute":{"DiscardRequest":{"job_id":target.job}}})
        || saved.owner_kind.is_some()
        || saved.owner_id.is_some()
    {
        return Err(invalid(
            "proposal source is not the exact authenticated owner request",
        ));
    }
    if require_executing {
        executing(&saved)?;
    }
    Ok(
        json!({"id":id,"event":saved.event,"application":saved.application,
        "channel":saved.channel,"owner":saved.owner,"message":saved.message,
        "payload":saved.payload,"runtime":saved.runtime,"target":saved.target,
        "created_at_bits":saved.created_at.to_bits()}),
    )
}

pub(super) fn click_in(
    db: &Connection,
    stored: &StoredProposal,
    id: &str,
    decision: Decision,
    fresh: bool,
) -> Result<i64> {
    let p = &stored.proposal;
    let saved = read_in(db, id)?;
    let event = saved
        .event
        .filter(|v| *v > 0)
        .ok_or_else(|| invalid("interaction event is missing"))?;
    let delivery: (i64,i64,String) = db.query_row(
        "SELECT revision,message_id,body_sha256 FROM cdr_recovery_abandonment_deliveries WHERE proposal_id=?",
        [&p.id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    )?;
    if saved.kind != "interaction"
        || delivery.0 != p.revision
        || delivery.1 <= 0
        || delivery.2 != p.review_sha256
        || saved.application != Some(p.application_id)
        || saved.channel != p.channel_id
        || saved.owner != p.owner_user_id
        || saved.message != Some(delivery.1)
        || saved.target.as_deref() != Some(&p.thread_id)
        || saved.runtime.as_deref() != stored.snapshot["context"]["runtime"]["app"].as_str()
        || !click_payload_matches(&saved.payload, &p.id, p.revision, decision)
    {
        return Err(invalid(
            "saved interaction does not match the displayed abandonment decision",
        ));
    }
    if fresh {
        executing(&saved)?;
        if runtime_in(db)? != stored.snapshot["context"]["runtime"] {
            return Err(invalid("runtime changed before decision"));
        }
        let duplicate: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM cdr_recovery_abandonment_decisions
             WHERE (ingress_id=? OR interaction_id=?) AND proposal_id!=?)",
            params![id, event, p.id],
            |r| r.get(0),
        )?;
        if duplicate {
            return Err(invalid("interaction was already consumed elsewhere"));
        }
    }
    Ok(event)
}

fn click_payload_matches(payload: &Value, id: &str, revision: i64, decision: Decision) -> bool {
    let work = json!({"Component":{"RecoveryAbandonDecision":{
        "proposal_id":id,"revision":revision,"decision":decision
    }}});
    // Preserve exact historical receipts; fresh dispatcher envelopes must carry
    // all three normal-admission fields, with no rejection or settings binding.
    payload == &json!({"version":1,"work":work})
        || payload
            == &json!({"version":1,"processing_mode":"normal","work":work,
            "settings_binding":null,"request_rejection":null})
}
