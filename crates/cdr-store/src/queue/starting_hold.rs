use std::collections::BTreeSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Transaction, TransactionBehavior, params};

use super::read::select_job;
use super::{QueueJobState, StoredQueueJob};
use crate::Result;
use crate::schema::open_initialized;

pub const STARTING_CANDIDATE_HOLD_PREFIX: &str = "[cdr-rust:turn-start-candidates-ambiguous:v1] ";
const NOTICE_PREFIX: &str = "turn-start-candidates-ambiguous:";
const MAX_CANDIDATES_IN_MARKER: usize = 4;
const MAX_CANDIDATES_IN_NOTICE: usize = 8;
const MAX_MARKER_CANDIDATE_CHARS: usize = 80;
const MAX_NOTICE_CANDIDATE_CHARS: usize = 120;
const MAX_PRIOR_ERROR_CHARS: usize = 400;
const MAX_MARKER_CHARS: usize = 1_000;
const MAX_NOTICE_CHARS: usize = 1_900;

pub fn hold_starting_for_ambiguous_candidates_if_claimed(
    path: &Path,
    claimed: &StoredQueueJob,
    candidate_turn_ids: &[String],
) -> Result<Option<StoredQueueJob>> {
    let candidates = CandidateSummary::new(candidate_turn_ids);
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    super::fork_handoff::ensure_schema(&transaction)?;

    if claimed
        .last_error
        .starts_with(STARTING_CANDIDATE_HOLD_PREFIX)
    {
        if !exact_starting_snapshot_matches(&transaction, claimed)? {
            transaction.commit()?;
            return Ok(None);
        }
        refresh_notice_if_pending(&transaction, claimed, &candidates)?;
        let current = select_job(&transaction, &claimed.job_id)?;
        transaction.commit()?;
        return Ok(Some(current));
    }

    let marker = marker(claimed, &candidates);
    let Some(baseline) = claimed.matching_baseline_json(&transaction)? else {
        return Ok(None);
    };
    let updated = transaction.execute(
        "UPDATE codex_turn_queue SET last_error = ?, updated_at = ? \
         WHERE job_id = ? AND target_thread_id = ? AND channel_id = ? \
         AND owner_user_id IS ? AND discord_message_id IS ? \
         AND app_server_generation = ? AND prompt = ? AND queued = ? AND ack_sent = ? \
         AND state = 'starting' AND attempt_count = ? AND turn_id IS NULL \
         AND baseline_turn_ids = ? AND last_error = ? AND created_at = ? AND updated_at = ? \
         AND goal_waiting = ? AND NOT EXISTS (SELECT 1 FROM codex_thread_fork_handoffs handoff \
             WHERE handoff.source_thread_id = codex_turn_queue.target_thread_id \
             AND handoff.target_thread_id IS NULL)",
        params![
            marker,
            now()?,
            claimed.job_id,
            claimed.target_thread_id,
            claimed.channel_id,
            claimed.owner_user_id,
            claimed.discord_message_id,
            claimed.app_server_generation,
            claimed.prompt,
            i64::from(claimed.queued),
            i64::from(claimed.ack_sent),
            claimed.attempt_count,
            baseline,
            claimed.last_error,
            claimed.created_at,
            claimed.updated_at,
            i64::from(claimed.goal_waiting),
        ],
    )?;
    if updated != 1 {
        transaction.commit()?;
        return Ok(None);
    }
    let held = select_job(&transaction, &claimed.job_id)?;
    insert_notice(&transaction, &held, &candidates)?;
    transaction.commit()?;
    Ok(Some(held))
}

fn exact_starting_snapshot_matches(
    transaction: &Transaction<'_>,
    claimed: &StoredQueueJob,
) -> Result<bool> {
    if claimed.state != QueueJobState::Starting || claimed.turn_id.is_some() {
        return Ok(false);
    }
    let Some(baseline) = claimed.matching_baseline_json(transaction)? else {
        return Ok(false);
    };
    Ok(transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_turn_queue job WHERE job_id = ? \
         AND target_thread_id = ? AND channel_id = ? AND owner_user_id IS ? \
         AND discord_message_id IS ? AND app_server_generation = ? AND prompt = ? \
         AND queued = ? AND ack_sent = ? AND state = 'starting' AND attempt_count = ? \
         AND turn_id IS NULL AND baseline_turn_ids = ? AND last_error = ? \
         AND created_at = ? AND updated_at = ? AND goal_waiting = ? \
         AND NOT EXISTS (SELECT 1 FROM codex_thread_fork_handoffs handoff \
             WHERE handoff.source_thread_id = job.target_thread_id \
             AND handoff.target_thread_id IS NULL))",
        params![
            claimed.job_id,
            claimed.target_thread_id,
            claimed.channel_id,
            claimed.owner_user_id,
            claimed.discord_message_id,
            claimed.app_server_generation,
            claimed.prompt,
            i64::from(claimed.queued),
            i64::from(claimed.ack_sent),
            claimed.attempt_count,
            baseline,
            claimed.last_error,
            claimed.created_at,
            claimed.updated_at,
            i64::from(claimed.goal_waiting),
        ],
        |row| row.get(0),
    )?)
}

