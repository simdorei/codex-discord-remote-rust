use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};

use cdr_codex_state::{CodexThreadStore, ThreadInfo};
use cdr_store::{
    mapping::mirror_targets,
    queue::{StoredQueueJob, list},
};
use futures_util::{FutureExt, StreamExt, future::BoxFuture, stream::FuturesUnordered};
use tokio::time::{Instant, MissedTickBehavior, interval, timeout};

use super::{SessionMirrorError, SessionMirrorPoll, SessionMirrorSender, SessionMirrorWorker};
use crate::session_mirror::SessionMirrorRetryState;

const DISCOVERY_INTERVAL: Duration = Duration::from_secs(1);
const TARGET_DEADLINE: Duration = Duration::from_secs(10);
const MAX_IN_FLIGHT: usize = 8;

#[derive(Default)]
struct Retry {
    state: SessionMirrorRetryState,
    ready_at: Option<Instant>,
}

#[derive(Default)]
struct Snapshot {
    threads: HashMap<String, Arc<ThreadInfo>>,
    ready: VecDeque<(String, i64)>,
    queue_jobs: Arc<Vec<StoredQueueJob>>,
}

struct Finished {
    thread: String,
    channel: i64,
    outcome: Result<SessionMirrorPoll, SessionMirrorError>,
}

struct Background<'a, S: SessionMirrorSender> {
    worker: &'a SessionMirrorWorker<S>,
    pending: FuturesUnordered<BoxFuture<'a, Finished>>,
    active_threads: HashSet<String>,
    active_channels: HashSet<i64>,
    retry: HashMap<String, Retry>,
    snapshot: Snapshot,
    started: Instant,
}

impl<S: SessionMirrorSender> SessionMirrorWorker<S> {
    pub(crate) async fn poll_background(
        &self,
        global_retry: &mut SessionMirrorRetryState,
    ) -> Result<SessionMirrorPoll, SessionMirrorError> {
        Background::new(self).run(global_retry).await
    }
}

impl<'a, S: SessionMirrorSender> Background<'a, S> {
    fn new(worker: &'a SessionMirrorWorker<S>) -> Self {
        Self {
            worker,
            pending: FuturesUnordered::new(),
            active_threads: HashSet::new(),
            active_channels: HashSet::new(),
            retry: HashMap::new(),
            snapshot: Snapshot::default(),
            started: Instant::now(),
        }
    }

    async fn run(
        mut self,
        global_retry: &mut SessionMirrorRetryState,
    ) -> Result<SessionMirrorPoll, SessionMirrorError> {
        let mut discovery = interval(DISCOVERY_INTERVAL);
        discovery.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = discovery.tick() => {
                    self.refresh()?;
                    // Only a shared discovery failure uses the runner-wide backoff.
                    let _ = global_retry.on_success();
                }
                Some(finished) = self.pending.next(), if !self.pending.is_empty() => {
                    self.completed(finished);
                }
            }
            self.dispatch();
        }
    }

    fn refresh(&mut self) -> Result<(), SessionMirrorError> {
        self.worker.recent_assistant_text.prune_expired();
        let threads = CodexThreadStore::open(&self.worker.state_db)?
            .load_recent_threads(0)?
            .into_iter()
            .map(|thread| (thread.id.clone(), Arc::new(thread)))
            .collect();
        let targets = mirror_targets(&self.worker.mirror_db, i64::MAX)?;
        let present = targets
            .iter()
            .map(|row| row.codex_thread_id.as_str())
            .collect::<HashSet<_>>();
        self.retry.retain(|thread, _| {
            present.contains(thread.as_str()) || self.active_threads.contains(thread)
        });
        let queue_jobs = Arc::new(list(&self.worker.mirror_db)?);
        let mut destinations = targets
            .iter()
            .map(|row| (row.codex_thread_id.clone(), row.discord_thread_id))
            .collect::<HashMap<_, _>>();
        let mut ready = std::mem::take(&mut self.snapshot.ready);
        // Discovery must not let already serviced targets overtake existing waiters.
        ready.retain_mut(|(id, channel)| {
            let Some(current) = destinations.remove(id) else {
                return false;
            };
            *channel = current;
            !self.active_threads.contains(id)
        });
        for row in targets {
            if destinations.remove(&row.codex_thread_id).is_some()
                && !self.active_threads.contains(&row.codex_thread_id)
            {
                ready.push_back((row.codex_thread_id, row.discord_thread_id));
            }
        }
        self.snapshot = Snapshot {
            threads,
            ready,
            queue_jobs,
        };
        Ok(())
    }

    fn dispatch(&mut self) {
        let candidates = self.snapshot.ready.len();
        for _ in 0..candidates {
            if self.pending.len() == MAX_IN_FLIGHT {
                break;
            }
            let Some((id, channel)) = self.snapshot.ready.pop_front() else {
                break;
            };
            if self.active_threads.contains(&id) {
                continue;
            }
            if self.active_channels.contains(&channel) {
                self.snapshot.ready.push_back((id, channel));
                continue;
            }
            if self
                .retry
                .get(&id)
                .and_then(|state| state.ready_at)
                .is_some_and(|ready| Instant::now() < ready)
            {
                continue;
            }
            self.active_threads.insert(id.clone());
            self.active_channels.insert(channel);
            let thread = self.snapshot.threads.get(&id).cloned();
            let jobs = Arc::clone(&self.snapshot.queue_jobs);
            let worker = self.worker;
            self.pending.push(async move {
                let outcome = match thread {
                    Some(thread) => {
                        match timeout(TARGET_DEADLINE, worker.poll_target(&thread, channel, &jobs)).await {
                            Ok(result) => result,
                            Err(_) => Err(SessionMirrorError::Delivery(
                                "phase=target_poll; deadline=10s; cursor retained; in-flight delivery outcome may be unknown; no success inferred".into(),
                            )),
                        }
                    }
                    None => Err(SessionMirrorError::TargetUnavailable),
                };
                Finished { thread: id, channel, outcome }
            }.boxed());
        }
    }

    fn completed(&mut self, finished: Finished) {
        let Finished {
            thread,
            channel,
            outcome,
        } = finished;
        self.active_threads.remove(&thread);
        self.active_channels.remove(&channel);
        match outcome {
            Ok(result) => {
                self.retry.remove(&thread);
                if result.sent > 0 {
                    eprintln!("session_mirror_poll: {result:?} target={thread} channel={channel}");
                }
            }
            Err(error) => {
                let retry = self.retry.entry(thread.clone()).or_default();
                let decision = retry
                    .state
                    .on_failure(self.started.elapsed(), &error.to_string());
                retry.ready_at = Some(Instant::now() + decision.retry_after);
                if let Some(report) = decision.report {
                    eprintln!(
                        "session_mirror_error count={} error={} target={thread} channel={channel}",
                        report.count, report.error,
                    );
                }
            }
        }
    }
}
