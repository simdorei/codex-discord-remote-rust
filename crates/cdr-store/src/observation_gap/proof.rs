use super::{
    COLUMNS, Connection, Gap, OptionalExtension, Path, Result, Scope, TransactionBehavior, active,
    invalid, open_initialized, params, read_gap, save,
};

#[derive(Clone, Debug)]
pub enum Effect {
    NoRequiredStore,
    Unconfirmed,
    Started {
        thread: String,
        turn: String,
    },
    Terminal {
        thread: String,
        turn: String,
        payload: String,
    },
    Final {
        thread: String,
        turn: String,
        content: String,
    },
    Question {
        id: String,
        thread: String,
        turn: String,
        item: String,
        body: String,
    },
}
fn completed(db: &Connection, scope: &Scope, thread: &str, turn: &str) -> Result<bool> {
    let marker = crate::mirror::turn_origin_marker(thread, turn);
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_idle_release i
        JOIN codex_session_mirror_events e ON e.event_digest=?5 AND e.codex_thread_id=i.thread_id
        WHERE i.owner_id=?1 AND i.generation=?2 AND i.thread_id=?3 AND i.turn_id=?4
        AND i.job_id!='')",
        params![scope.owner_id, scope.generation, thread, turn, marker],
        |r| r.get(0),
    )?)
}
fn matches_effect(db: &Connection, scope: &Scope, effect: &Effect) -> Result<bool> {
    match effect {
        Effect::NoRequiredStore=>Ok(true),
        Effect::Unconfirmed=>Ok(false),
        Effect::Started{thread,turn}=>Ok(completed(db,scope,thread,turn)? || db.query_row(
            "SELECT (SELECT COUNT(*) FROM codex_turn_queue WHERE target_thread_id=?1 AND state='running')=1
            AND EXISTS(SELECT 1 FROM codex_turn_queue WHERE target_thread_id=?1 AND turn_id=?2
                AND state='running' AND COALESCE(turn_observation_generation,app_server_generation)=?3)
            AND NOT EXISTS(SELECT 1 FROM cdr_async_questions WHERE runtime_id=?4 AND generation=?3
                AND thread_id=?1 AND turn_id!=?2 AND state NOT IN ('expired','submitted','rejected','closed_unknown'))",
            params![thread,turn,scope.generation,scope.owner_id],|r|r.get(0))?),
        Effect::Terminal{thread,turn,payload}=>Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM codex_observed_completions WHERE thread_id=?1 AND turn_id=?2
                AND generation=?3 AND resident_owner=?4 AND payload=?5)",
            params![thread,turn,scope.generation,scope.owner_id,payload],|r|r.get(0))?),
        Effect::Final{thread,turn,content}=>Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM codex_observed_final_answers WHERE thread_id=?1 AND turn_id=?2
                AND generation=?3 AND content=?4)",params![thread,turn,scope.generation,content],|r|r.get(0))?),
        Effect::Question{id,thread,turn,item,body}=>Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM cdr_async_questions WHERE id=?1 AND runtime_id=?2 AND generation=?3
                AND thread_id=?4 AND turn_id=?5 AND item_id=?6 AND body=?7 AND owner_confirmed=1)
            OR EXISTS(SELECT 1 FROM cdr_async_question_inbox i JOIN codex_turn_queue q ON q.job_id=i.candidate_job_id
                WHERE i.id=?1 AND i.runtime_id=?2 AND i.generation=?3 AND i.thread_id=?4 AND i.turn_id=?5
                AND i.item_id=?6 AND i.body=?7 AND i.state='waiting' AND q.target_thread_id=i.thread_id
                AND q.channel_id=i.candidate_channel_id AND q.owner_user_id=i.candidate_owner_id
                AND q.app_server_generation=i.candidate_generation
                AND q.execution_generation IS i.candidate_execution_generation
                AND q.attempt_count=i.candidate_attempt_count AND q.state='running')",
            params![id,scope.owner_id,scope.generation,thread,turn,item,body],|r|r.get(0))?),
    }
}

/// Actual required-effect checks and proof storage share one DB transaction.
pub fn certify(path: &Path, scope: &Scope, sequence: i64, effects: &[Effect]) -> Result<bool> {
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if !active(&tx, scope)? || effects.is_empty() {
        return Ok(false);
    }
    let current = tx
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM cdr_observation_gaps
        WHERE owner_id=?1 AND generation=?2 AND first_seq>0 AND first_seq<=?3 AND last_seq>=?3"
            ),
            params![scope.owner_id, scope.generation, sequence],
            Gap::read,
        )
        .optional()?;
    let Some(current) = current else {
        return Ok(false);
    };
    for effect in effects {
        if !matches_effect(&tx, scope, effect)? {
            return Ok(false);
        }
    }
    if !current.contains_verified(sequence) {
        let mut updated = current.clone();
        updated.add(sequence)?;
        if !save(&tx, &current, &updated)? {
            return Err(invalid("observation proof CAS lost"));
        }
    }
    tx.commit()?;
    Ok(true)
}

/// Cursor advancement is not certification. Existing positive spans are never erased.
pub fn finish_page(path: &Path, expected: &Gap, through: i64) -> Result<bool> {
    if through <= expected.cursor || through > expected.last {
        return Err(invalid("invalid observation scan progress"));
    }
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if !active(&tx, &expected.scope)? {
        return Ok(false);
    }
    let Some(current) = read_gap(&tx, expected.id)? else {
        return Ok(false);
    };
    if current != *expected {
        return Ok(false);
    }
    let mut updated = current.clone();
    updated.cursor = through;
    if !save(&tx, &current, &updated)? {
        return Ok(false);
    }
    if through == current.last || updated.complete() {
        tx.execute(
            "UPDATE cdr_observation_streams SET scan_after=?3 WHERE owner_id=?1 AND generation=?2",
            params![current.scope.owner_id, current.scope.generation, current.id],
        )?;
    }
    tx.commit()?;
    Ok(true)
}
