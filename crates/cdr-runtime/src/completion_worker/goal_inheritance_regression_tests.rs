//! Revision 13 regressions: inherited terminal evidence and completed-before-attach.
use super::*;
use cdr_store::observed_final_answer;

async fn receive_method(
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
    .expect("fixture must emit the actual resident notification")
}

fn final_count(messages: &[Value], text: &str) -> usize {
    messages
        .iter()
        .filter(|m| {
            m["content"]
                .as_str()
                .is_some_and(|s| s.starts_with("Final") && s.contains(text))
        })
        .count()
}

#[tokio::test]
async fn inherited_goal_terminal_journals_survive_restart_and_omitted_history() {
    for (new_runtime, legacy) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut f = Fixture::new().await;
        completed_goal(&f).await;
        f.worker().recover().await.unwrap();
        make_legacy(&f, legacy);
        let before = original(&f);
        f.restart(new_runtime).await;
        f.configure(json!({"ordinary":true,"goal":"active"})).await;
        let next = f.rpc("test/goal-turn", json!({})).await["turn"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        f.worker().recover().await.unwrap();
        assert_eq!(original(&f).turn_id.as_deref(), Some(next.as_str()));
        assert_execution_preserved(&before, &original(&f));
        let observed_generation = i64::try_from(f.server.generation()).unwrap();
        let mut receiver = f.server.subscribe_notifications();
        f.configure(json!({"goal":"complete","emit_final_answer":true}))
            .await;
        f.rpc(
            "test/complete",
            json!({"turnId":next,"status":"completed","text":"journal inherited final"}),
        )
        .await;
        let item = receive_method(&mut receiver, "item/completed").await;
        let terminal = receive_method(&mut receiver, "turn/completed").await;
        let worker = f.worker();
        worker.observe_terminal(&item).unwrap();
        worker.observe_terminal(&terminal).unwrap();
        assert_eq!(
            observed_final_answer::get(&f.db, "thread-b", &next, observed_generation).unwrap(),
            Some("journal inherited final".into()),
            "R12-1: inherited exact final was rejected"
        );
        assert!(
            observed_completion::contains(&f.db, "thread-b", &next).unwrap(),
            "R12-1: inherited terminal metadata was rejected"
        );
        drop(worker);
        // Drop all worker state and start a new resident whose numeric generation
        // can be reused. The persisted evidence remains tied to the attached turn.
        f.restart(true).await;
        f.configure(json!({"ordinary":true,"goal":"complete","omit_turn":next}))
            .await;
        let worker = f.worker();
        worker.recover_observed().await.unwrap();
        worker.recover_observed().await.unwrap();
        worker.recover().await.unwrap();
        assert!(queue::list(&f.db).unwrap().is_empty());
        assert!(delivery::list_pending(&f.db).unwrap().is_empty());
        assert!(!observed_completion::contains(&f.db, "thread-b", &next).unwrap());
        assert_eq!(
            observed_final_answer::get(&f.db, "thread-b", &next, observed_generation).unwrap(),
            None
        );
        assert_eq!(f.count("turn/start"), 1);
        assert_eq!(f.count("thread/settings/update"), 0);
        let messages = f.close().await;
        assert_eq!(final_count(&messages, "journal inherited final"), 1);
        assert_eq!(
            count_text(&messages, "[Goal progress]\ninherited progress"),
            1
        );
    }
}

