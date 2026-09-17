//! Revision 13 guard boundaries: safe no-successor finish, lost observation, owner races.
use super::*;

fn final_count(messages: &[Value]) -> usize {
    messages
        .iter()
        .filter(|m| {
            m["content"]
                .as_str()
                .is_some_and(|s| s.starts_with("Final"))
        })
        .count()
}

#[tokio::test]
async fn goal_completion_without_a_successor_still_finishes_once() {
    let mut f = Fixture::new().await;
    completed_goal(&f).await;
    f.worker().recover().await.unwrap();
    f.restart(false).await;
    f.configure(json!({"ordinary":true,"goal":"complete"}))
        .await;
    f.worker().recover().await.unwrap();
    f.worker().recover().await.unwrap();
    assert!(queue::list(&f.db).unwrap().is_empty());
    assert_eq!(f.count("turn/start"), 1);
    assert_eq!(final_count(&f.close().await), 1);
}

#[tokio::test]
async fn missing_start_observation_retains_the_original_across_worker_and_resident_restart() {
    let mut f = Fixture::new().await;
    completed_goal(&f).await;
    f.worker().recover().await.unwrap();
    let before = original(&f);
    f.restart(false).await;
    f.configure(json!({"ordinary":true,"goal":"active"})).await;
    for text in ["unowned next", "unowned later"] {
        let next = f.rpc("test/goal-turn", json!({})).await["turn"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        f.rpc(
            "test/complete",
            json!({"turnId":next,"status":"completed","text":text}),
        )
        .await;
    }
    f.configure(json!({"goal":"complete"})).await;
    for _ in 0..2 {
        let error = f.worker().recover().await.unwrap_err();
        assert!(error.to_string().contains("unattached turn"));
        assert_eq!(
            queue::list(&f.db).unwrap().as_slice(),
            std::slice::from_ref(&before)
        );
    }
    f.restart(true).await;
    f.configure(json!({"ordinary":true,"goal":"complete"}))
        .await;
    assert!(f.worker().recover().await.is_err());
    assert_eq!(queue::list(&f.db).unwrap(), [before]);
    assert!(delivery::list_pending(&f.db).unwrap().is_empty());
    assert_eq!(f.count("turn/start"), 1);
    assert_eq!(f.count("thread/settings/update"), 0);
    assert_eq!(final_count(&f.close().await), 0);
}

#[tokio::test]
async fn captured_waiting_owner_is_not_replaced_during_finish_or_staging() {
    let f = Fixture::new().await;
    let turn = completed_goal(&f).await;
    f.worker().recover().await.unwrap();
    f.configure(json!({"goal":"complete"})).await;
    let captured = original(&f);
    assert!(
        observed_completion::record(
            &f.db,
            "thread-b",
            &turn,
            captured.completion_evidence_generation(),
            "{}"
        )
        .unwrap()
    );
    rusqlite::Connection::open(&f.db)
        .unwrap()
        .execute(
            "UPDATE codex_turn_queue SET owner_user_id=99 WHERE job_id='goal-original'",
            [],
        )
        .unwrap();
    let changed = original(&f);
    let worker = f.worker();
    let error = worker
        .finish_waiting_goal_owned(f.server.generation(), &captured)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("ownership changed"));
    let error = f
        .queue
        .stage_owned_turn_completion(&captured, "wrong final", false)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("ownership changed"));
    assert_eq!(queue::list(&f.db).unwrap(), [changed]);
    assert!(observed_completion::contains(&f.db, "thread-b", &turn).unwrap());
    assert!(delivery::list_pending(&f.db).unwrap().is_empty());
    assert_eq!(final_count(&f.close().await), 0);
}

#[tokio::test]
async fn waiting_owner_changed_while_history_is_awaited_is_not_finalized() {
    let f = Fixture::new().await;
    completed_goal(&f).await;
    f.worker().recover().await.unwrap();
    f.configure(json!({"goal":"complete","gate":"thread/read"}))
        .await;
    let worker = f.worker();
    let generation = f.server.generation();
    let task =
        tokio::spawn(async move { worker.finish_waiting_goal(generation, "thread-b").await });
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
    let error = task.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("ownership changed"));
    assert_eq!(queue::list(&f.db).unwrap(), [changed]);
    assert!(delivery::list_pending(&f.db).unwrap().is_empty());
    assert_eq!(final_count(&f.close().await), 0);
}

#[tokio::test]
async fn previously_handed_off_goal_turns_do_not_prevent_a_legitimate_final() {
    let mut f = Fixture::new().await;
    completed_goal(&f).await;
    f.worker().recover().await.unwrap();
    f.restart(false).await;
    f.configure(json!({"ordinary":true,"goal":"active"})).await;
    let next = f.rpc("test/goal-turn", json!({})).await["turn"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.worker().recover().await.unwrap();
    f.rpc(
        "test/complete",
        json!({"turnId":next,"status":"completed","text":"second progress"}),
    )
    .await;
    f.worker().recover().await.unwrap();
    assert!(original(&f).goal_waiting);
    f.configure(json!({"goal":"complete"})).await;
    f.worker().recover().await.unwrap();
    f.worker().recover().await.unwrap();
    assert!(queue::list(&f.db).unwrap().is_empty());
    assert_eq!(f.count("turn/start"), 1);
    let messages = f.close().await;
    assert_eq!(final_count(&messages), 1);
    assert_eq!(count_text(&messages, "[Goal progress]\nsecond progress"), 1);
}
