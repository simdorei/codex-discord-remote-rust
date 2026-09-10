use crate::discord_dispatch::DiscordDispatchError;

use super::*;

#[derive(Clone, Copy)]
enum Failure {
    Acknowledge,
    Update,
    Invariant,
}

#[derive(Clone)]
struct FailureThenSuccess {
    failure: Failure,
    observed: mpsc::UnboundedSender<u64>,
}

impl InteractionHandler for FailureThenSuccess {
    fn handle(&self, item: InteractionIngress) -> InteractionFuture {
        let handler = self.clone();
        Box::pin(async move {
            handler.observed.send(item.sequence).unwrap();
            if item.sequence == 1 {
                let error = match handler.failure {
                    Failure::Acknowledge => DiscordDispatchError::Acknowledge("HTTP 503".into()),
                    Failure::Update => DiscordDispatchError::Update("HTTP 404 expired".into()),
                    Failure::Invariant => DiscordDispatchError::ClaimState,
                };
                return Err(error.into());
            }
            Ok(())
        })
    }
}

#[tokio::test]
async fn interaction_http_failure_does_not_stop_next_event_in_either_lane() {
    for (lane, tag) in [
        (InteractionLane::Normal, InteractionIngressTag::Normal),
        (InteractionLane::Reserved, InteractionIngressTag::Stopping),
    ] {
        for failure in [Failure::Acknowledge, Failure::Update] {
            let (sender, receiver) = mpsc::channel(2);
            let (observed, mut events) = mpsc::unbounded_channel();
            let (shutdown, shutdown_rx) = watch::channel(false);
            sender.send(super::tests::item(1, tag)).await.unwrap();
            sender.send(super::tests::item(2, tag)).await.unwrap();
            let mut runner = tokio::spawn(run_lane(
                receiver,
                lane,
                1,
                FailureThenSuccess { failure, observed },
                shutdown_rx,
            ));
            assert_eq!(events.recv().await, Some(1));
            tokio::select! {
                biased;
                ended = &mut runner => panic!("one HTTP failure stopped the lane: {ended:?}"),
                next = events.recv() => assert_eq!(next, Some(2)),
                () = tokio::time::sleep(Duration::from_secs(2)) => panic!("next interaction stalled"),
            }
            shutdown.send(true).unwrap();
            drop(sender);
            tokio::time::timeout(Duration::from_secs(2), runner)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
    }
}

#[tokio::test]
async fn interaction_invariant_failure_still_stops_the_lane_with_original_error() {
    let (_sender, receiver) = super::tests::filled_lane(1, InteractionIngressTag::Normal);
    let (observed, _events) = mpsc::unbounded_channel();
    let (_shutdown, shutdown_rx) = watch::channel(false);
    let result = run_lane(
        receiver,
        InteractionLane::Normal,
        1,
        FailureThenSuccess {
            failure: Failure::Invariant,
            observed,
        },
        shutdown_rx,
    )
    .await;
    assert!(matches!(
        result,
        Err(DiscordRuntimeError::Dispatch(
            DiscordDispatchError::ClaimState
        ))
    ));
}

#[test]
fn interaction_http_error_preserves_reason_but_never_reports_interaction_token() {
    let error = DiscordDispatchError::Update("HTTP 404 /webhooks/test-secret-token expired".into());
    let public = failure::redacted_error(&error, "test-secret-token");
    assert!(public.contains("HTTP 404"));
    assert!(public.contains("expired"));
    assert!(public.contains("[REDACTED]"));
    assert!(!public.contains("test-secret-token"));
}
