use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use crate::session_mirror::{
    DiscordSessionMirrorSender, SESSION_MIRROR_ASSISTANT_TEXT_NONCE_DOMAIN,
    SESSION_MIRROR_EVENT_NONCE_DOMAIN, SessionMirrorDeliveryIdentity, SessionMirrorError,
    SessionMirrorPoll, SessionMirrorSender, run_session_mirror_worker, session_delivery_identity,
};
use crate::session_mirror::{
    MirrorDetail, MirrorItem, MirrorKind, RecentTextCache, TargetFailures, assistant_scope,
    collect_items_with_context, discord_active_turn, discord_origin_user, format_item,
    normalized_text_digest, read_context_before,
};
use cdr_codex_state::{CodexThreadStore, read_new_session_events};
use cdr_store::mapping::{MirrorDetailMode, mirror_targets};
use cdr_store::mirror::{
    claim_event, get_cursor_turn, get_or_init_cursor, has_event, turn_origin_marker, update_cursor,
    update_cursor_with_turn,
};
use cdr_store::queue::{StoredQueueJob, list};

const RECENT_ASSISTANT_TEXT_TTL: Duration = Duration::from_mins(10);

pub struct SessionMirrorWorker<S: SessionMirrorSender> {
    state_db: PathBuf,
    mirror_db: PathBuf,
    sender: Arc<S>,
    recent_assistant_text: RecentTextCache,
}

impl<S: SessionMirrorSender> SessionMirrorWorker<S> {
    #[must_use]
    pub fn new(state_db: PathBuf, mirror_db: PathBuf, sender: Arc<S>) -> Self {
        Self {
            state_db,
            mirror_db,
            sender,
            recent_assistant_text: RecentTextCache::new(RECENT_ASSISTANT_TEXT_TTL),
        }
    }