fn insert_notice(
    transaction: &Transaction<'_>,
    held: &StoredQueueJob,
    candidates: &CandidateSummary,
) -> Result<()> {
    let notice_id = format!("{NOTICE_PREFIX}{}", held.job_id);
    let observed_at = now()?;
    transaction.execute(
        "INSERT INTO codex_delivery_outbox (delivery_id, job_id, target_thread_id, \
         turn_id, channel_id, content, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            notice_id,
            notice_id,
            held.target_thread_id,
            notice_id,
            held.channel_id,
            notice(held, candidates),
            observed_at,
            observed_at,
        ],
    )?;
    Ok(())
}

fn refresh_notice_if_pending(
    transaction: &Transaction<'_>,
    held: &StoredQueueJob,
    candidates: &CandidateSummary,
) -> Result<()> {
    let notice_id = format!("{NOTICE_PREFIX}{}", held.job_id);
    let content = notice(held, candidates);
    transaction.execute(
        "UPDATE codex_delivery_outbox SET content = ?, updated_at = ? \
         WHERE delivery_id = ? AND job_id = ? AND target_thread_id = ? AND content != ?",
        params![
            content,
            now()?,
            notice_id,
            notice_id,
            held.target_thread_id,
            content,
        ],
    )?;
    Ok(())
}

struct CandidateSummary {
    count: usize,
    marker_ids: Vec<String>,
    notice_ids: Vec<String>,
}

impl CandidateSummary {
    fn new(ids: &[String]) -> Self {
        let unique = ids.iter().map(String::as_str).collect::<BTreeSet<_>>();
        Self {
            count: unique.len(),
            marker_ids: bounded_ids(
                &unique,
                MAX_CANDIDATES_IN_MARKER,
                MAX_MARKER_CANDIDATE_CHARS,
            ),
            notice_ids: bounded_ids(
                &unique,
                MAX_CANDIDATES_IN_NOTICE,
                MAX_NOTICE_CANDIDATE_CHARS,
            ),
        }
    }
}

fn bounded_ids(ids: &BTreeSet<&str>, count: usize, chars: usize) -> Vec<String> {
    ids.iter()
        .take(count)
        .map(|id| id.chars().take(chars).collect())
        .collect()
}

fn marker(claimed: &StoredQueueJob, candidates: &CandidateSummary) -> String {
    let ids = serde_json::to_string(&candidates.marker_ids).unwrap_or_else(|_| "[]".into());
    let previous = previous_error(&claimed.last_error);
    let suffix = if previous.is_empty() {
        String::new()
    } else {
        format!("; previous_error={previous}")
    };
    format!(
        "{STARTING_CANDIDATE_HOLD_PREFIX}candidate_count={}; candidate_turn_ids={ids}; candidate_ids_listed={}{}",
        candidates.count,
        candidates.marker_ids.len(),
        suffix,
    )
    .chars()
    .take(MAX_MARKER_CHARS)
    .collect()
}

fn notice(held: &StoredQueueJob, candidates: &CandidateSummary) -> String {
    let ids = serde_json::to_string(&candidates.notice_ids).unwrap_or_else(|_| "[]".into());
    format!(
        "Codex could not safely determine which turn belongs to queued job {} on thread {}. The target queue is held, so later queued requests will not start until this is resolved. Candidate turns: count={}, listed={} {ids}",
        held.job_id,
        held.target_thread_id,
        candidates.count,
        candidates.notice_ids.len(),
    )
    .chars()
    .take(MAX_NOTICE_CHARS)
    .collect()
}

fn previous_error(error: &str) -> String {
    let error = error.trim();
    if error.starts_with("[cdr-rust:") {
        String::new()
    } else {
        error.chars().take(MAX_PRIOR_ERROR_CHARS).collect()
    }
}

fn now() -> Result<f64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64())
}