#[tokio::test]
async fn completed_unattached_goal_holds_prior_progress_then_exact_start_recovers_final_once() {
    let mut f = Fixture::new().await;
    let first = completed_goal(&f).await;
    f.worker().recover().await.unwrap();
    let waiting = original(&f);
    f.restart(false).await;
    f.configure(json!({"ordinary":true,"goal":"active"})).await;
    let mut receiver = f.server.subscribe_notifications();
    let next = f.rpc("test/goal-turn", json!({})).await["turn"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let started = receive_method(&mut receiver, "turn/started").await;
    f.rpc(
        "test/complete",
        json!({"turnId":next,"status":"completed","text":"fast inherited final"}),
    )
    .await;
    let terminal = receive_method(&mut receiver, "turn/completed").await;
    f.configure(json!({"goal":"complete"})).await;
    let worker = f.worker();
    // Production processing runs recovery before draining queued notifications.
    // Neither history's latest turn nor Goal=complete is authority to consume T1.
    let _ = worker.recover().await;
    assert_eq!(
        queue::list(&f.db).unwrap().as_slice(),
        std::slice::from_ref(&waiting),
        "R12-2: recovery must hold rather than finalize/delete the prior progress owner"
    );
    assert!(delivery::list_pending(&f.db).unwrap().is_empty());
    assert_eq!(original(&f).turn_id.as_deref(), Some(first.as_str()));
    worker.handle(started).await.unwrap();
    assert_eq!(original(&f).turn_id.as_deref(), Some(next.as_str()));
    assert_execution_preserved(&waiting, &original(&f));
    worker.observe_terminal(&terminal).unwrap();
    worker.handle(terminal).await.unwrap();
    worker.recover().await.unwrap();
    worker.recover_observed().await.unwrap();
    assert!(queue::list(&f.db).unwrap().is_empty());
    assert_eq!(f.count("turn/start"), 1);
    let messages = f.close().await;
    assert_eq!(final_count(&messages, "inherited progress"), 0);
    assert_eq!(final_count(&messages, "fast inherited final"), 1);
}

#[tokio::test]
async fn duplicate_prior_terminal_cannot_bypass_the_waiting_goal_guard() {
    let mut f = Fixture::new().await;
    let first = completed_goal(&f).await;
    f.worker().recover().await.unwrap();
    f.restart(false).await;
    f.configure(json!({"ordinary":true,"goal":"active"})).await;
    let next = f.rpc("test/goal-turn", json!({})).await["turn"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.rpc(
        "test/complete",
        json!({"turnId":next,"status":"completed","text":"not yet owned"}),
    )
    .await;
    f.configure(json!({"goal":"complete"})).await;
    let mut receiver = f.server.subscribe_notifications();
    f.rpc(
        "test/complete",
        json!({"turnId":first,"status":"completed","text":"inherited progress"}),
    )
    .await;
    let old_terminal = receive_method(&mut receiver, "turn/completed").await;
    let before = original(&f);
    let worker = f.worker();
    let _ = worker.handle(old_terminal).await;
    assert_eq!(
        queue::list(&f.db).unwrap(),
        [before],
        "duplicate T1 must not become Final"
    );
    assert_eq!(f.count("turn/start"), 1);
    let messages = f.close().await;
    assert_eq!(final_count(&messages, "inherited progress"), 0);
}

#[tokio::test]
async fn deferred_goal_events_are_rejournaled_after_exact_start_attachment() {
    let mut f = Fixture::new().await;
    completed_goal(&f).await;
    f.worker().recover().await.unwrap();
    f.restart(false).await;
    f.configure(json!({"ordinary":true,"goal":"active"})).await;
    let mut receiver = f.server.subscribe_notifications();
    let next = f.rpc("test/goal-turn", json!({})).await["turn"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let started = receive_method(&mut receiver, "turn/started").await;
    f.configure(json!({"goal":"complete","emit_final_answer":true}))
        .await;
    f.rpc(
        "test/complete",
        json!({"turnId":next,"status":"completed","text":"deferred exact final"}),
    )
    .await;
    let item = receive_method(&mut receiver, "item/completed").await;
    let terminal = receive_method(&mut receiver, "turn/completed").await;
    let worker = f.worker();
    // Match the real driver: its observer can run ahead of the FIFO processor.
    worker.observe_terminal(&item).unwrap();
    worker.observe_terminal(&terminal).unwrap();
    let generation = i64::try_from(f.server.generation()).unwrap();
    assert_eq!(
        observed_final_answer::get(&f.db, "thread-b", &next, generation).unwrap(),
        None
    );
    assert!(!observed_completion::contains(&f.db, "thread-b", &next).unwrap());
    assert!(worker.recover().await.is_err());
    worker.handle(started).await.unwrap();
    f.configure(json!({"omit_turn":next})).await;
    // Processing may now journal this exact owned item. This is a local INSERT
    // OR IGNORE, never a resend of the input or an external request retry.
    worker.handle(item).await.unwrap();
    assert_eq!(
        observed_final_answer::get(&f.db, "thread-b", &next, generation).unwrap(),
        Some("deferred exact final".into()),
        "deferred owned item must be re-observed after attachment"
    );
    worker.handle(terminal.clone()).await.unwrap();
    worker.handle(terminal).await.unwrap();
    worker.recover_observed().await.unwrap();
    assert!(queue::list(&f.db).unwrap().is_empty());
    assert_eq!(f.count("turn/start"), 1);
    let messages = f.close().await;
    assert_eq!(final_count(&messages, "deferred exact final"), 1);
    assert_eq!(final_count(&messages, "inherited progress"), 0);
}
