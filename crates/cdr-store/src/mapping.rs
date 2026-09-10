mod detail;
mod maintenance;
mod new_origin;
pub(crate) use new_origin::new_thread_origin_in;
pub use new_origin::{NewThreadOrigin, new_thread_origin};
mod project;
mod sync;
mod thread;
pub use sync::{
    MirrorThreadUpdate, commit_new_thread_sync, commit_thread_sync, retire_project_sync,
    retire_thread_sync,
};
pub(crate) use thread::mirrored_thread_id_in;

pub use detail::{MirrorDetailMode, get_detail_mode, set_detail_mode};
pub use maintenance::{
    ArchivedDeleteCounts, RemainingDiscordIds, StaleProject, StaleThread, delete_archived_state,
    delete_stale, is_mirrored_channel, remaining_discord_ids, stale_projects, stale_threads,
};
pub use project::{describe_project_channel, find_project, project_for_channel, upsert_project};
pub use thread::{
    MirrorTarget, mirror_targets, mirrored_thread_id, thread_channels, update_discord_thread_id,
    upsert_thread,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectMatch {
    pub channel_id: i64,
    pub stored_key: String,
}
