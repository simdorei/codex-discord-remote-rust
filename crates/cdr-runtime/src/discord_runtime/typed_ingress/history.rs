use std::future::pending;

use cdr_discord::gateway::ingress::MessageGapReceiver;
use cdr_discord::gateway::{GatewayIdentityConflictReceiver, GatewayIdentityReceiver};
use tokio::sync::{broadcast, watch};
use tokio::time::{Instant, MissedTickBehavior, interval_at};

use super::super::DiscordRuntimeError;
use super::TypedIngressContext;
use super::identity::{fail_on_conflict, guard, handle_conflict_change, wait_for_identity};
use crate::history_poll::HistoryPollState;

mod cycle;
mod gap;

pub(super) async fn run(
    mut identity_receiver: GatewayIdentityReceiver,
    mut conflict: GatewayIdentityConflictReceiver,
    mut gaps: MessageGapReceiver,
    context: TypedIngressContext,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), DiscordRuntimeError> {
    let Some(identity) =
        wait_for_identity(&mut identity_receiver, &mut conflict, &mut shutdown).await?
    else {
        return Ok(());
    };
    let mut poll = context.config.history_poll_interval.map(|period| {
        let mut timer = interval_at(Instant::now() + period, period);
        timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
        timer
    });
    let mut state = HistoryPollState::default();
    let mut gap_due = true;
    let mut poll_due = poll.is_some();

    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        fail_on_conflict(&conflict)?;
        if gap_due {
            let recovery = gap::recover_pending(&gaps, identity, &context);
            let Some(result) = guard(recovery, &mut conflict, &mut shutdown).await? else {
                return Ok(());
            };
            result?;
            gap_due = false;
            continue;
        }
        if poll_due {
            let cycle = cycle::run_periodic(&mut state, identity, &gaps, &context);
            let Some(result) = guard(cycle, &mut conflict, &mut shutdown).await? else {
                return Ok(());
            };
            result?;
            poll_due = false;
            gap_due = true;
            continue;
        }

        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
            changed = conflict.changed() => {
                handle_conflict_change(&changed, &conflict, &shutdown)?;
            }
            changed = gaps.changed() => {
                match changed {
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => gap_due = true,
                    Err(broadcast::error::RecvError::Closed) if *shutdown.borrow() => return Ok(()),
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(DiscordRuntimeError::TypedIngressClosed("message-gap"));
                    }
                }
            }
            () = async {
                match &mut poll {
                    Some(timer) => { timer.tick().await; }
                    None => pending::<()>().await,
                }
            } => {
                poll_due = true;
                gap_due = true;
            }
        }
    }
}
