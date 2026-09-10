use std::collections::BTreeSet;

use cdr_app_server::outcomes::TurnStatus;
use cdr_store::queue::{
    QueueJobState, STARTING_ATTEMPT_LEASE_SECONDS, STARTING_CANDIDATE_HOLD_PREFIX, StoredQueueJob,
    hold_starting_for_ambiguous_candidates_if_claimed, list, mark_running_if_claimed,
    record_start_failure_if_claimed, repair_legacy_definite_app_server_fork_failures,
    unresolved_app_server_fork_handoff_for_source,
};

use super::retry::unix_now;
use super::{QueueCoordinator, QueueRunnerError, TurnBackend, TurnRecord, generation_i64};

mod state;
mod target;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecoveryReport {
    pub adopted: usize,
    pub recovered_running: usize,
    pub requeued: usize,
    pub completed: usize,
    pub unresolved: usize,
    pub started: usize,
    /// Targets for which authoritative `thread/read` failed.
    pub read_unavailable_targets: BTreeSet<String>,
    /// Targets for which a writer-only resume/start operation failed.
    pub mutation_unavailable_targets: BTreeSet<String>,
    /// Compatibility union of read and mutation failures.
    pub unavailable_targets: BTreeSet<String>,
    pub active_writer_targets: BTreeSet<String>,
}

impl<B: TurnBackend> QueueCoordinator<B> {
    pub async fn recover(&self) -> Result<RecoveryReport, QueueRunnerError> {
        self.repair_legacy_definite_fork_failures()?;
        let initial_targets = self.recovery_targets()?;
        self.initialize_recovery(&initial_targets)?;
        let generation = generation_i64(self.backend.generation())?;
        let mut first = RecoveryReport::default();
        self.observe_targets(&initial_targets, generation, &mut first)
            .await?;
        self.prepare_unmanaged_targets().await?;
        let mutation_targets = self.recovery_targets()?;
        self.initialize_recovery(&mutation_targets)?;
        self.mutate_targets(&mutation_targets, generation, &mut first)
            .await?;
        if !self.backend.requires_app_server_fork() {
            return Ok(first);
        }
        let observed_conflicts = first.active_writer_targets.clone();
        let mut moved = false;
        for target in &observed_conflicts {
            match self.fork_writer_conflict_if_safe(target).await {
                Ok(changed) => moved |= changed,
                Err(error) => {
                    let durably_fenced =
                        unresolved_app_server_fork_handoff_for_source(&self.db_path, target)?
                            .is_some();
                    if super::fork_handoff::is_nonfatal_recovery_blocker(&error) && durably_fenced {
                        eprintln!(
                            "rust_queue_app_server_writer_handoff_blocked target={target} error={error}"
                        );
                    } else {
                        return Err(error);
                    }
                }
            }
        }
        if !moved {
            return Ok(first);
        }
        let moved_targets = self.recovery_targets()?;
        self.initialize_recovery(&moved_targets)?;
        let mut recovered = RecoveryReport::default();
        self.observe_targets(&moved_targets, generation, &mut recovered)
            .await?;
        self.mutate_targets(&moved_targets, generation, &mut recovered)
            .await?;
        recovered.active_writer_targets.extend(observed_conflicts);
        Ok(recovered)
    }

    /// Reconcile and, when safe, start work for exactly one already-selected target.
    ///
    /// This deliberately does not scan or fork any other target. It is used after
    /// an app-server handoff has already selected the destination thread.
    pub async fn recover_target(&self, target: &str) -> Result<RecoveryReport, QueueRunnerError> {
        self.repair_legacy_definite_fork_failures()?;
        let targets = BTreeSet::from([target.to_owned()]);
        self.initialize_recovery(&targets)?;
        let generation = generation_i64(self.backend.generation())?;
        let mut report = RecoveryReport::default();
        self.observe_targets(&targets, generation, &mut report)
            .await?;
        self.mutate_targets(&targets, generation, &mut report)
            .await?;
        Ok(report)
    }

    fn repair_legacy_definite_fork_failures(&self) -> Result<(), QueueRunnerError> {
        if !self.backend.requires_app_server_fork() {
            let retired = cdr_store::queue::retire_copy_only_handoffs(&self.db_path)?;
            if retired != 0 {
                eprintln!(
                    "rust_exact_routing retired_copy_only_handoffs={retired}; mappings and requests unchanged"
                );
            }
            return Ok(());
        }
        for repaired in repair_legacy_definite_app_server_fork_failures(&self.db_path)? {
            eprintln!(
                "rust_queue_legacy_definite_fork_repaired source={} handoff={} jobs={}",
                repaired.source_thread_id, repaired.handoff_id, repaired.affected_jobs
            );
        }
        Ok(())
    }

