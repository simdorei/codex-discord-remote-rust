use super::*;

#[tokio::test]
async fn context_timeout_keeps_worker_slot_until_io_really_exits() {
    static SLOT: Semaphore = Semaphore::const_new(1);
    let (send, recv) = std::sync::mpsc::channel();
    let result = bounded_reader(&SLOT, Duration::from_millis(20), move || {
        recv.recv_timeout(Duration::from_secs(2)).unwrap();
        "late snapshot".into()
    })
    .await;
    assert!(result.unwrap_err().contains("timed out"));
    assert!(
        bounded_reader(&SLOT, Duration::from_secs(1), || panic!("extra read"))
            .await
            .unwrap_err()
            .contains("still running")
    );
    send.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while SLOT.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        bounded_reader(&SLOT, Duration::from_secs(1), || "fresh".into())
            .await
            .unwrap(),
        "fresh"
    );
}

#[test]
fn context_limit_explicitly_counts_unread_threads() {
    let root = tempfile::tempdir().unwrap();
    let threads = (0..51)
        .map(|n| ThreadInfo {
            id: format!("thread-{n}"),
            title: format!("title-{n}"),
            rollout_path: root.path().join("missing"),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let text = render_blocking(&threads, false, 10, RecentTextMode::Visible);
    assert!(text.contains("미조회 대화: 1"));
    assert!(text.contains("thread-49"));
    assert!(!text.contains("thread-50"));
    assert_eq!(text.matches("조회 실패:").count(), 50);
}
