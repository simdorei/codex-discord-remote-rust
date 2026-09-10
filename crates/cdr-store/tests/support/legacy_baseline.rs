use std::path::{Path, PathBuf};

use cdr_store::Result;
use cdr_store::queue::{
    NewQueueJob, StoredQueueJob, enqueue, hold_starting_for_ambiguous_candidates_if_claimed, list,
    mark_running_if_claimed, record_start_failure_if_claimed, try_begin_attempt,
};
use cdr_store::schema::open_initialized;

pub const PYTHON_JSON: &str =
    r#"["baseline-a", "path/one", "\uae30\uc900", "\ud83d\ude80", "baseline-a"]"#;
pub const SPACED_JSON: &str = r#"["baseline-a", "path/one", "기준", "🚀", "baseline-a"]"#;
pub const ESCAPED_JSON: &str =
    r#"["\u0062aseline-a","path\/one","\uae30\uc900","\uD83D\uDE80","baseline-a"]"#;
pub const COMPACT_JSON: &str = r#"["baseline-a","path/one","기준","🚀","baseline-a"]"#;

#[derive(Clone, Copy, Debug)]
pub enum Operation {
    Running,
    DefiniteFailure,
    AmbiguousFailure,
    Hold,
    Refresh,
}

pub const OPERATIONS: [Operation; 5] = [
    Operation::Running,
    Operation::DefiniteFailure,
    Operation::AmbiguousFailure,
    Operation::Hold,
    Operation::Refresh,
];

pub struct Fixture {
    _temp: tempfile::TempDir,
    pub path: PathBuf,
    pub claimed: StoredQueueJob,
}

impl Fixture {
    pub fn new(operation: Operation, encoding: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("legacy-baseline.sqlite");
        enqueue(
            &path,
            NewQueueJob {
                job_id: "legacy",
                target_thread_id: "source",
                channel_id: 70,
                owner_user_id: Some(10),
                discord_message_id: Some(11),
                app_server_generation: 9,
                prompt: "retain original prompt",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        let baseline = baseline_values();
        try_begin_attempt(&path, "legacy", &baseline, 9)
            .unwrap()
            .unwrap();
        // Only the temporary fixture emulates the old writer's representation.
        open_initialized(&path)
            .unwrap()
            .execute(
                "UPDATE codex_turn_queue SET updated_at = 1.0 WHERE job_id = 'legacy'",
                [],
            )
            .unwrap();
        let claimed = list(&path).unwrap().remove(0);
        if matches!(operation, Operation::Refresh) {
            hold_starting_for_ambiguous_candidates_if_claimed(
                &path,
                &claimed,
                &["candidate-before".into()],
            )
            .unwrap()
            .unwrap();
        }
        replace_baseline(&path, encoding);
        let claimed = list(&path).unwrap().remove(0);
        assert_eq!(claimed.baseline_turn_ids, baseline);
        Self {
            _temp: temp,
            path,
            claimed,
        }
    }

    pub fn apply(&self, operation: Operation) -> Result<Option<StoredQueueJob>> {
        apply(&self.path, &self.claimed, operation)
    }

    pub fn raw_baseline(&self) -> String {
        open_initialized(&self.path)
            .unwrap()
            .query_row(
                "SELECT baseline_turn_ids FROM codex_turn_queue WHERE job_id = 'legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }
}

pub fn apply(
    path: &Path,
    claimed: &StoredQueueJob,
    operation: Operation,
) -> Result<Option<StoredQueueJob>> {
    match operation {
        Operation::Running => mark_running_if_claimed(path, claimed, "observed-turn"),
        Operation::DefiniteFailure | Operation::AmbiguousFailure => {
            record_start_failure_if_claimed(
                path,
                claimed,
                "observed start failure",
                matches!(operation, Operation::AmbiguousFailure),
            )
        }
        Operation::Hold | Operation::Refresh => hold_starting_for_ambiguous_candidates_if_claimed(
            path,
            claimed,
            &["candidate-after-a".into(), "candidate-after-b".into()],
        ),
    }
}

pub fn replace_baseline(path: &Path, encoding: &str) {
    open_initialized(path)
        .unwrap()
        .execute(
            "UPDATE codex_turn_queue SET baseline_turn_ids = ? WHERE job_id = 'legacy'",
            [encoding],
        )
        .unwrap();
}

pub fn baseline_values() -> Vec<String> {
    ["baseline-a", "path/one", "기준", "🚀", "baseline-a"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}
