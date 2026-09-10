use cdr_discord::gateway::{GatewayShutdownError, GatewayShutdownReport};

use super::ShutdownCause;
use crate::discord_runtime::DiscordRuntimeError;
use crate::discord_runtime::worker_supervision::WorkerShutdownReport;

fn worker_report(
    trigger: Option<Result<(), DiscordRuntimeError>>,
    cleanup: Result<(), DiscordRuntimeError>,
) -> WorkerShutdownReport {
    WorkerShutdownReport::from_parts(trigger, cleanup)
}

fn gateway_report(
    trigger: Option<Result<(), GatewayShutdownError>>,
    cleanup: Result<(), GatewayShutdownError>,
) -> GatewayShutdownReport {
    GatewayShutdownReport::from_parts(trigger, cleanup)
}

#[test]
fn wsu_04_triggering_worker_preserves_its_exact_error() {
    let error = ShutdownCause::Worker("message")
        .propagate(
            gateway_report(None, Ok(())),
            worker_report(
                Some(Err(DiscordRuntimeError::TypedIngressClosed("message"))),
                Ok(()),
            ),
            Ok(()),
            Ok(()),
        )
        .expect_err("exact worker failure is fatal");

    assert!(matches!(
        error,
        DiscordRuntimeError::TypedIngressClosed("message")
    ));
}

#[test]
fn wsu_05_clean_unexpected_exit_is_still_fatal() {
    let error = ShutdownCause::Worker("completion")
        .propagate(
            gateway_report(None, Ok(())),
            worker_report(Some(Ok(())), Ok(())),
            Ok(()),
            Ok(()),
        )
        .expect_err("clean early exit is unexpected");

    assert!(matches!(
        error,
        DiscordRuntimeError::WorkerExited {
            worker: "completion"
        }
    ));
}

#[test]
fn wsu_06_triggering_gateway_preserves_join_or_timeout_failure() {
    let error = ShutdownCause::GatewayShard(3)
        .propagate(
            gateway_report(Some(Err(GatewayShutdownError::Timeout)), Ok(())),
            worker_report(None, Ok(())),
            Ok(()),
            Ok(()),
        )
        .expect_err("gateway cleanup failure is more precise than early exit");

    assert!(matches!(
        error,
        DiscordRuntimeError::GatewayShutdown(GatewayShutdownError::Timeout)
    ));
}

#[test]
fn wsu_09_clean_triggering_exit_is_not_overwritten_by_cleanup_failure() {
    let error = ShutdownCause::Worker("completion")
        .propagate(
            gateway_report(None, Ok(())),
            worker_report(
                Some(Ok(())),
                Err(DiscordRuntimeError::TypedIngressClosed("cleanup")),
            ),
            Ok(()),
            Ok(()),
        )
        .expect_err("the unexpected exit remains primary");

    assert!(matches!(
        error,
        DiscordRuntimeError::WorkerExited {
            worker: "completion"
        }
    ));
}
