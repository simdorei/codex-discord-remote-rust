use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use super::shutdown::{GatewayTask, join_gateway_tasks, join_gateway_tasks_for_cause};
use super::{GatewayRuntime, GatewayShutdownError, ingress::GatewayIngressConfig};
use tokio::sync::mpsc;

const SHARD_SOURCE: &str = include_str!("gateway/shard.rs");
const SHUTDOWN_SOURCE: &str = include_str!("gateway/shutdown.rs");

#[tokio::test]
async fn gi_rt_shutdown_01_join_failure_still_drains_remaining_tasks() {
    let mut runtime = GatewayRuntime::new_offline_for_test(GatewayIngressConfig::default())
        .expect("default config is valid");
    let second_finished = Arc::new(AtomicBool::new(false));
    let task_finished = Arc::clone(&second_finished);
    runtime.tasks = vec![
        GatewayTask::new(0, tokio::spawn(async { panic!("offline shard failure") })),
        GatewayTask::new(
            1,
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                task_finished.store(true, Ordering::Release);
            }),
        ),
    ];

    let result = runtime.shutdown().await;

    assert!(matches!(result, Err(GatewayShutdownError::Join(_))));
    assert!(second_finished.load(Ordering::Acquire));
}

#[test]
fn gi_rt_shutdown_02_close_request_disables_the_repeating_watch_branch() {
    assert!(SHARD_SOURCE.contains("changed = shutdown.changed(), if !closing"));
    assert!(SHARD_SOURCE.contains("closing = true"));
    assert!(SHARD_SOURCE.contains("closing || *shutdown.borrow()"));
}

#[tokio::test]
async fn gi_rt_shutdown_03_timeout_aborts_and_joins_every_remaining_task() {
    struct DropCount(Arc<AtomicUsize>);

    impl Drop for DropCount {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    let dropped = Arc::new(AtomicUsize::new(0));
    let (ready, mut readiness) = mpsc::channel(2);
    let mut tasks = (0..2)
        .map(|_| {
            let dropped = Arc::clone(&dropped);
            let ready = ready.clone();
            tokio::spawn(async move {
                let _guard = DropCount(dropped);
                ready.send(()).await.expect("readiness receiver stays open");
                std::future::pending::<()>().await;
            })
        })
        .collect::<Vec<_>>();
    drop(ready);
    assert_eq!(readiness.recv().await, Some(()));
    assert_eq!(readiness.recv().await, Some(()));

    let result = join_gateway_tasks(&mut tasks, Duration::from_millis(100)).await;

    assert_eq!(
        result.unwrap_err().to_string(),
        GatewayShutdownError::Timeout.to_string()
    );
    assert!(tasks.is_empty());
    assert_eq!(dropped.load(Ordering::Acquire), 2);
}

#[tokio::test]
async fn gi_rt_shutdown_04_drop_aborts_tasks_that_ignore_the_shutdown_signal() {
    struct Dropped(Arc<AtomicBool>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    let dropped = Arc::new(AtomicBool::new(false));
    let task_dropped = Arc::clone(&dropped);
    let mut runtime = GatewayRuntime::new_offline_for_test(GatewayIngressConfig::default())
        .expect("default config is valid");
    runtime.tasks.push(GatewayTask::new(
        0,
        tokio::spawn(async move {
            let _guard = Dropped(task_dropped);
            std::future::pending::<()>().await;
        }),
    ));
    tokio::task::yield_now().await;

    drop(runtime);
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }

    assert!(dropped.load(Ordering::Acquire));
}

#[test]
fn gi_rt_shutdown_05_hard_expiry_escalates_instead_of_detaching() {
    assert!(SHUTDOWN_SOURCE.contains("timeout_at(deadline, &mut task.handle)"));
    assert!(SHUTDOWN_SOURCE.contains("std::process::abort()"));
}

#[tokio::test]
async fn gi_rt_shutdown_06_triggering_shard_panic_precedes_earlier_cleanup_panic() {
    let (release_cleanup, cleanup_released) = tokio::sync::oneshot::channel();
    let (trigger_started, trigger_observed) = tokio::sync::oneshot::channel();
    let cleanup = GatewayTask::new(
        1,
        tokio::spawn(async move {
            cleanup_released.await.expect("cleanup release is sent");
            panic!("cleanup shard panic");
        }),
    );
    let trigger = GatewayTask::new(
        2,
        tokio::spawn(async move {
            trigger_started
                .send(())
                .expect("test receives trigger start");
            panic!("trigger shard panic");
        }),
    );
    trigger_observed.await.expect("trigger task starts");
    release_cleanup.send(()).expect("cleanup task is alive");
    let mut tasks = vec![cleanup, trigger];

    let report = join_gateway_tasks_for_cause(
        &mut tasks,
        tokio::time::Instant::now() + Duration::from_secs(1),
        Some(2),
    )
    .await;
    let (trigger, cleanup) = report.into_parts();

    assert_eq!(
        panic_payload(trigger.expect("trigger shard is present")),
        "trigger shard panic"
    );
    assert_eq!(panic_payload(cleanup), "cleanup shard panic");
    assert!(tasks.is_empty());
}

fn panic_payload(result: Result<(), GatewayShutdownError>) -> String {
    let GatewayShutdownError::Join(error) = result.expect_err("shard must panic") else {
        panic!("expected gateway join failure");
    };
    let payload = error.into_panic();
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        panic!("gateway panic payload was not text");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[should_panic(expected = "fatal gateway shutdown timeout")]
async fn gi_rt_shutdown_07_abort_resistant_task_hits_the_hard_deadline() {
    let (started, observed) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        started.send(()).expect("test receives readiness");
        std::thread::sleep(Duration::from_millis(250));
    });
    observed.await.expect("abort-resistant task starts");
    let mut tasks = vec![task];

    let _ = join_gateway_tasks(&mut tasks, Duration::from_millis(25)).await;
}
