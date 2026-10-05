use super::{
    Arc, AtomicUsize, Duration, Fixture, Future, HEALTHY_B_BUDGET, Mutex, Notify, Ordering, Pin,
    SessionMirrorDeliveryIdentity, SessionMirrorSender, event, fs, get_offset, params, timeout,
    update_cursor, upsert_thread,
};
use rusqlite::Connection;
use tokio::time::{Instant, advance};

struct GateSender {
    active: AtomicUsize,
    maximum: AtomicUsize,
    starts: AtomicUsize,
    cancellations: AtomicUsize,
    a_calls: AtomicUsize,
    entered: Notify,
    cancelled: Notify,
    release_first: Notify,
    block_all: bool,
    order: Mutex<Vec<(u64, String)>>,
}

impl GateSender {
    fn new(block_all: bool) -> Self {
        Self {
            active: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
            starts: AtomicUsize::new(0),
            cancellations: AtomicUsize::new(0),
            a_calls: AtomicUsize::new(0),
            entered: Notify::new(),
            cancelled: Notify::new(),
            release_first: Notify::new(),
            block_all,
            order: Mutex::new(Vec::new()),
        }
    }
}

struct Active<'a> {
    sender: &'a GateSender,
    completed: bool,
}

impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.sender.active.fetch_sub(1, Ordering::SeqCst);
        if !self.completed {
            self.sender.cancellations.fetch_add(1, Ordering::SeqCst);
            self.sender.cancelled.notify_one();
        }
    }
}

impl SessionMirrorSender for GateSender {
    fn send<'a>(
        &'a self,
        channel: u64,
        _identity: &'a SessionMirrorDeliveryIdentity,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            let mut guard = Active {
                sender: self,
                completed: false,
            };
            self.starts.fetch_add(1, Ordering::SeqCst);
            self.order.lock().unwrap().push((channel, text.to_owned()));
            let first_a = channel == 201 && self.a_calls.fetch_add(1, Ordering::SeqCst) == 0;
            self.entered.notify_one();
            if self.block_all {
                std::future::pending::<()>().await;
            } else if first_a {
                self.release_first.notified().await;
            }
            guard.completed = true;
            Ok(())
        })
    }
}

