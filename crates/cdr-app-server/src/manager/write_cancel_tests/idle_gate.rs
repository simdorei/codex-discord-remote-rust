use super::*;
use crate::idle_release::{IdleReleaseJournal, IdleReleaseToken};
use crate::requests::{ServiceTierUpdate, ThreadSettingsUpdate};
use crate::{Notification, RequestId, ServerRequest, ServerRequestOccurrence};

struct Journal;
impl IdleReleaseJournal for Journal {
    fn before_mutation(
        &self,
        _: &str,
        _: u64,
        _: &str,
    ) -> Result<Option<IdleReleaseToken>, AppServerError> {
        Ok(None)
    }
    fn check_mutation(&self, _: &str) -> Result<(), AppServerError> {
        Ok(())
    }
    fn resume_required(&self, _: &str) -> Result<bool, AppServerError> {
        Ok(false)
    }
    fn verify(&self, _: &IdleReleaseToken, _: bool) -> Result<(), AppServerError> {
        Ok(())
    }
    fn transition(
        &self,
        t: &IdleReleaseToken,
        state: &str,
        detail: &str,
    ) -> Result<IdleReleaseToken, AppServerError> {
        let mut t = t.clone();
        t.state = state.into();
        t.detail = detail.into();
        t.revision += 1;
        Ok(t)
    }
    fn old_child_exited(&self, _: &str, _: u64) -> Result<(), AppServerError> {
        Ok(())
    }
}

fn token(server: &ResidentAppServer) -> IdleReleaseToken {
    IdleReleaseToken {
        intent_id: "i".into(),
        owner_id: server.instance_id().into(),
        generation: 1,
        thread_id: "A".into(),
        turn_id: "T1".into(),
        job_id: "j".into(),
        revision: 1,
        state: "Candidate".into(),
        detail: String::new(),
    }
}

async fn mutate(
    server: &ResidentAppServer,
    request: &ServerRequest,
    route: u8,
) -> Result<(), AppServerError> {
    let occurrence = request.occurrence;
    match route {
        0 => server
            .request(
                "turn/steer",
                json!({"threadId":"A","expectedTurnId":"T1"}),
                Duration::from_secs(2),
                Some(1),
            )
            .await
            .map(|_| ()),
        1 => server
            .update_settings_with_watermark(
                "A",
                &ThreadSettingsUpdate {
                    model: Some("test".into()),
                    effort: None,
                    service_tier: ServiceTierUpdate::Unchanged,
                },
                1,
            )
            .await
            .map(|_| ()),
        2 => server.respond(&request.id, occurrence, json!({}), 1).await,
        3 => {
            server
                .respond_current(&request.id, occurrence, json!({}), 1)
                .await
        }
        _ => {
            server
                .respond_error(
                    &request.id,
                    occurrence,
                    crate::RpcErrorPayload {
                        code: -1,
                        message: "test".into(),
                        data: None,
                    },
                    1,
                )
                .await
        }
    }
}

#[tokio::test]
async fn ir10_all_mutation_routes_waiting_for_stdin_exclude_release_until_resolution() {
    for route in 0..5 {
        let pause = Arc::new(WriteTestPause::for_response_resolution());
        let client = test_client(Arc::clone(&pause));
        let server = Arc::new(server_from_client(client.clone()));
        server
            .install_idle_release_journal(Arc::new(Journal))
            .unwrap();
        let occurrence = ServerRequestOccurrence::from_bytes(7_u128.to_be_bytes());
        let request = ServerRequest {
            id: RequestId::Integer(7),
            occurrence,
            method: "item/tool/requestUserInput".into(),
            params: json!({"threadId":"A","turnId":"T1"}),
        };
        {
            let mut state = client.inner.state.lock().unwrap();
            state.record_notification(Notification {
                method: "turn/started".into(),
                params: json!({"threadId":"A","turn":{"id":"T1"}}),
            });
            state.record_server_request(request.clone()).unwrap();
        }
        let writer = client.inner.stdin.lock().await;
        let running = tokio::spawn({
            let server = Arc::clone(&server);
            async move { mutate(&server, &request, route).await }
        });
        timeout(Duration::from_secs(2), pause.wait_until_before_lock())
            .await
            .unwrap();
        let failure = server
            .target_gate
            .reserve(&token(&server))
            .err()
            .expect("ordinary mutation must exclude release");
        assert!(
            failure.to_string().contains("admitted target mutation"),
            "route {route}: {failure}"
        );
        drop(writer);
        if route < 2 {
            // Resolve the exact request after the fixture's actual write.
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
        } else {
            timeout(Duration::from_secs(2), pause.wait_until_entered())
                .await
                .unwrap();
            assert!(
                server.target_gate.reserve(&token(&server)).is_err(),
                "response permit must survive flush through claim resolution"
            );
            pause.release();
        }
        timeout(Duration::from_secs(3), running)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let reservation = server.target_gate.reserve(&token(&server)).unwrap();
        drop(reservation);
        server.close().await.unwrap();
    }
}
