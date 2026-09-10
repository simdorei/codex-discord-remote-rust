use super::ResidentAppServer;
use tokio::sync::watch;

impl ResidentAppServer {
    /// Subscribe before checking a lifecycle snapshot to avoid a lost wakeup.
    /// Any change revokes previously admitted optional work, including close,
    /// transport death, quarantine, restart request and generation replacement.
    /// The value is the pending restart generation, not a health snapshot.
    #[must_use]
    pub fn subscribe_lifecycle_changes(&self) -> watch::Receiver<Option<u64>> {
        self.state.subscribe_restart_pending()
    }
}
