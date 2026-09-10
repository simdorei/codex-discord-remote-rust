//! Restart-drain admission classification for typed Discord interactions.

use cdr_discord::components::{BusyAction, ComponentId};
use cdr_discord::interaction::RoutedWork;

use super::DiscordDispatchError;
use crate::restart_readiness::drain::{AdmissionGate, AdmissionPermit, DrainGateError};

pub(super) fn admit(
    gate: Option<&AdmissionGate>,
    work: Option<&RoutedWork>,
) -> Result<(Option<AdmissionPermit>, bool), DiscordDispatchError> {
    let (Some(gate), Some(work)) = (gate, work) else {
        return Ok((None, false));
    };
    let result = if is_drain_control(work) {
        gate.try_enter_control()
    } else {
        gate.try_enter()
    };
    match result {
        Ok(permit) => Ok((Some(permit), false)),
        Err(DrainGateError::Sealed) => Ok((None, true)),
        Err(error) => Err(error.into()),
    }
}

fn is_drain_control(work: &RoutedWork) -> bool {
    matches!(
        work,
        RoutedWork::Component(
            ComponentId::Approval { .. }
                | ComponentId::BoundApproval { .. }
                | ComponentId::Input { .. }
                | ComponentId::BoundInput { .. }
                | ComponentId::Busy {
                    action: BusyAction::Stop,
                    ..
                },
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::restart_readiness::drain::DrainFenceKey;

    fn busy(action: BusyAction) -> RoutedWork {
        RoutedWork::Component(ComponentId::Busy {
            choice_id: "0123456789abcdef01234567".into(),
            action,
        })
    }

    #[test]
    fn sealed_gate_allows_only_existing_work_controls() {
        let gate = AdmissionGate::new();
        let key = DrainFenceKey::new("runtime-a", "42|99", "controls").unwrap();
        gate.seal(&key).unwrap();

        let (stop, rejected) = admit(Some(&gate), Some(&busy(BusyAction::Stop))).unwrap();
        assert!(stop.is_some());
        assert!(!rejected);
        let (queue, rejected) = admit(Some(&gate), Some(&busy(BusyAction::Queue))).unwrap();
        assert!(queue.is_none());
        assert!(rejected);

        drop(stop);
        gate.close_controls(&key).unwrap();
        let (late_stop, rejected) = admit(Some(&gate), Some(&busy(BusyAction::Stop))).unwrap();
        assert!(late_stop.is_none());
        assert!(rejected);
    }
}
