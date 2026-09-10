//! Coordinate the local deletion guard and both independent recovery stores.
use super::{ActionError, ActionExecutor, ActionResult, immediate};
use crate::{archive_delete::delete_archived_thread, queue_runner::TurnBackend};
use cdr_store::{mapping::delete_archived_state, room_cleanup::archive};

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) fn delete_archive_preview(&self, reference: &str) -> Result<String, ActionError> {
        let thread = self.resolve_reference(reference, true)?;
        Ok(format!(
            "Archived thread deletion preview\nthread_id: {}\ntitle: {}\ncwd: {}\nrollout_path: {}\nTo delete, run !confirm_delete_archive {}",
            thread.id,
            thread.title,
            thread.cwd,
            thread.rollout_path.display(),
            thread.id
        ))
    }

    pub(super) fn delete_archive_confirm(
        &self,
        reference: &str,
        context: super::ActionContext,
    ) -> Result<ActionResult, ActionError> {
        let thread = self.resolve_reference(reference, true)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs_f64();
        let confirmation = context
            .discord_message_id
            .map(|id| -> Result<_, ActionError> {
                Ok(archive::Confirmation {
                    message_id: i64::try_from(id).map_err(|_| ActionError::IntegerRange)?,
                    channel_id: i64::try_from(context.channel_id)
                        .map_err(|_| ActionError::IntegerRange)?,
                    user_id: i64::try_from(context.user_id)
                        .map_err(|_| ActionError::IntegerRange)?,
                    reference,
                })
            })
            .transpose()?;
        let token =
            archive::begin_confirmed(&self.mirror_db, &thread.id, now, confirmation.as_ref())?;
        let recovery = self
            .archive_delete_paths
            .backup_root
            .join(format!("archive-guard-{token}"));
        let result = (|| -> Result<ActionResult, ActionError> {
            std::fs::create_dir_all(&recovery)?;
            let connection = rusqlite::Connection::open_with_flags(
                &self.mirror_db,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .map_err(cdr_store::StoreError::from)?;
            connection
                .backup(rusqlite::MAIN_DB, recovery.join("mirror.sqlite"), None)
                .map_err(cdr_store::StoreError::from)?;
            let mut paths = self.archive_delete_paths.clone();
            paths.backup_root = recovery.join("source");
            let deleted = delete_archived_thread(&paths, &thread.id)?;
            let mirror = delete_archived_state(&self.mirror_db, &thread.id)?;
            archive::complete(&self.mirror_db, &thread.id, &token)?;
            Ok(immediate(format!(
                "Archived thread deleted after backup\nthread_id: {}\nbackup_dir: {}\nbackup_files: {}\ndeleted_log_rows: {}\ndeleted_rollout_path: {}\nmirror_rows: {}\nmirror_offsets: {}",
                thread.id,
                recovery.display(),
                deleted.backup_paths.len() + 1,
                deleted.deleted_log_rows,
                deleted.deleted_rollout_path.display(),
                mirror.mirror_threads,
                mirror.session_mirror_offsets
            )))
        })();
        result.map_err(|error|ActionError::Invalid(format!("archive deletion stopped and remains fenced; recovery directory: {}; inspect existing backups and the original failure before manual reconciliation; do not retry automatically: {error}",recovery.display())))
    }
}
