use super::*;

fn replace_payload(f: &HistoryFixture, payload: &Value) -> ingress::StoredIngress {
    let original = unresolved_archive(f);
    cdr_store::schema::open_initialized(&f.db)
        .unwrap()
        .execute(
            "UPDATE discord_ingress_journal SET payload_json=? WHERE ingress_id=?",
            rusqlite::params![payload.to_string(), original.ingress_id],
        )
        .unwrap();
    ingress::get(&f.db, &original.ingress_id).unwrap().unwrap()
}

async fn rejects_claim(payload: Value) {
    let f = HistoryFixture::new(&completed_history()).await;
    queue::complete(&f.db, "next").unwrap();
    owning_queue(&f).recover_target("thread-b").await.unwrap();
    fixture::pending(&f.db, "next", "thread-b", 1);
    let intent = replace_payload(&f, &payload);
    let jobs = queue::list(&f.db).unwrap();
    let result = queue::try_begin_attempt(&f.db, "next", &[], 1);
    assert!(
        result.is_err(),
        "unclassifiable object claimed a Pending job: {result:?}"
    );
    assert!(cdr_store::async_resolution::admission_held(&f.db, "thread-b").unwrap());
    assert_eq!(
        ingress::get(&f.db, &intent.ingress_id).unwrap(),
        Some(intent)
    );
    assert_no_mutation(&f, &jobs);
    f.server.close().await.unwrap();
}

async fn rejects_recovery(payload: Value) {
    let f = HistoryFixture::new(&completed_history()).await;
    let intent = replace_payload(&f, &payload);
    let jobs = queue::list(&f.db).unwrap();
    let result = owning_queue(&f).recover_target("thread-b").await;
    assert_no_mutation(&f, &jobs);
    assert_eq!(result.unwrap().started, 0);
    assert_eq!(f.obligation().1, "terminal");
    assert!(cdr_store::async_resolution::admission_held(&f.db, "thread-b").unwrap());
    assert_eq!(
        ingress::get(&f.db, &intent.ingress_id).unwrap(),
        Some(intent)
    );
    f.server.close().await.unwrap();
}

#[tokio::test]
async fn empty_object_cannot_claim_after_original_terminal_settlement() {
    rejects_claim(json!({})).await;
}

#[tokio::test]
async fn empty_execute_cannot_claim_after_original_terminal_settlement() {
    rejects_claim(json!({"version":1,"plan":{"Execute":{}}})).await;
}

#[tokio::test]
async fn empty_object_cannot_resume_in_public_terminal_recovery() {
    rejects_recovery(json!({})).await;
}

#[tokio::test]
async fn empty_execute_cannot_resume_in_public_terminal_recovery() {
    rejects_recovery(json!({"version":1,"plan":{"Execute":{}}})).await;
}

fn ordinary_commands() -> Vec<crate::command_plan::CommandAction> {
    use crate::command_plan::CommandAction::*;
    vec![
        Help,
        List { limit: 10 },
        ArchivedList { limit: 10 },
        Use {
            reference: "target".into(),
        },
        Status { reference: None },
        Settings {
            reference: None,
            model: None,
            effort: None,
            speed: None,
        },
        AutoReserve {
            reference: None,
            enabled: false,
        },
        Where,
        Context {
            all_threads: false,
            refresh: false,
            limit: 10,
        },
        Usage { days: 7 },
        New {
            prompt: String::new(),
        },
        Ask {
            prompt: "ordinary".into(),
        },
        Interview {
            prompt: "ordinary".into(),
        },
        Doctor,
        Runners,
        SavedRequest {
            request_id: "request".into(),
        },
        Retract { reference: None },
        MirrorCheck,
        MirrorInspect {
            limit: None,
            list: false,
        },
        BridgeSync { limit: None },
        QaButtons,
        Open {
            reference: "target".into(),
            abort: false,
        },
        Recover { reference: None },
        Repair { reference: None },
        SettingsOptions {
            reference: None,
            field: None,
        },
        RestartCodex,
        ForceRestartCodex,
        DeleteArchivePreview {
            reference: "target".into(),
        },
        DeleteArchiveConfirm {
            reference: "target".into(),
        },
        Resume { reference: None },
        Identity,
        Resources,
        Approval,
        Steer {
            prompt: "ordinary".into(),
        },
        HostReboot,
    ]
}

