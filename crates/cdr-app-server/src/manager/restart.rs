use std::future::Future;

use crate::{AppServerClient, AppServerError};

use super::ResidentAppServer;
use super::admission::{ReplacementCleanup, RestartCandidate};
use super::events::{ResidentForwarders, prepare_forwarders};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestartOutcome {
    Restarted,
    Deferred,
    Settled,
}

impl ResidentAppServer {
    pub async fn restart_if_quiescent(&self) -> Result<bool, AppServerError> {
        self.restart_if_quiescent_observed(|_| ()).await
    }

    pub(in crate::manager) async fn restart_if_quiescent_observed(
        &self,
        observe: impl FnOnce(&AppServerClient),
    ) -> Result<bool, AppServerError> {
        let outcome = self
            .restart_if_quiescent_using(
                None,
                None,
                observe,
                |client| async move { client.close().await },
                |_, _| {},
            )
            .await?;
        Ok(outcome == RestartOutcome::Restarted)
    }

    async fn restart_if_quiescent_using<F, Fut, BeforeInstall>(
        &self,
        expected_generation: Option<u64>,
        forced_generation: Option<u64>,
        observe: impl FnOnce(&AppServerClient),
        mut cleanup: F,
        before_install: BeforeInstall,
    ) -> Result<RestartOutcome, AppServerError>
    where
        F: FnMut(AppServerClient) -> Fut,
        Fut: Future<Output = Result<(), AppServerError>>,
        BeforeInstall: FnOnce(&AppServerClient, &ResidentForwarders),
    {
        let _guard = self.restart_lock.lock().await;
        if let Some(outcome) = self.unneeded_restart(expected_generation, forced_generation) {
            return Ok(outcome);
        }
        if !self.prepare_replacement_using(&mut cleanup).await? {
            return Ok(RestartOutcome::Deferred);
        }
        let old = match self.state.restart_candidate(expected_generation)? {
            RestartCandidate::NotPending => return Ok(RestartOutcome::Settled),
            RestartCandidate::Busy => return Ok(RestartOutcome::Deferred),
            RestartCandidate::Sealed(client) => client,
        };
        let new_generation = self.state.replacement_generation();
        self.stop_forwarders().await;
        cleanup(old).await?;
        let generation_rx = self.forwarder_generation.subscribe();
        let mut recorded_cleanup = None;
        let replacement_start =
            AppServerClient::start_observed_fallible(self.config.clone(), |client| {
                self.state.record_replacement(client, new_generation)?;
                recorded_cleanup = Some(ReplacementCleanup {
                    client: client.clone(),
                    generation: new_generation,
                });
                observe(client);
                Ok(prepare_forwarders(
                    client,
                    new_generation,
                    self.notifications.clone(),
                    self.server_requests.clone(),
                    generation_rx,
                )
                .with_death_monitor(client, self.state.clone(), new_generation))
            })
            .await;
        let (replacement, forwarders) = match replacement_start {
            Ok(replacement) => replacement,
            Err(error @ AppServerError::StartupCleanup { .. }) => return Err(error),
            Err(primary) => {
                let Some(cleanup_debt) = recorded_cleanup else {
                    return Err(primary);
                };
                if let Err(cleanup) = self.state.finish_replacement_cleanup(&cleanup_debt) {
                    return Err(AppServerError::StartupCleanup {
                        primary: Box::new(primary),
                        cleanup: Box::new(cleanup),
                    });
                }
                return Err(primary);
            }
        };
        before_install(&replacement, &forwarders);
        let cleanup_debt = recorded_cleanup.expect("successful replacement was recorded");
        let mut staged_forwarders = Some(forwarders);
        let install = self
            .state
            .install_replacement_with(&replacement, new_generation, || {
                let mut forwarder_slot = self.forwarders.lock().expect("resident forwarders lock");
                if forwarder_slot.is_some() {
                    return Err(AppServerError::ReplacementState {
                        message: "resident forwarder slot was not empty before replacement"
                            .to_owned(),
                    });
                }
                *forwarder_slot = staged_forwarders.take();
                self.forwarder_generation.send_replace(new_generation);
                forwarder_slot
                    .as_ref()
                    .expect("stored replacement forwarders")
                    .activate();
                Ok(())
            });
        match install {
            Ok(()) => Ok(RestartOutcome::Restarted),
            Err(primary) => {
                if let Some(forwarders) = staged_forwarders.take() {
                    forwarders.join().await;
                }
                match cleanup(replacement).await {
                    Ok(()) => match self.state.finish_replacement_cleanup(&cleanup_debt) {
                        Ok(()) => Err(primary),
                        Err(cleanup) => Err(AppServerError::StartupCleanup {
                            primary: Box::new(primary),
                            cleanup: Box::new(cleanup),
                        }),
                    },
                    Err(cleanup) => Err(AppServerError::StartupCleanup {
                        primary: Box::new(primary),
                        cleanup: Box::new(cleanup),
                    }),
                }
            }
        }
    }

