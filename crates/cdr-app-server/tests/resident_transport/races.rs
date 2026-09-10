use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cdr_app_server::{
    AppServerConfig, AppServerError, ResidentAppServer, ResidentNotificationEvent,
};
use serde_json::json;
use tokio::time::timeout;
use uuid::Uuid;

use super::{approval_for_generation, fake_config};

fn close_race_config() -> (AppServerConfig, PathBuf, PathBuf) {
    let mut config = fake_config();
    let token = Uuid::new_v4();
    let entered = std::env::temp_dir().join(format!("cdr-close-entered-{token}"));
    let release = std::env::temp_dir().join(format!("cdr-close-release-{token}"));
    config.environment.extend([
        ("CDR_FAKE_LATE_INGRESS_ON_CLOSE".to_owned(), "1".to_owned()),
        (
            "CDR_FAKE_CLOSE_ENTERED_FILE".to_owned(),
            entered.to_string_lossy().into_owned(),
        ),
        (
            "CDR_FAKE_CLOSE_RELEASE_FILE".to_owned(),
            release.to_string_lossy().into_owned(),
        ),
    ]);
    (config, entered, release)
}

async fn wait_for_file(path: &Path) {
    timeout(Duration::from_secs(2), async {
        while !path.try_exists().expect("barrier path") {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("close barrier");
}

#[tokio::test]
async fn admitted_rpc_blocks_restart_and_completes_on_the_old_generation() {
    let server = Arc::new(
        timeout(
            Duration::from_secs(10),
            ResidentAppServer::start(fake_config()),
        )
        .await
        .expect("start timeout")
        .expect("start"),
    );
    let mut notifications = server.subscribe_notifications();
    let delayed = tokio::spawn({
        let server = Arc::clone(&server);
        async move {
            server
                .request(
                    "test/delayedEcho",
                    json!({"generation": 1}),
                    Duration::from_secs(2),
                    Some(1),
                )
                .await
        }
    });
    let ResidentNotificationEvent::Notification { notification, .. } =
        timeout(Duration::from_secs(2), notifications.recv())
            .await
            .expect("admission marker timeout")
            .expect("admission marker")
    else {
        panic!("unexpected notification gap");
    };
    assert_eq!(notification.method, "test/delayedEntered");
    assert!(!server.force_restart_if_quiescent().await.expect("busy"));
    let busy = server.lifecycle_snapshot().await;
    assert_eq!(busy.generation, 1);
    assert!(busy.healthy, "Busy must leave resident admission open");
    assert!(busy.restart_pending);
    assert!(!busy.quarantined);
    assert_eq!(
        server
            .request(
                "test/echo",
                json!({"while": "busy"}),
                Duration::from_secs(1),
                Some(1),
            )
            .await
            .expect("ordinary request while restart is busy"),
        json!({"while": "busy"})
    );
    server
        .request(
            "test/releaseDelayed",
            json!({}),
            Duration::from_secs(1),
            Some(1),
        )
        .await
        .expect("release");
    assert_eq!(
        delayed.await.expect("task").expect("delayed result"),
        json!({"generation": 1})
    );
    assert!(server.restart_if_quiescent().await.expect("restart"));
    assert_eq!(server.generation(), 2);
    server.close().await.expect("close");
}

#[tokio::test]
async fn restart_seal_rejects_old_admissions_and_late_ingress() {
    let (config, entered, release) = close_race_config();
    let server = Arc::new(ResidentAppServer::start(config).await.expect("start"));
    let mut requests = server.subscribe_server_requests();
    let approval = approval_for_generation(&server, &mut requests, 1).await;
    server
        .respond(&approval.id, approval.occurrence, json!({}), 1)
        .await
        .expect("settle approval");
    let restart = tokio::spawn({
        let server = Arc::clone(&server);
        async move { server.force_restart_if_quiescent().await }
    });
    wait_for_file(&entered).await;
    let error = server
        .respond(&approval.id, approval.occurrence, json!({}), 1)
        .await
        .expect_err("sealed response");
    assert!(matches!(error, AppServerError::Closed));
    assert_eq!(server.generation(), 1);
    std::fs::write(&release, b"release").expect("release close");
    assert!(restart.await.expect("task").expect("restart"));
    assert!(matches!(
        requests.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    let stale = server
        .request("test/echo", json!({}), Duration::from_secs(1), Some(1))
        .await
        .expect_err("stale generation");
    assert!(matches!(stale, AppServerError::GenerationMismatch { .. }));
    server.close().await.expect("close");
    let _ = std::fs::remove_file(entered);
    let _ = std::fs::remove_file(release);
}

#[tokio::test]
async fn abort_after_confirmed_write_keeps_generation_fail_closed() {
    let server = Arc::new(
        timeout(
            Duration::from_secs(10),
            ResidentAppServer::start(fake_config()),
        )
        .await
        .expect("start timeout")
        .expect("start"),
    );
    let mut notifications = server.subscribe_notifications();
    let request = tokio::spawn({
        let server = Arc::clone(&server);
        async move {
            server
                .request(
                    "test/delayedEcho",
                    json!({"sideEffect": true}),
                    Duration::from_secs(30),
                    Some(1),
                )
                .await
        }
    });
    let ResidentNotificationEvent::Notification { notification, .. } =
        timeout(Duration::from_secs(2), notifications.recv())
            .await
            .expect("write marker timeout")
            .expect("write marker")
    else {
        panic!("unexpected notification gap");
    };
    assert_eq!(notification.method, "test/delayedEntered");
    request.abort();
    assert!(request.await.expect_err("aborted request").is_cancelled());

    let snapshot = server.lifecycle_snapshot().await;
    assert!(snapshot.quarantined);
    assert!(snapshot.restart_pending);
    assert_eq!(snapshot.generation, 1);
    assert!(
        !timeout(Duration::from_secs(2), server.force_restart_if_quiescent())
            .await
            .expect("restart decision timeout")
            .expect("fail closed")
    );
    assert_eq!(server.generation(), 1);
    timeout(Duration::from_secs(10), server.close())
        .await
        .expect("close timeout")
        .expect("close");
}
