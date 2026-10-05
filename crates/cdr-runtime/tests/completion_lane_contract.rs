use cdr_app_server::{Notification, ResidentNotificationEvent, requests::AppRequest};
use cdr_store::{delivery, delivery_receipt, queue};
use serde_json::json;
use std::time::Duration;
use tokio::sync::{broadcast, watch};

#[path = "support/completion_lane_fixture.rs"]
mod fixture;
#[path = "support/completion_lane_http.rs"]
mod http;
use fixture::{Rig, wait_for};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn incremental_cold_recovery_preserves_uncertain_starting_across_generations() {
    let temp = tempfile::tempdir().unwrap();
    let remote = http::start([], []).await;
    let rig = Rig::new(&temp, &remote.address, "action").await;
    for (target, generation, error) in
        [("cold-current", 1, ""), ("cold-old", 37, "kept diagnostic")]
    {
        queue::enqueue(
            rig.queue.db_path(),
            queue::NewQueueJob {
                job_id: target,
                target_thread_id: target,
                channel_id: 44,
                owner_user_id: Some(1),
                discord_message_id: None,
                app_server_generation: generation,
                prompt: "never replay this input",
                queued: false,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        queue::begin_attempt(
            rig.queue.db_path(),
            target,
            &["known-turn".into()],
            generation,
        )
        .unwrap();
        rusqlite::Connection::open(rig.queue.db_path())
            .unwrap()
            .execute(
                "UPDATE codex_turn_queue SET updated_at=1,last_error=? WHERE job_id=?",
                rusqlite::params![error, target],
            )
            .unwrap();
    }
    rig.server.close().await.unwrap();
    for restart in 0..2 {
        let cold = Rig::new(&temp, &remote.address, "action").await;
        let (events, receiver) = broadcast::channel(512);
        let running = cold.run(receiver);
        let b = format!("new-b-{restart}");
        cold.seed(&b, 43, &b, &b);
        events.send(cold.failed(&b, &b)).unwrap();
        wait_for(|| {
            cold.stored(&b, &b)
                && cold.delivered(&b)
                && !queue::list_filtered(cold.queue.db_path(), Some("cold-current"), None).unwrap()
                    [0]
                .last_error
                .is_empty()
        })
        .await;
        for (target, generation) in [("cold-current", 1), ("cold-old", 37)] {
            let jobs = queue::list_filtered(cold.queue.db_path(), Some(target), None).unwrap();
            assert_eq!(jobs.len(), 1);
            let job = &jobs[0];
            assert_eq!(job.state, queue::QueueJobState::Starting);
            assert_eq!(job.app_server_generation, generation);
            assert_eq!(job.attempt_count, 1);
            assert_eq!(job.baseline_turn_ids, ["known-turn"]);
            assert_eq!(job.prompt, "never replay this input");
            assert!(job.turn_id.is_none());
            if target == "cold-old" {
                assert_eq!(job.last_error, "kept diagnostic");
            }
        }
        running.stop().await;
        cold.server.close().await.unwrap();
    }
    fixture::assert_no_execution_requests(&temp.path().join("rpc.jsonl"));
    assert_eq!(
        remote.posts.lock().unwrap().len(),
        2,
        "only new B failures were delivered"
    );
    remote.close().await;
}

fn count(remote: &http::Server, channel: u64) -> usize {
    remote
        .posts
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.channel == channel)
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_http_slots_stalled_still_allow_b_local_completion_and_later_delivery() {
    let temp = tempfile::tempdir().unwrap();
    let remote = http::start(42..46, []).await;
    let rig = Rig::new(&temp, &remote.address, "action").await;
    let (events, receiver) = broadcast::channel(512);
    let running = rig.run(receiver);
    for channel in 42..46 {
        let id = format!("a-{channel}");
        rig.seed(&id, channel, &id, &id);
        events.send(rig.failed(&id, &id)).unwrap();
    }
    wait_for(|| remote.posts.lock().unwrap().len() == 4).await;
    rig.seed("b", 46, "b", "turn-b");
    events.send(rig.failed("b", "turn-b")).unwrap();
    wait_for(|| rig.stored("b", "turn-b")).await;
    assert!(
        queue::list_filtered(rig.queue.db_path(), Some("b"), None)
            .unwrap()
            .is_empty()
    );
    assert!(
        !rig.delivered("b"),
        "the global stall does not pretend HTTP success"
    );
    assert_eq!(
        remote.posts.lock().unwrap().len(),
        4,
        "the four-slot HTTP cap is saturated"
    );
    remote.release();
    wait_for(|| {
        delivery::list_pending(rig.queue.db_path())
            .unwrap()
            .is_empty()
    })
    .await;
    assert_eq!(remote.posts.lock().unwrap().len(), 5);
    assert_eq!(count(&remote, 46), 1);
    running.stop().await;
    rig.server.close().await.unwrap();
    fixture::assert_no_execution_requests(&temp.path().join("rpc.jsonl"));
    remote.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_over_128_heads_and_continuous_a_do_not_starve_b() {
    let temp = tempfile::tempdir().unwrap();
    let remote = http::start([50], []).await;
    let rig = Rig::new(&temp, &remote.address, "action").await;
    for n in 0..140 {
        rig.stage_unknown(&format!("old-{n:03}"), 200 + n);
    }
    for n in 0..20 {
        rig.stage_unknown(&format!("a-backlog-{n:03}"), 42);
    }
    rig.seed("live-a", 50, "live-a", "turn-a");
    let (events, receiver) = broadcast::channel(512);
    let running = rig.run(receiver);
    events.send(rig.failed("live-a", "turn-a")).unwrap();
    wait_for(|| count(&remote, 50) == 1).await;
    let event = ResidentNotificationEvent::Notification {
        generation: rig.server.generation(),
        notification: Notification {
            method: "item/delta".into(),
            params: json!({"threadId":"live-a","text":"tick"}),
        },
    };
    for _ in 0..64 {
        events.send(event.clone()).unwrap();
    }
    let flooding = events.clone();
    let (stop_flood, mut stopped) = watch::channel(false);
    let flood = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        loop {
            tokio::select! {
                _=stopped.changed()=>return,
                _=tick.tick()=>{let _=flooding.send(event.clone());},
            }
        }
    });
    rig.seed("b", 43, "b", "turn-b");
    events.send(rig.failed("b", "turn-b")).unwrap();
    wait_for(|| rig.stored("b", "turn-b") && rig.delivered("b") && count(&remote, 43) == 1).await;
    stop_flood.send(true).unwrap();
    flood.await.unwrap();
    assert_eq!(
        remote.posts.lock().unwrap().len(),
        2,
        "old unknown receipts are never resent"
    );
    assert_eq!(
        delivery::list_pending(rig.queue.db_path()).unwrap().len(),
        161
    );
    running.stop().await;
    assert_eq!(
        delivery_receipt::unknown_count(rig.queue.db_path()).unwrap(),
        161
    );
    rig.server.close().await.unwrap();
    fixture::assert_no_execution_requests(&temp.path().join("rpc.jsonl"));
    remote.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lost_and_confirmed_receipts_survive_cold_reconstruction_without_repost() {
    let temp = tempfile::tempdir().unwrap();
    let remote = http::start([], [42]).await;
    let rig = Rig::new(&temp, &remote.address, "action").await;
    rig.seed("a", 42, "lost", "turn-a");
    rig.seed("confirmed", 44, "confirmed", "turn-c");
    rusqlite::Connection::open(rig.queue.db_path()).unwrap().execute_batch(
        "CREATE TRIGGER keep_confirmed BEFORE DELETE ON codex_delivery_outbox
         WHEN OLD.job_id='confirmed' BEGIN SELECT RAISE(ABORT,'fixture completion DELETE failure'); END;"
    ).unwrap();
    let (events, receiver) = broadcast::channel(512);
    let running = rig.run(receiver);
    events.send(rig.failed("a", "turn-a")).unwrap();
    events.send(rig.failed("confirmed", "turn-c")).unwrap();
    wait_for(|| {
        count(&remote, 42) == 1
            && count(&remote, 44) == 1
            && delivery_receipt::unknown_count(rig.queue.db_path()).unwrap() == 1
    })
    .await;
    assert!(!rig.delivered("lost"));
    assert!(!rig.delivered("confirmed"));
    running.stop().await;
    rig.server.close().await.unwrap();
    rusqlite::Connection::open(rig.queue.db_path())
        .unwrap()
        .execute_batch("DROP TRIGGER keep_confirmed")
        .unwrap();
    for restart in 0..2 {
        let cold = Rig::new(&temp, &remote.address, "action").await;
        let (events, receiver) = broadcast::channel(512);
        let running = cold.run(receiver);
        let b = format!("b-{restart}");
        cold.seed(&b, 43, &b, &b);
        events.send(cold.failed(&b, &b)).unwrap();
        wait_for(|| cold.stored(&b, &b) && cold.delivered(&b) && cold.delivered("confirmed")).await;
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert_eq!(
            count(&remote, 42),
            1,
            "lost acceptance must not be reposted"
        );
        assert_eq!(
            count(&remote, 44),
            1,
            "confirmed receipt finishes DELETE without repost"
        );
        assert_eq!(
            delivery::list_pending(cold.queue.db_path()).unwrap().len(),
            1
        );
        assert_eq!(
            delivery_receipt::unknown_count(cold.queue.db_path()).unwrap(),
            1
        );
        running.stop().await;
        cold.server.close().await.unwrap();
    }
    fixture::assert_no_execution_requests(&temp.path().join("rpc.jsonl"));
    remote.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_goal_successor_commits_final_behind_held_progress_while_b_delivers() {
    let temp = tempfile::tempdir().unwrap();
    let remote = http::start([42], []).await;
    let rig = Rig::new(&temp, &remote.address, "goal").await;
    rig.seed("thread", 42, "goal-owner", "T1");
    rig.journal("thread", "T1", "completed");
    let running = rig.run(rig.server.subscribe_notifications());
    wait_for(|| count(&remote, 42) == 1).await;
    rig.seed("b", 43, "b", "turn-b");
    rig.journal("b", "turn-b", "failed");
    wait_for(|| rig.stored("b", "turn-b") && rig.delivered("b")).await;
    rig.server
        .execute(
            AppRequest {
                method: "test/advance-goal",
                params: json!({}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
    wait_for(|| rig.stored("thread", "T2")).await;
    let saved = delivery::list_pending(rig.queue.db_path()).unwrap();
    assert_eq!(
        saved
            .iter()
            .find(|p| p.job_id == "goal-owner")
            .unwrap()
            .content,
        "Final\ngoal final"
    );
    assert!(
        queue::list_filtered(rig.queue.db_path(), Some("thread"), None)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        count(&remote, 42),
        1,
        "same-channel Final cannot pass the progress HTTP"
    );
    remote.release();
    wait_for(|| rig.delivered("goal-owner")).await;
    let posts = remote.posts.lock().unwrap().clone();
    let a = posts
        .iter()
        .filter(|p| p.channel == 42)
        .map(|p| p.content.as_str())
        .collect::<Vec<_>>();
    assert_eq!(a, ["[Goal progress]\nprogress", "Final\ngoal final"]);
    assert_eq!(count(&remote, 43), 1);
    running.stop().await;
    rig.server.close().await.unwrap();
    let rpc = std::fs::read_to_string(temp.path().join("goal-rpc.log")).unwrap();
    assert!(
        !rpc.lines()
            .any(|m| matches!(m, "turn/start" | "turn/steer" | "thread/fork"))
    );
    remote.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn more_than_128_eligible_confirmed_heads_do_not_delay_a_new_b() {
    let temp = tempfile::tempdir().unwrap();
    let remote = http::start([], []).await;
    let rig = Rig::new(&temp, &remote.address, "action").await;
    for n in 0..130 {
        let job = format!("confirmed-{n:03}");
        let channel = 200 + n;
        rig.stage_unknown(&job, channel);
        let key = serde_json::to_string(&(channel, "completion/v1", &job, 0)).unwrap();
        delivery_receipt::confirm(rig.queue.db_path(), &key, &(10_000 + n).to_string()).unwrap();
    }
    let mut cursor = cdr_store::completion_work::Cursor::default();
    let mut eligible = 0;
    while !cursor.finished {
        eligible += cdr_store::completion_work::page(
            rig.queue.db_path(),
            cdr_store::completion_work::Source::Final,
            &mut cursor,
            rig.server.instance_id(),
            i64::try_from(rig.server.generation()).unwrap(),
        )
        .unwrap()
        .entries
        .len();
    }
    assert_eq!(
        eligible, 130,
        "these are eligible candidates, not excluded unknowns"
    );
    let (events, receiver) = broadcast::channel(512);
    let running = rig.run(receiver);
    rig.seed("b", 43, "b", "turn-b");
    let observed_at = std::time::Instant::now();
    events.send(rig.failed("b", "turn-b")).unwrap();
    let mut milestones = [false; 3];
    wait_for(|| {
        let stored = rig.stored("b", "turn-b");
        let states = [
            stored,
            stored && rig.delivered("b"),
            count(&remote, 43) == 1,
        ];
        for (index, label) in ["b_marker", "b_outbox_removed", "b_http_post"]
            .iter()
            .enumerate()
        {
            if states[index] && !milestones[index] {
                eprintln!(
                    "eligible_heads stage={label} elapsed_ms={}",
                    observed_at.elapsed().as_millis()
                );
                milestones[index] = true;
            }
        }
        states.into_iter().all(|ready| ready)
    })
    .await;
    eprintln!(
        "eligible_heads stage=b_complete elapsed_ms={}",
        observed_at.elapsed().as_millis()
    );
    let cleanup_started = std::time::Instant::now();
    let mut last_remaining = i64::MAX;
    wait_for(|| {
        let remaining: i64 = rusqlite::Connection::open_with_flags(
            rig.queue.db_path(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap()
        .query_row("SELECT COUNT(*) FROM codex_delivery_outbox", [], |row| {
            row.get(0)
        })
        .unwrap();
        if remaining != last_remaining {
            eprintln!(
                "eligible_heads stage=old_outbox_cleanup remaining={remaining} elapsed_ms={}",
                cleanup_started.elapsed().as_millis()
            );
            last_remaining = remaining;
        }
        remaining == 0
    })
    .await;
    assert!(
        delivery::list_pending(rig.queue.db_path())
            .unwrap()
            .is_empty(),
        "the production pending API also observes complete cleanup"
    );
    assert_eq!(
        remote.posts.lock().unwrap().len(),
        1,
        "confirmed old messages are not reposted"
    );
    running.stop().await;
    rig.server.close().await.unwrap();
    fixture::assert_no_execution_requests(&temp.path().join("rpc.jsonl"));
    remote.close().await;
}
