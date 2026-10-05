use super::{PendingResponse, insert, spawn_deadline, take};
use crate::RequestId;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn p03a_old_registration_cannot_remove_a_new_occurrence() {
    let client = super::tests::test_client();
    let id = RequestId::String("same-id-new-registration".into());
    let (first, first_receiver) =
        PendingResponse::new(client.inner.lifecycle.admit().expect("permit"));
    let first_registration = super::register(
        &client.inner,
        id.clone(),
        first,
        Duration::from_secs(30),
        true,
    )
    .expect("first registration");
    drop(take(&client.inner, &id));
    drop(first_receiver);
    let (second, second_receiver) =
        PendingResponse::new(client.inner.lifecycle.admit().expect("permit"));
    let second_registration = super::register(
        &client.inner,
        id.clone(),
        second,
        Duration::from_secs(30),
        true,
    )
    .expect("replacement registration");
    drop(first_registration);
    assert_eq!(client.inner.pending.lock().expect("pending").len(), 1);
    drop(second_registration);
    assert_eq!(client.inner.pending.lock().expect("pending").len(), 0);
    assert!(second_receiver.await.is_err());
}

#[tokio::test(start_paused = true)]
async fn p03a_old_deadline_cannot_expire_a_reused_response_id() {
    let client = super::tests::test_client();
    let id = RequestId::String("same-id-new-occurrence".into());
    let (first, first_receiver) =
        PendingResponse::new(client.inner.lifecycle.admit().expect("permit"));
    drop(insert(&client.inner, id.clone(), first));
    spawn_deadline(
        Arc::downgrade(&client.inner),
        id.clone(),
        Duration::from_millis(100),
    );
    tokio::task::yield_now().await;
    drop(take(&client.inner, &id));
    drop(first_receiver);
    let (second, mut second_receiver) =
        PendingResponse::new(client.inner.lifecycle.admit().expect("permit"));
    drop(insert(&client.inner, id.clone(), second));
    spawn_deadline(
        Arc::downgrade(&client.inner),
        id.clone(),
        Duration::from_secs(10),
    );
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(100)).await;
    tokio::task::yield_now().await;
    assert!(
        matches!(
            second_receiver.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ),
        "old occurrence's deadline must not retire the replacement"
    );
    assert_eq!(client.inner.pending.lock().expect("pending").len(), 1);
    drop(take(&client.inner, &id));
}
