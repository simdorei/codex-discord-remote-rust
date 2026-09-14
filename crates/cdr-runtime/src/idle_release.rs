//! Store adapter installed before Discord intake, recovery, or question answers.
use cdr_app_server::{
    AppServerError, ResidentAppServer,
    idle_release::{IdleReleaseJournal, IdleReleaseToken},
};
use cdr_store::idle_release::{self as store, Intent};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) fn install(server: &ResidentAppServer, path: &Path) -> Result<(), AppServerError> {
    // Validate schema/holds at cold start. Never clear an old owner's rows here.
    store::pending(path).map_err(error)?;
    server.install_idle_release_journal(Arc::new(Journal(path.to_owned())))
}

struct Journal(PathBuf);

fn error(e: impl std::fmt::Display) -> AppServerError {
    AppServerError::IdleRelease {
        message: e.to_string(),
    }
}

pub(crate) fn token(i: Intent) -> Result<IdleReleaseToken, AppServerError> {
    Ok(IdleReleaseToken {
        intent_id: i.intent_id,
        owner_id: i.owner_id,
        generation: u64::try_from(i.generation).map_err(error)?,
        thread_id: i.thread_id,
        turn_id: i.turn_id,
        job_id: i.job_id,
        revision: i.revision,
        state: i.state,
        detail: i.detail,
    })
}

fn intent(t: &IdleReleaseToken) -> Result<Intent, AppServerError> {
    Ok(Intent {
        intent_id: t.intent_id.clone(),
        owner_id: t.owner_id.clone(),
        generation: i64::try_from(t.generation).map_err(error)?,
        thread_id: t.thread_id.clone(),
        turn_id: t.turn_id.clone(),
        job_id: t.job_id.clone(),
        revision: t.revision,
        state: t.state.clone(),
        detail: t.detail.clone(),
    })
}

impl IdleReleaseJournal for Journal {
    fn before_mutation(
        &self,
        owner: &str,
        generation: u64,
        thread: &str,
    ) -> Result<Option<IdleReleaseToken>, AppServerError> {
        store::before_mutation(
            &self.0,
            owner,
            i64::try_from(generation).map_err(error)?,
            thread,
        )
        .map_err(error)?
        .map(token)
        .transpose()
    }
    fn check_mutation(&self, thread: &str) -> Result<(), AppServerError> {
        if let Some(i) = store::get(&self.0, thread).map_err(error)?
            && !matches!(i.state.as_str(), "Candidate" | "Settled")
        {
            return Err(error(format!(
                "thread {thread} subscription {} requires review; automatic mutation held: {}",
                i.state, i.detail
            )));
        }
        Ok(())
    }
    fn verify(&self, t: &IdleReleaseToken, require_idle: bool) -> Result<(), AppServerError> {
        store::verify(&self.0, &intent(t)?, require_idle).map_err(error)
    }
    fn resume_required(&self, thread: &str) -> Result<bool, AppServerError> {
        if store::get(&self.0, thread)
            .map_err(error)?
            .is_some_and(|i| i.state == "AwaitUnload")
        {
            return Ok(true);
        }
        self.check_mutation(thread)?;
        Ok(false)
    }
    fn transition(
        &self,
        t: &IdleReleaseToken,
        state: &str,
        detail: &str,
    ) -> Result<IdleReleaseToken, AppServerError> {
        token(store::transition(&self.0, &intent(t)?, state, detail).map_err(error)?)
    }
    fn old_child_exited(&self, owner: &str, generation: u64) -> Result<(), AppServerError> {
        store::settle_exited_owner(&self.0, owner, i64::try_from(generation).map_err(error)?)
            .map_err(error)
    }
}
