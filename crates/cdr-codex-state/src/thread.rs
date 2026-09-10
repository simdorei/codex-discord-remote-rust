use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadInfo {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub updated_at: i64,
    pub rollout_path: PathBuf,
    pub model: String,
    pub reasoning_effort: String,
    /// None means the state source did not record cumulative usage.
    pub tokens_used: Option<i64>,
    pub archived_at: i64,
}