fn ordinary_payloads() -> Vec<Value> {
    use cdr_discord::components::{ApprovalAnswer, BusyAction, ComponentId};
    let mut result = ordinary_commands()
        .into_iter()
        .map(|command| json!({"version":1,"plan":{"Execute":command},"lifecycle_binding":null}))
        .collect::<Vec<_>>();
    for plan in [
        json!({"Respond":"known response"}),
        json!({"Error":"known rejection"}),
        json!({"Ignore":"known ignore"}),
    ] {
        result.push(json!({"version":1,"plan":plan}));
    }
    for component in [
        ComponentId::AsyncChoice {
            question_id: "a".repeat(64),
            option: 0,
        },
        ComponentId::Busy {
            choice_id: "b".repeat(24),
            action: BusyAction::Queue,
        },
        ComponentId::Busy {
            choice_id: "b".repeat(24),
            action: BusyAction::Steer,
        },
        ComponentId::Busy {
            choice_id: "b".repeat(24),
            action: BusyAction::Ignore,
        },
        ComponentId::Approval {
            thread_id: "thread-b".into(),
            answer: ApprovalAnswer::Approve,
        },
        ComponentId::BoundApproval {
            thread_fingerprint: "c".repeat(16),
            request_fingerprint: "d".repeat(32),
            answer: ApprovalAnswer::Reject,
        },
        ComponentId::Input {
            thread_id: "thread-b".into(),
            value: "yes".into(),
        },
        ComponentId::BoundInput {
            thread_fingerprint: "c".repeat(16),
            request_fingerprint: "d".repeat(32),
            value: "yes".into(),
        },
    ] {
        result.push(
            json!({"version":1,"work":cdr_discord::interaction::RoutedWork::Component(component)}),
        );
    }
    result.extend([
        json!({"version":1,"work":{"Slash":{"name":"usage","values":{"days":{"Integer":7}}}}}),
        json!({"version":1,"work":{"Slash":{"name":"ask","values":{"prompt":{"String":"ordinary"}}}}}),
        json!({"version":1,"command":"usage"}),
    ]);
    result
}

#[tokio::test]
async fn actual_noncontrol_serializations_and_unknown_shapes_stay_distinct() {
    let f = HistoryFixture::new(&completed_history()).await;
    queue::complete(&f.db, "next").unwrap();
    owning_queue(&f).recover_target("thread-b").await.unwrap();
    let original = unresolved_archive(&f);
    let positive = ordinary_payloads();
    let negative = [
        json!({}),
        json!({"version":1,"plan":{"Execute":{}}}),
        json!({"version":1,"plan":{"Execute":{"Ask":{}}}}),
        json!({"version":1,"plan":{"Execute":{"Usage":{"days":"7"}}}}),
        json!({"version":1,"plan":{"Execute":{"UnknownCommand":{}}}}),
        json!({"version":1,"plan":{"Respond":false}}),
        json!({"version":1,"plan":{"Execute":"Help","Error":"ambiguous"}}),
        json!({"version":1,"work":{"Slash":{"name":"","values":{}}}}),
        json!({"version":1,"work":{"Component":{"AsyncChoice":{"question_id":"a"}}}}),
        json!({"version":1,"plan":{"Execute":{"Ask":{"prompt":"ordinary"}}},"lifecycle_binding":{}}),
        json!({"version":2,"plan":{"Execute":{"Usage":{"days":7}}}}),
        json!({"version":1,"work":{"Component":{"Busy":{"choice_id":"b".repeat(24),"action":"Stop"}}}}),
    ];
    for (payload, expected) in positive
        .iter()
        .map(|v| (v, false))
        .chain(negative.iter().map(|v| (v, true)))
    {
        cdr_store::schema::open_initialized(&f.db)
            .unwrap()
            .execute(
                "UPDATE discord_ingress_journal SET payload_json=? WHERE ingress_id=?",
                rusqlite::params![payload.to_string(), original.ingress_id],
            )
            .unwrap();
        let stored = ingress::get(&f.db, &original.ingress_id).unwrap();
        assert_eq!(
            cdr_store::async_resolution::admission_held(&f.db, "thread-b").unwrap(),
            expected,
            "classification: {payload}"
        );
        assert_eq!(ingress::get(&f.db, &original.ingress_id).unwrap(), stored);
        assert_no_mutation(&f, &[]);
    }
    f.server.close().await.unwrap();
}
