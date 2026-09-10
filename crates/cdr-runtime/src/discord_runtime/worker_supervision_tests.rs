use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;
use std::{process::Command, str};

use super::{WorkerSet, exit_channel, spawn_monitored};
use crate::discord_runtime::DiscordRuntimeError;
use crate::discord_runtime::shutdown_deadline::terminate_on_shutdown_timeout;
use tokio::sync::oneshot;

const PRIMARY_TIMEOUT_PROBE_ENV: &str = "CDR_PRIMARY_TIMEOUT_PROBE";

#[tokio::test]
async fn wsu_01_clean_early_exit_is_observed_without_polling() {
    let (notifier, mut exits) = exit_channel();
    let worker = spawn_monitored("clean", notifier, async { Ok(()) });

    assert_eq!(exits.recv().await, Some("clean"));
    WorkerSet::from_workers([worker])
        .shutdown_until(tokio::time::Instant::now() + Duration::from_secs(1))
        .await
        .expect("completed worker joins cleanly");
}

#[tokio::test]
async fn wsu_02_panic_is_observed_and_preserved_with_worker_name() {
    let (notifier, mut exits) = exit_channel();
    let worker = spawn_monitored("panicking", notifier, async {
        panic!("injected worker panic");
    });

    assert_eq!(exits.recv().await, Some("panicking"));
    let error = WorkerSet::from_workers([worker])
        .shutdown_until(tokio::time::Instant::now() + Duration::from_secs(1))
        .await
        .expect_err("panic must remain fatal");
    assert!(matches!(
        error,
        DiscordRuntimeError::WorkerTask {
            worker: "panicking",
            ref source,
        } if source.is_panic()
    ));
}

