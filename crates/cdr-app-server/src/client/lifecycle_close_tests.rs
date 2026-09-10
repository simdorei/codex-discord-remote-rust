use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::Duration;

use tokio::sync::{Mutex as AsyncMutex, broadcast};
use tokio::time::timeout;

use super::ClientLifecycle;
use crate::RequestId;
use crate::client::{Inner, PendingOutcome, PendingResponse};
use crate::diagnostics::BoundedDiagnostics;
use crate::state::RuntimeState;
use crate::transport::mark_closed;

fn test_inner() -> Arc<Inner> {
    let (notifications, _) = broadcast::channel(1);
    let (server_requests, _) = broadcast::channel(1);
    Arc::new(Inner {
        child: AsyncMutex::new(None),
        closed: AtomicBool::new(false),
        diagnostics: Mutex::new(BoundedDiagnostics::default()),
        lifecycle: Arc::new(ClientLifecycle::new()),
        notifications,
        pending: Mutex::new(HashMap::new()),
        server_requests,
        state: Mutex::new(RuntimeState::starting(None)),
        stdin: AsyncMutex::new(None),
        write_pause: Mutex::new(None),
    })
}

#[tokio::test]
async fn losing_close_cannot_publish_before_the_winner_finishes_pending_cleanup() {
    let inner = test_inner();
    let permit = inner.lifecycle.admit().expect("admit pending request");
    let (pending, pending_result) = PendingResponse::new(permit);
    inner
        .pending
        .lock()
        .expect("pending response lock")
        .insert(RequestId::String("pending".to_owned()), pending);

    let (first, second, mut close_waiter, second_completed, published_early) = {
        let pending_guard = inner.pending.lock().expect("hold pending response lock");
        let first_inner = Arc::clone(&inner);
        let first = thread::spawn(move || mark_closed(&first_inner, "first close"));
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if inner
                .state
                .lock()
                .expect("runtime state lock")
                .closed_reason
                .as_deref()
                == Some("first close")
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "first close did not claim the canonical reason"
            );
            thread::yield_now();
        }

        let mut close_waiter = Box::pin(inner.lifecycle.wait_closed());
        let (second_done, second_finished) = mpsc::sync_channel(1);
        let second_inner = Arc::clone(&inner);
        let second = thread::spawn(move || {
            mark_closed(&second_inner, "second close");
            let _ = second_done.send(());
        });
        let second_completed = second_finished.recv_timeout(Duration::from_secs(2)).is_ok();
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let published_early = matches!(close_waiter.as_mut().poll(&mut context), Poll::Ready(_));
        drop(pending_guard);
        (
            first,
            second,
            close_waiter,
            second_completed,
            published_early,
        )
    };

    first.join().expect("first close thread");
    second.join().expect("second close thread");
    assert!(second_completed, "losing close waited on pending cleanup");
    assert!(!published_early, "losing close published the close signal");
    assert_eq!(
        timeout(Duration::from_secs(2), &mut close_waiter)
            .await
            .expect("close signal timeout"),
        "first close"
    );
    match timeout(Duration::from_secs(2), pending_result)
        .await
        .expect("pending response timeout")
        .expect("pending response sender")
    {
        PendingOutcome::TransportClosed { reason } => assert_eq!(reason, "first close"),
        PendingOutcome::Response(_) | PendingOutcome::Timeout => {
            panic!("pending response did not receive transport closure")
        }
    }
    assert_eq!(inner.lifecycle.in_flight(), 0);
    assert!(
        inner
            .pending
            .lock()
            .expect("pending response lock")
            .is_empty()
    );
}