#[tokio::test(start_paused = true)]
async fn background_target_deadline_drops_only_its_owned_future() {
    let fixture = Fixture::new(false);
    let sender = Arc::new(GateSender::new(true));
    let started = Instant::now();
    let running = fixture.run(sender.clone());
    timeout(Duration::from_secs(1), sender.entered.notified())
        .await
        .unwrap();
    timeout(Duration::from_secs(11), sender.cancelled.notified())
        .await
        .expect("a target must release its in-flight slot at the 10s cooperative deadline");
    assert_eq!(started.elapsed(), Duration::from_secs(10));
    assert_eq!(sender.cancellations.load(Ordering::SeqCst), 1);
    assert_eq!(sender.active.load(Ordering::SeqCst), 0);
    assert_eq!(
        get_offset(&fixture.mirror_db, "thread-a")
            .unwrap()
            .unwrap()
            .cursor,
        0
    );
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn background_has_eight_slots_even_with_more_than_128_targets_and_shutdown_drops_them() {
    let fixture = Fixture::new(true);
    let connection = Connection::open(&fixture.state_db).unwrap();
    for index in 0..128_i64 {
        let id = format!("busy-{index:03}");
        let rollout = fixture.temp.path().join(format!("{id}.jsonl"));
        fs::write(&rollout, event(&id)).unwrap();
        connection.execute(
            "INSERT INTO threads VALUES (?1,?1,'C:/repo',?2,?3,'gpt','high',0,0,0,'vscode','user')",
            params![id, index + 3, rollout.to_string_lossy().as_ref()],
        ).unwrap();
        upsert_thread(
            &fixture.mirror_db,
            &id,
            "project",
            &id,
            100,
            300 + index,
            3.0,
        )
        .unwrap();
        update_cursor(
            &fixture.mirror_db,
            &id,
            rollout.to_string_lossy().as_ref(),
            0,
            1.0,
        )
        .unwrap();
    }
    drop(connection);
    let sender = Arc::new(GateSender::new(true));
    let running = fixture.run(sender.clone());
    timeout(Duration::from_secs(2), async {
        while sender.starts.load(Ordering::SeqCst) < 8 {
            sender.entered.notified().await;
        }
    })
    .await
    .expect("independent targets must occupy the bounded concurrent slots");
    advance(Duration::from_secs(2)).await;
    assert_eq!(sender.starts.load(Ordering::SeqCst), 8);
    assert_eq!(sender.maximum.load(Ordering::SeqCst), 8);
    assert_eq!(sender.active.load(Ordering::SeqCst), 8);
    running.stop().await;
    assert_eq!(
        sender.active.load(Ordering::SeqCst),
        0,
        "no detached dispatch survives shutdown"
    );
    assert_eq!(sender.cancellations.load(Ordering::SeqCst), 8);
}

#[tokio::test(start_paused = true)]
async fn background_preserves_same_target_order_during_repeated_discovery() {
    let fixture = Fixture::new(true);
    let a_rollout = fixture.temp.path().join("a.jsonl");
    fs::write(&a_rollout, event("A first") + &event("A second")).unwrap();
    let sender = Arc::new(GateSender::new(false));
    let running = fixture.run(sender.clone());
    timeout(HEALTHY_B_BUDGET, fixture.b_committed())
        .await
        .unwrap();
    advance(Duration::from_secs(3)).await;
    assert_eq!(
        sender.a_calls.load(Ordering::SeqCst),
        1,
        "A must not have a second in-flight poll"
    );
    sender.release_first.notify_one();
    let expected = i64::try_from(fs::metadata(&a_rollout).unwrap().len()).unwrap();
    timeout(HEALTHY_B_BUDGET, async {
        while get_offset(&fixture.mirror_db, "thread-a")
            .unwrap()
            .unwrap()
            .cursor
            != expected
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    running.stop().await;
    let order = sender.order.lock().unwrap();
    let a = order
        .iter()
        .filter(|(channel, _)| *channel == 201)
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>();
    assert_eq!(a, ["In progress\n\nA first", "In progress\n\nA second"]);
    assert_eq!(sender.cancellations.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn background_discovery_preserves_waiting_targets_across_full_slot_epochs() {
    let fixture = Fixture::new(true);
    let connection = Connection::open(&fixture.state_db).unwrap();
    for index in 0..18_i64 {
        let id = format!("blocked-{index:02}");
        let rollout = fixture.temp.path().join(format!("{id}.jsonl"));
        fs::write(&rollout, event(&id)).unwrap();
        connection.execute(
            "INSERT INTO threads VALUES (?1,?1,'C:/repo',?2,?3,'gpt','high',0,0,0,'vscode','user')",
            params![id, index + 3, rollout.to_string_lossy().as_ref()],
        ).unwrap();
        upsert_thread(
            &fixture.mirror_db,
            &id,
            "project",
            &id,
            100,
            300 + index,
            3.0,
        )
        .unwrap();
        update_cursor(
            &fixture.mirror_db,
            &id,
            rollout.to_string_lossy().as_ref(),
            0,
            1.0,
        )
        .unwrap();
    }
    drop(connection);
    let sender = Arc::new(GateSender::new(true));
    let running = fixture.run(sender.clone());
    timeout(Duration::from_secs(25), async {
        while !sender.order.lock().unwrap().iter().any(|(channel, _)| *channel == 202) {
            sender.entered.notified().await;
        }
    }).await.expect("the last target must reach a slot in the third 10s epoch, not starve behind refreshed old targets");
    running.stop().await;
    assert!(sender.maximum.load(Ordering::SeqCst) <= 8);
    assert_eq!(sender.active.load(Ordering::SeqCst), 0);
}