#[tokio::test(start_paused = true)]
async fn wsu_03_one_deadline_aborts_and_joins_every_stuck_worker() {
    struct Dropped(Arc<AtomicUsize>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    let dropped = Arc::new(AtomicUsize::new(0));
    let (notifier, _exits) = exit_channel();
    let workers = (0..2)
        .map(|index| {
            let dropped = Arc::clone(&dropped);
            spawn_monitored(
                if index == 0 { "stuck-a" } else { "stuck-b" },
                notifier.clone(),
                async move {
                    let _guard = Dropped(dropped);
                    std::future::pending::<()>().await;
                    Ok(())
                },
            )
        })
        .collect::<Vec<_>>();
    tokio::task::yield_now().await;

    let error = WorkerSet::from_workers(workers)
        .shutdown_until(tokio::time::Instant::now() + Duration::from_secs(2))
        .await
        .expect_err("forced shutdown is visible");

    assert!(matches!(
        error,
        DiscordRuntimeError::WorkerShutdownTimeout { ref workers }
            if workers == "stuck-a,stuck-b"
    ));
    assert_eq!(dropped.load(Ordering::Acquire), 2);
}

#[tokio::test]
async fn wsu_07_expired_deadline_still_joins_an_aborted_worker() {
    struct Dropped(Arc<AtomicBool>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    let dropped = Arc::new(AtomicBool::new(false));
    let (ready, started) = oneshot::channel();
    let (notifier, _exits) = exit_channel();
    let worker = spawn_monitored("expired", notifier, {
        let dropped = Arc::clone(&dropped);
        async move {
            let _guard = Dropped(dropped);
            ready.send(()).expect("test receives readiness");
            std::future::pending::<()>().await;
            Ok(())
        }
    });
    started.await.expect("worker starts");

    let error = WorkerSet::from_workers([worker])
        .shutdown_until(tokio::time::Instant::now() + Duration::from_millis(100))
        .await
        .expect_err("expired drain phase forces a visible abort");

    assert!(matches!(
        error,
        DiscordRuntimeError::WorkerShutdownTimeout { ref workers } if workers == "expired"
    ));
    assert!(dropped.load(Ordering::Acquire));
}

#[tokio::test]
async fn wsu_08_triggering_panic_stays_primary_over_earlier_cleanup_failure() {
    let (release_cleanup, cleanup_released) = oneshot::channel();
    let (notifier, mut exits) = exit_channel();
    let cleanup = spawn_monitored("cleanup", notifier.clone(), async move {
        cleanup_released.await.expect("cleanup release is sent");
        Err(DiscordRuntimeError::TypedIngressClosed("cleanup"))
    });
    let trigger = spawn_monitored("trigger", notifier, async {
        panic!("injected triggering panic");
    });
    assert_eq!(exits.recv().await, Some("trigger"));
    release_cleanup.send(()).expect("cleanup worker is alive");

    let report = WorkerSet::from_workers([cleanup, trigger])
        .shutdown_for_cause(
            tokio::time::Instant::now() + Duration::from_secs(1),
            Some("trigger"),
        )
        .await;
    let (trigger, cleanup) = report.into_parts();

    assert!(matches!(
        trigger.expect("trigger worker is present").expect_err("trigger panics"),
        DiscordRuntimeError::WorkerTask {
            worker: "trigger",
            ref source,
        } if source.is_panic()
    ));
    assert!(matches!(
        cleanup.expect_err("cleanup failure remains visible separately"),
        DiscordRuntimeError::TypedIngressClosed("cleanup")
    ));
}

#[tokio::test]
async fn wsu_11_triggering_returned_error_stays_separate_from_cleanup_failure() {
    let (release_cleanup, cleanup_released) = oneshot::channel();
    let (notifier, mut exits) = exit_channel();
    let cleanup = spawn_monitored("cleanup", notifier.clone(), async move {
        cleanup_released.await.expect("cleanup release is sent");
        Err(DiscordRuntimeError::TypedIngressClosed("cleanup"))
    });
    let trigger = spawn_monitored("trigger", notifier, async {
        Err(DiscordRuntimeError::InvalidHistoryChannel)
    });
    assert_eq!(exits.recv().await, Some("trigger"));
    release_cleanup.send(()).expect("cleanup worker is alive");

    let report = WorkerSet::from_workers([cleanup, trigger])
        .shutdown_for_cause(
            tokio::time::Instant::now() + Duration::from_secs(1),
            Some("trigger"),
        )
        .await;
    let (trigger, cleanup) = report.into_parts();

    assert!(matches!(
        trigger.expect("trigger worker is present"),
        Err(DiscordRuntimeError::InvalidHistoryChannel)
    ));
    assert!(matches!(
        cleanup,
        Err(DiscordRuntimeError::TypedIngressClosed("cleanup"))
    ));
}

#[test]
#[should_panic(expected = "fatal runtime shutdown timeout: test-worker")]
fn wsu_12_hard_deadline_expiry_escalates_instead_of_returning() {
    terminate_on_shutdown_timeout("test-worker");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[should_panic(expected = "fatal runtime shutdown timeout: runtime-worker")]
async fn wsu_14_abort_resistant_worker_hits_the_hard_deadline() {
    let (started, observed) = oneshot::channel();
    let (_release, blocked) = std::sync::mpsc::channel::<()>();
    let (notifier, _exits) = exit_channel();
    let worker = spawn_monitored("abort-resistant", notifier, async move {
        started.send(()).expect("test receives readiness");
        // Parent unwinding drops the sender after the expected hard-timeout panic.
        let _ = blocked.recv_timeout(Duration::from_secs(30));
        Ok(())
    });
    observed.await.expect("abort-resistant worker starts");

    let _ = WorkerSet::from_workers([worker])
        .shutdown_until(tokio::time::Instant::now() + Duration::from_millis(25))
        .await;
}

#[test]
#[ignore = "spawned by the primary-cause timeout regression test"]
fn wsu_primary_error_hard_timeout_probe() {
    if std::env::var_os(PRIMARY_TIMEOUT_PROBE_ENV).is_none() {
        return;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("probe runtime");
    runtime.block_on(async {
        let (notifier, mut exits) = exit_channel();
        let (started, observed) = oneshot::channel();
        let (_release, blocked) = std::sync::mpsc::channel::<()>();
        let cleanup = spawn_monitored("abort-resistant-cleanup", notifier.clone(), async move {
            started.send(()).expect("probe receives cleanup readiness");
            let _ = blocked.recv_timeout(Duration::from_secs(30));
            std::future::pending::<Result<(), DiscordRuntimeError>>().await
        });
        observed
            .await
            .expect("cleanup is inside its abort-resistant poll");
        let trigger = spawn_monitored("primary-trigger", notifier, async {
            Err(DiscordRuntimeError::TypedIngressClosed("primary-probe"))
        });
        assert_eq!(exits.recv().await, Some("primary-trigger"));

        let _ = WorkerSet::from_workers([cleanup, trigger])
            .shutdown_for_cause(
                tokio::time::Instant::now() + Duration::from_millis(25),
                Some("primary-trigger"),
            )
            .await;
    });
}

#[test]
fn wsu_15_hard_timeout_retains_the_exact_triggering_error_in_diagnostics() {
    let output = Command::new(std::env::current_exe().expect("current test executable"))
        .args([
            "--exact",
            "discord_runtime::worker_supervision::tests::wsu_primary_error_hard_timeout_probe",
            "--ignored",
            "--nocapture",
        ])
        .env(PRIMARY_TIMEOUT_PROBE_ENV, "1")
        .env("CDR_TEST_REAL_SHUTDOWN_ABORT", "1")
        .output()
        .expect("run hard-timeout diagnostics probe");
    assert!(!output.status.success(), "probe must hit fatal timeout");
    let diagnostics = format!(
        "{}{}",
        str::from_utf8(&output.stdout).expect("UTF-8 stdout"),
        str::from_utf8(&output.stderr).expect("UTF-8 stderr")
    );

    assert!(
        diagnostics.contains(
            "primary_runtime_shutdown_error worker=primary-trigger error=Discord typed ingress primary-probe channel closed before shutdown"
        ),
        "exact primary cause missing from fatal diagnostics: {diagnostics}"
    );
    assert!(
        diagnostics
            .contains("fatal_runtime_worker_abort_join_timeout worker=abort-resistant-cleanup"),
        "hard-timeout diagnostic missing: {diagnostics}"
    );
}