    fn recovery_targets(&self) -> Result<BTreeSet<String>, QueueRunnerError> {
        Ok(list(&self.db_path)?
            .iter()
            .filter(|job| job.state != QueueJobState::Quarantined)
            .map(|job| job.target_thread_id.clone())
            .collect())
    }

    async fn observe_targets(
        &self,
        targets: &BTreeSet<String>,
        generation: i64,
        report: &mut RecoveryReport,
    ) -> Result<(), QueueRunnerError> {
        for target in targets {
            let lock = self.target_lock(target)?;
            let _guard = lock.lock().await;
            self.observe_one_locked(target, generation, report).await?;
        }
        Ok(())
    }

    async fn mutate_targets(
        &self,
        targets: &BTreeSet<String>,
        generation: i64,
        report: &mut RecoveryReport,
    ) -> Result<(), QueueRunnerError> {
        for target in targets {
            let lock = self.target_lock(target)?;
            let _guard = lock.lock().await;
            self.mutate_one_locked(target, generation, report).await?;
        }
        Ok(())
    }

    fn recover_starting(
        &self,
        job: &StoredQueueJob,
        generation: i64,
        cold: bool,
        turns: &[TurnRecord],
        report: &mut RecoveryReport,
    ) -> Result<bool, QueueRunnerError> {
        let baseline = job
            .baseline_turn_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let candidate_turn_ids = turns
            .iter()
            .filter(|turn| !baseline.contains(turn.turn_id.as_str()))
            .map(|turn| turn.turn_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if job.last_error.starts_with(STARTING_CANDIDATE_HOLD_PREFIX) {
            let _ = hold_starting_for_ambiguous_candidates_if_claimed(
                &self.db_path,
                job,
                &candidate_turn_ids,
            )?;
            report.unresolved += 1;
            return Ok(false);
        }
        let lease_active = job.last_error.is_empty()
            && job.updated_at.is_finite()
            && unix_now()? < job.updated_at + STARTING_ATTEMPT_LEASE_SECONDS;
        if lease_active {
            report.unresolved += 1;
            return Ok(false);
        }
        if candidate_turn_ids.len() == 1 {
            let recovered =
                mark_running_if_claimed(&self.db_path, job, &candidate_turn_ids[0])?.is_some();
            if recovered {
                report.recovered_running += 1;
            }
            return Ok(recovered);
        }
        if candidate_turn_ids.is_empty() && (cold || job.app_server_generation != generation) {
            if cdr_store::new_reply::get(&self.db_path, &job.job_id)?
                .is_some_and(|reply| reply.turn_id.is_none())
            {
                // A newly accepted turn may not be present in thread/read yet.
                // An empty snapshot, even after restart, cannot prove non-execution.
                if job.last_error.is_empty() {
                    record_start_failure_if_claimed(
                        &self.db_path,
                        job,
                        "new first-turn acceptance remains unknown; empty history does not authorize retry",
                        true,
                    )?;
                }
                report.unresolved += 1;
                return Ok(false);
            }
            let requeued = record_start_failure_if_claimed(
                &self.db_path,
                job,
                "previous app-server generation ended before a turn appeared",
                false,
            )?
            .is_some();
            if requeued {
                report.requeued += 1;
            }
            return Ok(requeued);
        }
        if candidate_turn_ids.len() > 1 {
            let _ = hold_starting_for_ambiguous_candidates_if_claimed(
                &self.db_path,
                job,
                &candidate_turn_ids,
            )?;
        }
        report.unresolved += 1;
        Ok(false)
    }

    fn recover_running(job: &StoredQueueJob, turns: &[TurnRecord], report: &mut RecoveryReport) {
        let Some(turn_id) = job.turn_id.as_deref() else {
            report.unresolved += 1;
            return;
        };
        let status = turns
            .iter()
            .find(|turn| turn.turn_id == turn_id)
            .map(|turn| turn.status);
        match status {
            Some(TurnStatus::Completed | TurnStatus::Interrupted | TurnStatus::Failed) => {
                // The completion worker must first durably stage the Discord reply.
                report.unresolved += 1;
            }
            Some(TurnStatus::InProgress) => {}
            None => report.unresolved += 1,
        }
    }
}
