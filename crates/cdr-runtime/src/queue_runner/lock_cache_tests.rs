use super::*;

struct NeverBackend;

impl TurnBackend for NeverBackend {
    fn generation(&self) -> u64 {
        1
    }
    fn active_turn_id<'a>(&'a self, _: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { panic!("lock cache never calls backend") })
    }
    fn read_turns<'a>(&'a self, _: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async { panic!("lock cache never calls backend") })
    }
    fn resume_thread<'a>(&'a self, _: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async { panic!("lock cache never calls backend") })
    }
    fn start_turn<'a>(&'a self, _: &'a str, _: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async { panic!("lock cache never calls backend") })
    }
}

#[test]
fn unused_target_locks_do_not_accumulate_for_every_historical_thread() {
    let coordinator = QueueCoordinator::new(PathBuf::from("unused.sqlite"), Arc::new(NeverBackend));
    for index in 0..100 {
        drop(
            coordinator
                .target_lock(&format!("finished-{index}"))
                .unwrap(),
        );
    }
    let _current = coordinator.target_lock("current").unwrap();
    assert_eq!(coordinator.target_locks.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn pruning_never_replaces_a_lock_with_an_active_owner_or_waiter() {
    let coordinator = QueueCoordinator::new(PathBuf::from("unused.sqlite"), Arc::new(NeverBackend));
    let original = coordinator.target_lock("active").unwrap();
    let guard = Arc::clone(&original).lock_owned().await;
    let waiter_lock = coordinator.target_lock("active").unwrap();
    let mut waiter = Box::pin(Arc::clone(&waiter_lock).lock_owned());
    assert!(futures_util::poll!(&mut waiter).is_pending());
    for index in 0..100 {
        drop(coordinator.target_lock(&format!("other-{index}")).unwrap());
    }
    let repeated = coordinator.target_lock("active").unwrap();
    assert!(Arc::ptr_eq(&original, &repeated));
    assert!(Arc::ptr_eq(&waiter_lock, &repeated));
    assert!(repeated.try_lock().is_err());
    drop(guard);
    let waiting_guard = waiter.await;
    assert!(repeated.try_lock().is_err());
    drop(waiting_guard);
    assert!(repeated.try_lock().is_ok());
}
