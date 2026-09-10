use super::*;
use crate::{
    Notification,
    requests::{ServiceTierUpdate, ThreadSettingsUpdate},
};

#[tokio::test]
async fn settings_watermark_excludes_matching_notification_before_writer_admission() {
    let pause = Arc::new(WriteTestPause::new());
    let client = test_client(pause.clone());
    let server = Arc::new(server_from_client(client.clone()));
    let writer = client.inner.stdin.lock().await;
    let request = tokio::spawn({
        let server = server.clone();
        async move {
            server
                .update_settings_with_watermark(
                    "thread-b",
                    &ThreadSettingsUpdate {
                        model: Some("model-b".into()),
                        effort: None,
                        service_tier: ServiceTierUpdate::Unchanged,
                    },
                    1,
                )
                .await
        }
    });
    timeout(Duration::from_secs(2), pause.wait_until_before_lock())
        .await
        .unwrap();
    client.inner.state.lock().unwrap().record_notification(Notification {
        method:"thread/settings/updated".into(),params:json!({"threadId":"thread-b","threadSettings":{"model":"model-b","effort":"high","serviceTier":null}})
    });
    let premature = server
        .observed_thread_settings("thread-b", 1)
        .unwrap()
        .unwrap()
        .0;
    drop(writer);
    timeout(Duration::from_secs(2), pause.wait_until_entered())
        .await
        .unwrap();
    let id = client
        .inner
        .pending
        .lock()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();
    crate::client::take_pending_response(&client.inner, &id)
        .unwrap()
        .respond(Ok(json!({})));
    pause.release();
    let watermark = timeout(Duration::from_secs(2), request)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    server.close().await.unwrap();
    assert!(
        watermark >= premature,
        "pre-dispatch matching notification would pass revision > watermark"
    );
}
