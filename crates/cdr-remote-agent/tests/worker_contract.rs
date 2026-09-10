use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use cdr_remote_agent::bridge::workers::BoundedWorkers;
use tokio::sync::Semaphore;

#[tokio::test]
async fn worker1_allows_four_running_sixteen_pending_and_rejects_the_next() {
    let gate = Arc::new(Semaphore::new(0));
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut workers = BoundedWorkers::new(4, 16);

    for value in 0..16 {
        let gate = Arc::clone(&gate);
        let running = Arc::clone(&running);
        let peak = Arc::clone(&peak);
        workers
            .submit(async move {
                let current = running.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(current, Ordering::SeqCst);
                gate.acquire().await.expect("gate remains open").forget();
                running.fetch_sub(1, Ordering::SeqCst);
                value
            })
            .expect("within pending capacity");
    }
    assert!(workers.submit(async { 16 }).is_err());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(peak.load(Ordering::SeqCst), 4);
    gate.add_permits(16);
    let mut completed = Vec::new();
    while let Some(value) = workers.join_next().await {
        completed.push(value.expect("worker task"));
    }
    completed.sort_unstable();
    assert_eq!(completed, (0..16).collect::<Vec<_>>());
}
