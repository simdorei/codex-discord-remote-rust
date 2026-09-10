use tokio::sync::mpsc;

use super::ShardExitGuard;

#[tokio::test]
async fn gse_01_normal_task_exit_publishes_exact_shard() {
    let (sender, mut exits) = mpsc::channel(1);
    drop(ShardExitGuard { shard: 7, sender });

    assert_eq!(exits.recv().await, Some(7));
}

#[tokio::test]
async fn gse_02_panicking_task_publishes_before_join_reports_panic() {
    let (sender, mut exits) = mpsc::channel(1);
    let task = tokio::spawn(async move {
        let _exit = ShardExitGuard { shard: 11, sender };
        panic!("injected shard panic");
    });

    assert_eq!(exits.recv().await, Some(11));
    assert!(task.await.unwrap_err().is_panic());
}
