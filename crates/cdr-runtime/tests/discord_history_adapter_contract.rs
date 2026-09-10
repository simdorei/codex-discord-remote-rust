const HISTORY_POLL: &str = include_str!("../src/history_poll/mod.rs");
const HISTORY_ADAPTER: &str = include_str!("../src/history_poll/discord_adapter.rs");
const MESSAGE_WORKER: &str = include_str!("../src/message_worker.rs");
const PROCESSING_BOUNDARY: &str = include_str!("../src/message_worker/processing_boundary.rs");
const MESSAGE_CREATE: &str = include_str!("../src/discord_runtime/message_create.rs");

fn compact(source: &str) -> String {
    source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

#[test]
fn dha_00_history_poll_declares_a_production_discord_adapter() {
    assert!(
        compact(HISTORY_POLL).contains("moddiscord_adapter;"),
        "history polling still has no production Discord adapter"
    );
    let adapter = compact(HISTORY_ADAPTER);
    assert!(adapter.contains("typePayload=Message;"));
    assert!(adapter.contains("fetch_latest_channel_messages(channel_id)"));
    assert!(adapter.contains("limit!=HISTORY_POLL_PAGE_LIMIT"));
    assert!(!adapter.contains("message.clone()"));
    assert!(!adapter.contains("(*message).clone()"));
}

#[test]
fn dha_00_live_message_dispatch_uses_the_shared_processing_boundary() {
    let worker = compact(MESSAGE_WORKER);
    let live = compact(MESSAGE_CREATE);

    assert!(worker.contains("modprocessing_boundary;"));
    assert!(worker.contains("process_with_error_report"));
    assert!(live.contains("process_with_error_report("));
    assert!(!live.contains("asyncfnreport_processing_error("));
}

#[test]
fn dha_05_bot_ignore_precedes_classifier_and_policy_is_a_cycle_snapshot() {
    let adapter = compact(HISTORY_ADAPTER);
    let bot = adapter
        .find("ifmessage.author.bot{")
        .expect("bot history boundary");
    let classify = adapter
        .find("letkind=matchclassify(message)?{")
        .expect("canonical classifier boundary");

    assert!(bot < classify);
    assert!(adapter.contains("policy:InteractionAccessPolicy"));
    assert!(!adapter.contains("interaction_policy("));
    assert_eq!(
        adapter.matches("classify_gateway_message(message,").count(),
        1
    );
}

#[test]
fn dha_06_shared_boundary_reports_ordinary_errors_but_not_database_mismatch() {
    let boundary = compact(PROCESSING_BOUNDARY);
    let process = boundary
        .find("matchprocess(work).await{")
        .expect("process call");
    let mismatch = boundary
        .find("iferror.is_database_mismatch()=>Err(error)")
        .expect("fatal database mismatch branch");
    let report = boundary
        .find("report(target,error).await;")
        .expect("ordinary error report");

    assert!(process < mismatch && mismatch < report);
    assert_eq!(boundary.matches("report(target,error).await").count(), 1);
}
