//! Revision 14: delayed Goal FIFO and partial-history final-answer precedence.
use super::*;
use cdr_store::observed_final_answer;

async fn receive(
    receiver: &mut tokio::sync::broadcast::Receiver<ResidentNotificationEvent>,
    method: &str,
) -> ResidentNotificationEvent {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let event = receiver.recv().await.unwrap();
            if let ResidentNotificationEvent::Notification { notification, .. } = &event
                && notification.method == method
            {
                return event;
            }
        }
    })
    .await
    .unwrap()
}

fn final_count(messages: &[Value], text: &str) -> usize {
    messages
        .iter()
        .filter(|message| {
            message["content"]
                .as_str()
                .is_some_and(|s| s.starts_with("Final") && s.contains(text))
        })
        .count()
}

async fn automatic_turn(
    f: &Fixture,
    receiver: &mut tokio::sync::broadcast::Receiver<ResidentNotificationEvent>,
    text: &str,
) -> (String, Vec<ResidentNotificationEvent>) {
    let turn = f.rpc("test/goal-turn", json!({})).await["turn"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let started = receive(receiver, "turn/started").await;
    f.rpc(
        "test/complete",
        json!({"turnId":turn,"status":"completed","text":text}),
    )
    .await;
    let item = receive(receiver, "item/completed").await;
    let terminal = receive(receiver, "turn/completed").await;
    (turn, vec![started, item, terminal])
}

#[tokio::test]
async fn revision14_backlogged_goal_fifo_delivers_only_the_last_turn_once() {
    for (new_runtime, legacy) in [(false, false), (true, true)] {
        let mut f = Fixture::new().await;
        completed_goal(&f).await;
        f.worker().recover().await.unwrap();
        make_legacy(&f, legacy);
        let original_owner = original(&f);
        f.restart(new_runtime).await;
        f.configure(json!({"ordinary":true,"goal":"active","emit_final_answer":true}))
            .await;
        let mut receiver = f.server.subscribe_notifications();
        let (second, second_events) = automatic_turn(&f, &mut receiver, "middle answer").await;
        let (third, third_events) = automatic_turn(&f, &mut receiver, "last answer").await;
        f.configure(json!({"goal":"complete"})).await;
        let worker = f.worker();
        // The observer sees all events before the FIFO processor gets T2's start.
        for event in second_events.iter().chain(&third_events) {
            worker.observe_terminal(event).unwrap();
        }
        for event in second_events {
            worker.handle(event).await.unwrap();
        }
        let jobs = queue::list(&f.db).unwrap();
        assert_eq!(
            jobs.len(),
            1,
            "R13-1: middle turn must not delete original job"
        );
        assert_eq!(jobs[0].turn_id.as_deref(), Some(second.as_str()));
        assert!(
            jobs[0].goal_waiting,
            "exact next start must still be attachable"
        );
        assert_execution_preserved(&original_owner, &jobs[0]);
        let terminal = third_events.last().unwrap().clone();
        for event in third_events {
            worker.handle(event).await.unwrap();
        }
        worker.handle(terminal).await.unwrap();
        worker.recover().await.unwrap();
        worker.recover_observed().await.unwrap();
        assert!(queue::list(&f.db).unwrap().is_empty());
        assert!(delivery::list_pending(&f.db).unwrap().is_empty());
        assert!(!observed_completion::contains(&f.db, "thread-b", &third).unwrap());
        assert_eq!(f.count("turn/start"), 1);
        assert_eq!(f.count("thread/settings/update"), 0);
        let messages = f.close().await;
        assert_eq!(final_count(&messages, "middle answer"), 0);
        assert_eq!(final_count(&messages, "last answer"), 1);
        assert_eq!(count_text(&messages, "[Goal progress]\nmiddle answer"), 1);
    }
}

#[tokio::test]
async fn revision14_recovery_hands_off_middle_turn_without_selecting_history_latest() {
    let f = Fixture::new().await;
    completed_goal(&f).await;
    f.worker().recover().await.unwrap();
    f.configure(json!({"emit_final_answer":true})).await;
    let mut receiver = f.server.subscribe_notifications();
    let (second, second_events) = automatic_turn(&f, &mut receiver, "recovered middle").await;
    let worker = f.worker();
    worker.handle(second_events[0].clone()).await.unwrap();
    let before = original(&f);
    let (third, third_events) = automatic_turn(&f, &mut receiver, "recovered final").await;
    f.configure(json!({"goal":"complete"})).await;
    worker.recover().await.unwrap();
    let jobs = queue::list(&f.db).unwrap();
    assert_eq!(
        jobs.len(),
        1,
        "R13-1: recovery must retain the original owner"
    );
    assert_eq!(jobs[0].turn_id.as_deref(), Some(second.as_str()));
    assert!(jobs[0].goal_waiting);
    assert_execution_preserved(&before, &jobs[0]);
    // Without a start observation the worker must not auto-pick T3 from history.
    assert!(worker.recover().await.is_err());
    assert_eq!(original(&f).turn_id.as_deref(), Some(second.as_str()));
    for event in third_events {
        worker.handle(event).await.unwrap();
    }
    assert!(queue::list(&f.db).unwrap().is_empty());
    assert!(!observed_completion::contains(&f.db, "thread-b", &third).unwrap());
    assert_eq!(f.count("turn/start"), 1);
    let messages = f.close().await;
    assert_eq!(final_count(&messages, "recovered middle"), 0);
    assert_eq!(final_count(&messages, "recovered final"), 1);
}

async fn partial_history_case(items: Value, goal: &str) {
    let mut f = Fixture::new().await;
    completed_goal(&f).await;
    f.worker().recover().await.unwrap();
    f.restart(false).await;
    f.configure(json!({"ordinary":true,"goal":"active","emit_final_answer":true}))
        .await;
    let mut receiver = f.server.subscribe_notifications();
    let turn = f.rpc("test/goal-turn", json!({})).await["turn"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let worker = f.worker();
    worker
        .handle(receive(&mut receiver, "turn/started").await)
        .await
        .unwrap();
    f.rpc(
        "test/complete",
        json!({"turnId":turn,"status":"completed","text":"durable exact answer"}),
    )
    .await;
    let item = receive(&mut receiver, "item/completed").await;
    let terminal = receive(&mut receiver, "turn/completed").await;
    worker.observe_terminal(&item).unwrap();
    worker.observe_terminal(&terminal).unwrap();
    let evidence = original(&f).completion_evidence_generation();
    assert_eq!(
        observed_final_answer::get(&f.db, "thread-b", &turn, evidence).unwrap(),
        Some("durable exact answer".into())
    );
    drop(worker);
    f.restart(true).await;
    f.configure(
        json!({"ordinary":true,"goal":goal,"history_items_turn":turn,"history_items":items}),
    )
    .await;
    let worker = f.worker();
    worker.recover_observed().await.unwrap();
    worker.recover_observed().await.unwrap();
    if goal == "complete" {
        assert!(queue::list(&f.db).unwrap().is_empty());
    } else {
        assert!(original(&f).goal_waiting);
    }
    assert_eq!(
        observed_final_answer::get(&f.db, "thread-b", &turn, evidence).unwrap(),
        None
    );
    assert_eq!(f.count("turn/start"), 1);
    let messages = f.close().await;
    if goal == "complete" {
        assert_eq!(
            final_count(&messages, "durable exact answer"),
            1,
            "R13-2: partial history displaced exact journal"
        );
    } else {
        assert_eq!(
            count_text(&messages, "[Goal progress]\ndurable exact answer"),
            1,
            "R13-2: progress consumed the wrong text"
        );
    }
    assert!(messages.iter().all(|m| {
        !m["content"]
            .as_str()
            .is_some_and(|s| s.contains("stale commentary") || s.contains("no visible reply"))
    }));
}

#[tokio::test]
async fn revision14_empty_history_uses_exact_journal_after_resident_restart() {
    for goal in ["complete", "active"] {
        partial_history_case(json!([]), goal).await;
    }
}

#[tokio::test]
async fn revision14_commentary_history_uses_exact_journal_after_resident_restart() {
    for goal in ["complete", "active"] {
        partial_history_case(
            json!([{"type":"agentMessage","phase":"commentary","text":"stale commentary"}]),
            goal,
        )
        .await;
    }
}

#[tokio::test]
async fn revision14_real_fifo_handles_backlog_from_the_original_turn() {
    let f = Fixture::new().await;
    f.configure(json!({"ordinary":true,"goal":"active","emit_final_answer":true}))
        .await;
    let first = f.submit("goal-original").await.turn_id.unwrap();
    let mut receiver = f.server.subscribe_notifications();
    f.rpc(
        "test/complete",
        json!({"turnId":first,"status":"completed","text":"original middle"}),
    )
    .await;
    let mut events = vec![
        receive(&mut receiver, "item/completed").await,
        receive(&mut receiver, "turn/completed").await,
    ];
    let (_, next) = automatic_turn(&f, &mut receiver, "FIFO last answer").await;
    events.extend(next);
    f.configure(json!({"goal":"complete"})).await;
    let worker = f.worker();
    let (sender, pending) = tokio::sync::mpsc::channel(16);
    for event in events {
        worker.observe_terminal(&event).unwrap();
        sender.send(event).await.unwrap();
    }
    drop(sender);
    // Exercise the actual production processor, including its initial recovery.
    driver::process(&worker, pending).await;
    worker.recover_observed().await.unwrap();
    assert!(queue::list(&f.db).unwrap().is_empty());
    assert!(delivery::list_pending(&f.db).unwrap().is_empty());
    assert_eq!(f.count("turn/start"), 1);
    let messages = f.close().await;
    assert_eq!(final_count(&messages, "original middle"), 0);
    assert_eq!(final_count(&messages, "FIFO last answer"), 1);
}

#[tokio::test]
async fn revision14_attached_owner_change_during_history_read_preserves_journal() {
    let f = Fixture::new().await;
    completed_goal(&f).await;
    f.worker().recover().await.unwrap();
    f.configure(json!({"emit_final_answer":true})).await;
    let mut receiver = f.server.subscribe_notifications();
    let (turn, events) = automatic_turn(&f, &mut receiver, "retained exact answer").await;
    let worker = f.worker();
    worker.handle(events[0].clone()).await.unwrap();
    worker.handle(events[1].clone()).await.unwrap();
    let evidence = original(&f).completion_evidence_generation();
    f.configure(json!({"goal":"complete","gate":"thread/read"}))
        .await;
    let terminal = events[2].clone();
    let task = tokio::spawn(async move { worker.handle(terminal).await });
    let root = f.log.parent().unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        while !root.join("gate-ready").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    rusqlite::Connection::open(&f.db)
        .unwrap()
        .execute(
            "UPDATE codex_turn_queue SET owner_user_id=77 WHERE job_id='goal-original'",
            [],
        )
        .unwrap();
    let changed = original(&f);
    std::fs::write(root.join("gate-release"), "release").unwrap();
    assert!(
        task.await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("ownership changed")
    );
    assert_eq!(queue::list(&f.db).unwrap(), [changed]);
    assert_eq!(
        observed_final_answer::get(&f.db, "thread-b", &turn, evidence).unwrap(),
        Some("retained exact answer".into())
    );
    assert!(delivery::list_pending(&f.db).unwrap().is_empty());
    assert_eq!(final_count(&f.close().await, "retained exact answer"), 0);
}

#[tokio::test]
async fn revision14_explicit_history_final_remains_stronger_than_journal() {
    let f = Fixture::new().await;
    let turn = completed_goal(&f).await;
    assert!(
        observed_final_answer::record(
            &f.db,
            "thread-b",
            &turn,
            original(&f).completion_evidence_generation(),
            "journal text"
        )
        .unwrap()
    );
    f.configure(
        json!({"goal":"complete","history_items_turn":turn,"history_items":[
            {"type":"agentMessage","phase":"final_answer","text":"explicit history final"},
            {"type":"agentMessage","phase":"commentary","text":"later commentary"}
        ]}),
    )
    .await;
    f.worker().recover().await.unwrap();
    assert!(queue::list(&f.db).unwrap().is_empty());
    let messages = f.close().await;
    assert_eq!(final_count(&messages, "explicit history final"), 1);
    assert_eq!(final_count(&messages, "journal text"), 0);
    assert_eq!(final_count(&messages, "later commentary"), 0);
}
