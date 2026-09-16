use super::{MirrorSyncError, MirrorSynchronizer, db_id, now};

impl MirrorSynchronizer {
    pub(super) fn ensure_cleanup_reconciled(&self) -> Result<(), MirrorSyncError> {
        let rooms = cdr_store::room_cleanup::unconfirmed_channels(&self.mirror_db)?;
        if !rooms.is_empty() {
            return Err(MirrorSyncError::Invalid(format!(
                "cleanup outcome is unconfirmed and fenced for rooms {rooms:?}; manual reconciliation required before sync"
            )));
        }
        Ok(())
    }

    pub(super) fn fence_missing_room(
        &self,
        channel: u64,
        target: Option<&str>,
    ) -> Result<(), MirrorSyncError> {
        let id = db_id(channel)?;
        match cdr_store::room_cleanup::phase(&self.mirror_db, id)?.as_deref() {
            Some("deleted") => Ok(()),
            Some(_) => Err(MirrorSyncError::Invalid(
                "previous cleanup outcome is unconfirmed; mapping retained".into(),
            )),
            None => {
                let token = cdr_store::room_cleanup::begin(&self.mirror_db, id, target, now()?)
                    .map_err(cleanup_begin_error)?;
                cdr_store::room_cleanup::complete(&self.mirror_db, id, &token)?;
                Ok(())
            }
        }
    }
    pub(super) async fn delete_guarded(
        &self,
        channel: u64,
        target: Option<&str>,
    ) -> Result<(), MirrorSyncError> {
        let id = db_id(channel)?;
        let token = cdr_store::room_cleanup::begin(&self.mirror_db, id, target, now()?)
            .map_err(cleanup_begin_error)?;
        self.dispatch_guarded_delete(channel, &token).await
    }

    pub(super) async fn dispatch_guarded_delete(
        &self,
        channel: u64,
        token: &str,
    ) -> Result<(), MirrorSyncError> {
        let id = db_id(channel)?;
        match self.remote.delete(channel).await {
            Ok(()) => cdr_store::room_cleanup::complete(&self.mirror_db, id, token)?,
            Err(error @ MirrorSyncError::DeleteRejected(_)) => {
                cdr_store::room_cleanup::release_rejected(&self.mirror_db, id, token)?;
                return Err(error);
            }
            Err(error) => {
                return Err(MirrorSyncError::Invalid(format!(
                    "room {channel} cleanup outcome is unconfirmed; durable fence retained, no automatic retry: {error}"
                )));
            }
        }
        Ok(())
    }
}

// Only the transaction's pre-delete pending check proves this attempt did not
// dispatch deletion. Mapping, database, and post-delete failures stay uncertain.
pub(super) fn cleanup_begin_error(error: cdr_store::StoreError) -> MirrorSyncError {
    match error {
        cdr_store::StoreError::CleanupProtected { channel, reason } if channel > 0 => {
            MirrorSyncError::CleanupProtected {
                channel: channel.unsigned_abs(),
                reason,
            }
        }
        error => MirrorSyncError::Store(error),
    }
}