    fn unneeded_restart(
        &self,
        expected_generation: Option<u64>,
        forced_generation: Option<u64>,
    ) -> Option<RestartOutcome> {
        if forced_generation.is_some_and(|generation| self.state.generation() != generation) {
            return Some(RestartOutcome::Restarted);
        }
        if forced_generation.is_some() {
            self.state.request_restart();
        }
        expected_generation
            .is_some_and(|generation| !self.state.restart_pending_for(generation))
            .then_some(RestartOutcome::Settled)
    }

    async fn prepare_replacement_using<F, Fut>(
        &self,
        cleanup_client: &mut F,
    ) -> Result<bool, AppServerError>
    where
        F: FnMut(AppServerClient) -> Fut,
        Fut: Future<Output = Result<(), AppServerError>>,
    {
        if let Some(cleanup) = self.state.replacement_cleanup() {
            cleanup_client(cleanup.client.clone()).await?;
            self.state.finish_replacement_cleanup(&cleanup)?;
        }
        self.fence_dead_generation_before_restart().await
    }

    #[cfg(test)]
    pub(in crate::manager) async fn restart_if_quiescent_observed_with_cleanup<F, Fut>(
        &self,
        observe: impl FnOnce(&AppServerClient),
        cleanup: F,
    ) -> Result<bool, AppServerError>
    where
        F: FnMut(AppServerClient) -> Fut,
        Fut: Future<Output = Result<(), AppServerError>>,
    {
        let outcome = self
            .restart_if_quiescent_using(None, None, observe, cleanup, |_, _| {})
            .await?;
        Ok(outcome == RestartOutcome::Restarted)
    }

    #[cfg(test)]
    pub(in crate::manager) async fn restart_if_quiescent_with_before_install(
        &self,
        before_install: impl FnOnce(&AppServerClient, &ResidentForwarders),
    ) -> Result<bool, AppServerError> {
        let outcome = self
            .restart_if_quiescent_using(
                None,
                None,
                |_| {},
                |client| async move { client.close().await },
                before_install,
            )
            .await?;
        Ok(outcome == RestartOutcome::Restarted)
    }

    pub async fn force_restart_if_quiescent(&self) -> Result<bool, AppServerError> {
        let observed_generation = self.state.generation();
        let outcome = self
            .restart_if_quiescent_using(
                Some(observed_generation),
                Some(observed_generation),
                |_| {},
                |client| async move { client.close().await },
                |_, _| {},
            )
            .await?;
        Ok(outcome == RestartOutcome::Restarted)
    }

    pub(super) async fn restart_generation_if_quiescent(
        &self,
        expected_generation: u64,
    ) -> Result<bool, AppServerError> {
        let outcome = self
            .restart_if_quiescent_using(
                Some(expected_generation),
                None,
                |_| {},
                |client| async move { client.close().await },
                |_, _| {},
            )
            .await?;
        Ok(outcome != RestartOutcome::Deferred)
    }
}