    pub async fn poll_once(&self) -> Result<SessionMirrorPoll, SessionMirrorError> {
        self.recent_assistant_text.prune_expired();
        let threads = CodexThreadStore::open(&self.state_db)?
            .load_recent_threads(0)?
            .into_iter()
            .map(|thread| (thread.id.clone(), thread))
            .collect::<HashMap<_, _>>();
        let targets = mirror_targets(&self.mirror_db, i64::MAX)?;
        let mut result = SessionMirrorPoll {
            targets: targets.len(),
            ..SessionMirrorPoll::default()
        };
        let queue_jobs = list(&self.mirror_db)?;
        let mut failures = TargetFailures::default();
        for target in targets {
            let Some(thread) = threads.get(&target.codex_thread_id) else {
                failures.record(
                    target.codex_thread_id,
                    SessionMirrorError::TargetUnavailable,
                );
                continue;
            };
            let outcome = self
                .poll_target(thread, target.discord_thread_id, &queue_jobs)
                .await;
            match outcome {
                Ok(one) => {
                    result.events += one.events;
                    result.sent += one.sent;
                }
                Err(error) => {
                    failures.record(target.codex_thread_id, error);
                }
            }
        }
        failures.finish(result)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Keep ordered delivery and cursor commit together for one polling target"
    )]
    async fn poll_target(
        &self,
        thread: &cdr_codex_state::ThreadInfo,
        channel_id: i64,
        queue_jobs: &[StoredQueueJob],
    ) -> Result<SessionMirrorPoll, SessionMirrorError> {
        if !thread.rollout_path.is_file() {
            return Err(SessionMirrorError::TargetRolloutUnavailable);
        }
        let rollout = thread.rollout_path.to_string_lossy();
        let size = thread.rollout_path.metadata()?.len();
        let initial = i64::try_from(size).map_err(|_| SessionMirrorError::CursorRange)?;
        let timestamp = now()?;
        let mut cursor =
            get_or_init_cursor(&self.mirror_db, &thread.id, &rollout, initial, timestamp)?;
        if cursor < 0 || u64::try_from(cursor).map_err(|_| SessionMirrorError::CursorRange)? > size
        {
            cursor = 0;
            update_cursor(&self.mirror_db, &thread.id, &rollout, cursor, timestamp)?;
        }
        let tail = read_new_session_events(
            &thread.rollout_path,
            u64::try_from(cursor).map_err(|_| SessionMirrorError::CursorRange)?,
            None,
        )?;
        if tail.events.is_empty() {
            return Ok(SessionMirrorPoll::default());
        }
        let detail = match cdr_store::mapping::get_detail_mode(&self.mirror_db, &thread.id)? {
            MirrorDetailMode::Send => MirrorDetail::Send,
            MirrorDetailMode::All => MirrorDetail::All,
        };
        let current_turn = match get_cursor_turn(&self.mirror_db, &thread.id)? {
            Some(turn) => (!turn.is_empty()).then_some(turn),
            None => read_context_before(
                &thread.rollout_path,
                u64::try_from(cursor).map_err(|_| SessionMirrorError::CursorRange)?,
            )?,
        };
        let (items, current_turn) =
            collect_items_with_context(&thread.id, &tail.events, detail, current_turn);
        let mut sent = 0;
        let mut assistant_texts = HashSet::new();
        let channel = u64::try_from(channel_id).map_err(|_| SessionMirrorError::CursorRange)?;
        for item in items {
            if self.current_discord_owner(&thread.id, &item)? {
                continue;
            }
            if discord_origin_user(&self.mirror_db, &thread.id, &item, queue_jobs)? {
                continue;
            }
            if item.kind != MirrorKind::User && discord_active_turn(&thread.id, &item, queue_jobs) {
                continue;
            }
            let assistant_text = item.dedupe_recent_text;
            let text_scope = assistant_scope(&thread.id, &item);
            if assistant_text {
                let digest = normalized_text_digest(&item.text);
                if self
                    .recent_assistant_text
                    .is_recent(&text_scope, &item.text)
                    || !assistant_texts.insert((text_scope.clone(), digest))
                {
                    continue;
                }
            }
            if item.kind != MirrorKind::User
                && let Some(turn_id) = item.turn_id.as_deref()
                && has_event(
                    &self.mirror_db,
                    &turn_origin_marker(&thread.id, turn_id),
                    &thread.id,
                )?
            {
                continue;
            }
            if has_event(&self.mirror_db, &item.digest, &thread.id)? {
                continue;
            }
            let identity = session_delivery_identity(&thread.id, &item);
            let message = format_item(&item);
            self.sender
                .send(channel, &identity, &message)
                .await
                .map_err(SessionMirrorError::Delivery)?;
            let _ = claim_event(&self.mirror_db, &item.digest, &thread.id, now()?)?;
            if assistant_text {
                self.recent_assistant_text.remember(&text_scope, &item.text);
            }
            sent += 1;
        }
        let next = i64::try_from(tail.next_offset).map_err(|_| SessionMirrorError::CursorRange)?;
        update_cursor_with_turn(
            &self.mirror_db,
            &thread.id,
            &rollout,
            next,
            now()?,
            current_turn.as_deref(),
        )?;
        Ok(SessionMirrorPoll {
            targets: 1,
            events: tail.events.len(),
            sent,
        })
    }

    fn current_discord_owner(
        &self,
        thread: &str,
        item: &MirrorItem,
    ) -> Result<bool, SessionMirrorError> {
        // Refresh the poll-wide snapshot; unresolved starts must retain the cursor.
        let jobs = cdr_store::queue::list_filtered(&self.mirror_db, Some(thread), None)?;
        // Completion processing may still be awaiting history/Discord before
        // setting goal_waiting. Do not let the next goal turn escape that gap.
        for job in &jobs {
            if job.state == cdr_store::queue::QueueJobState::Running
                && let Some(turn) = job.turn_id.as_deref()
                && cdr_store::observed_completion::contains(&self.mirror_db, thread, turn)?
            {
                return Err(SessionMirrorError::OwnershipPending);
            }
        }
        if jobs.iter().any(|job| {
            job.state == cdr_store::queue::QueueJobState::Starting
                || (job.state == cdr_store::queue::QueueJobState::Running && job.goal_waiting)
        }) {
            return Err(SessionMirrorError::OwnershipPending);
        }
        Ok(discord_origin_user(&self.mirror_db, thread, item, &jobs)?
            || (item.kind != MirrorKind::User && discord_active_turn(thread, item, &jobs)))
    }
}

fn now() -> Result<f64, SessionMirrorError> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64())
}
