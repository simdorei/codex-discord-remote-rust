use std::future::Future;

use cdr_discord::gateway::{
    GatewayIdentity, GatewayIdentityConflictReceiver, GatewayIdentityReceiver,
};
use tokio::sync::{broadcast, watch};

use super::super::DiscordRuntimeError;

pub(super) async fn wait_for_identity(
    identity: &mut GatewayIdentityReceiver,
    conflict: &mut GatewayIdentityConflictReceiver,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<Option<GatewayIdentity>, DiscordRuntimeError> {
    loop {
        if *shutdown.borrow() {
            return Ok(None);
        }
        fail_on_conflict(conflict)?;
        if let Some(value) = identity.snapshot() {
            return Ok(Some(value));
        }
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(None);
                }
            }
            changed = conflict.changed() => {
                handle_conflict_change(&changed, conflict, shutdown)?;
            }
            changed = identity.changed() => {
                match changed {
                    Ok(Some(value)) => return Ok(Some(value)),
                    Ok(None) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        if *shutdown.borrow() {
                            return Ok(None);
                        }
                        return Err(DiscordRuntimeError::TypedIngressClosed("identity"));
                    }
                }
            }
        }
    }
}

pub(super) async fn guard<F>(
    future: F,
    conflict: &mut GatewayIdentityConflictReceiver,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<Option<F::Output>, DiscordRuntimeError>
where
    F: Future,
{
    tokio::pin!(future);
    loop {
        if *shutdown.borrow() {
            return Ok(None);
        }
        fail_on_conflict(conflict)?;
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(None);
                }
            }
            changed = conflict.changed() => {
                handle_conflict_change(&changed, conflict, shutdown)?;
            }
            output = &mut future => return Ok(Some(output)),
        }
    }
}

pub(super) fn fail_on_conflict(
    conflict: &GatewayIdentityConflictReceiver,
) -> Result<(), DiscordRuntimeError> {
    conflict
        .snapshot()
        .map_or(Ok(()), |value| Err(value.into()))
}

pub(super) fn handle_conflict_change(
    changed: &Result<
        Option<cdr_discord::gateway::GatewayIdentityConflict>,
        broadcast::error::RecvError,
    >,
    conflict: &GatewayIdentityConflictReceiver,
    shutdown: &watch::Receiver<bool>,
) -> Result<(), DiscordRuntimeError> {
    match changed {
        Ok(Some(value)) => Err((*value).into()),
        Ok(None) | Err(broadcast::error::RecvError::Lagged(_)) => fail_on_conflict(conflict),
        Err(broadcast::error::RecvError::Closed) if *shutdown.borrow() => Ok(()),
        Err(broadcast::error::RecvError::Closed) => {
            Err(DiscordRuntimeError::TypedIngressClosed("identity-conflict"))
        }
    }
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
