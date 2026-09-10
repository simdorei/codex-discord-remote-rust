const GATEWAY_LOOP: &str = include_str!("../src/discord_runtime/gateway_loop.rs");
const RECEIVE_ERROR: &str = include_str!("../src/discord_runtime/typed_ingress/receive_error.rs");
const TYPED_MESSAGE: &str = include_str!("../src/discord_runtime/typed_ingress/message.rs");
const MESSAGE_CREATE: &str = include_str!("../src/discord_runtime/message_create.rs");
const DISCORD_RUNTIME_ERROR: &str = include_str!("../src/discord_runtime/error.rs");
const MESSAGE_WORKER: &str = include_str!("../src/message_worker.rs");
const PROCESSING_BOUNDARY: &str = include_str!("../src/message_worker/processing_boundary.rs");

fn compact(source: &str) -> String {
    source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn position(source: &str, needle: &str) -> usize {
    source
        .find(needle)
        .unwrap_or_else(|| panic!("missing pre-claim boundary marker: {needle}"))
}

#[test]
fn mpb_05_gateway_classifies_once_before_admission_and_never_reports_boundary_errors() {
    let handler = compact(MESSAGE_CREATE);
    let prepare_start = position(&handler, "fnprepare_message_create_at");
    let dispatch_start = position(&handler, "asyncfndispatch_message_create");
    let handle_start = position(&handler, "pub(super)asyncfnhandle_message_create");
    let tests_start = position(
        &handler[handle_start..],
        "#[cfg(test)]#[path=\"message_create_tests.rs\"]modtests;",
    ) + handle_start;
    let prepare = &handler[prepare_start..dispatch_start];
    let dispatch = &handler[dispatch_start..handle_start];
    let handle = &handler[handle_start..tests_start];
    let policy = position(prepare, "interaction_policy(");
    let classify = position(prepare, "classify_gateway_message(");
    let admit = position(prepare, "admit_message_candidate_at(");

    assert!(
        policy < classify && classify < admit,
        "read-only policy and canonical classification must precede durable admission"
    );
    assert!(dispatch.contains("PreparedMessage::Duplicate=>returnOk(())"));

    assert_eq!(handler.matches("classify_gateway_message(").count(), 1);
    assert!(prepare.contains("map_err(MessageCreateBoundaryError::Admission)?"));
    assert!(!prepare.contains("report_processing_error("));
    assert!(dispatch.contains("let(admitted,_admission_permit)=matchprepare()?"));

    assert!(
        dispatch.contains("process_with_error_report(report_target,admitted,process,report).await")
    );
    let boundary = compact(PROCESSING_BOUNDARY);
    let process = position(&boundary, "matchprocess(work).await");
    let mismatch = position(&boundary, "error.is_database_mismatch()");
    let propagated = position(&boundary[mismatch..], "=>Err(error)") + mismatch;
    let error_report = position(&boundary, "report(target,error).await");
    assert!(
        process < mismatch && mismatch < propagated && propagated < error_report,
        "database-affinity failures must exit before the ordinary ErrorReport path"
    );
    let handler_dispatch = position(handle, "dispatch_message_create(");
    let before_dispatch = &handle[..handler_dispatch];
    for forbidden in [
        "create_dir_all(",
        "enrich_message_attachments(",
        "execute_with_context(",
        "send_reply_once(",
        "report_processing_error(",
    ] {
        assert!(
            !before_dispatch.contains(forbidden),
            "side effect appeared before MessageCreate dispatch boundary: {forbidden}"
        );
    }
}

#[test]
fn mpb_06_runtime_preserves_the_admission_error_as_a_transparent_source() {
    let runtime = compact(DISCORD_RUNTIME_ERROR);
    assert!(
        runtime.contains("#[error(transparent)]MessageAdmission(#[from]MessageAdmissionError)")
    );

    let cause = std::io::Error::other("admission source sentinel");
    let error = DiscordRuntimeError::from(MessageAdmissionError::from(StoreError::Io(cause)));
    assert_eq!(error.to_string(), "I/O error: admission source sentinel");
    assert!(error.source().is_some());
    match error {
        DiscordRuntimeError::MessageAdmission(MessageAdmissionError::Store(StoreError::Io(
            source,
        ))) => assert_eq!(source.to_string(), "admission source sentinel"),
        other => panic!("unexpected admission source chain: {other:?}"),
    }
}

#[test]
fn mpb_07_gateway_cannot_bypass_the_opaque_admitted_message_token() {
    let worker = compact(MESSAGE_WORKER);
    let gateway = compact(GATEWAY_LOOP);
    let message_create = compact(MESSAGE_CREATE);

    assert!(
        worker.contains("process_admitted_gateway_message<B:TurnBackend>(admitted:AdmittedMessage")
    );
    assert!(!gateway.contains("process_gateway_message("));
    assert!(!gateway.contains("AdmittedMessage{"));
    assert!(!message_create.contains("AdmittedMessage{"));
    let handle_start = position(&message_create, "pub(super)asyncfnhandle_message_create");
    let tests_start = position(
        &message_create[handle_start..],
        "#[cfg(test)]#[path=\"message_create_tests.rs\"]modtests;",
    ) + handle_start;
    let handle = &message_create[handle_start..tests_start];
    let target = position(
        handle,
        "letreport_target=ErrorReportTarget::from_message(&message);",
    );
    let consume = position(handle, "prepare_message_create(message,");
    let report = position(
        handle,
        "report_processing_error(database,&api,target,error)",
    );
    assert!(message_create.contains("pub(super)asyncfnhandle_message_create(message:Message,"));
    assert!(target < consume && consume < report);
    assert!(!message_create.contains("message.clone()"));
    assert!(!message_create.contains("(*message).clone()"));
    assert!(!message_create.contains("plan_message("));
    let report_function = compact(PROCESSING_BOUNDARY);
    assert!(
        report_function
            .contains("target.channel_id,target.message_id,MessageReplyKind::ErrorReport")
    );
}

#[test]
fn mpb_08_receive_decode_errors_are_log_only() {
    assert!(RECEIVE_ERROR.contains("Discord gateway receive error"));
    assert!(!RECEIVE_ERROR.contains("ErrorReport"));
    assert!(!RECEIVE_ERROR.contains("send_reply_once"));
    assert!(!GATEWAY_LOOP.contains("GatewayItem"));
}

#[test]
fn mpb_09_gateway_requires_ready_identity_before_durable_admission() {
    let consumer = compact(TYPED_MESSAGE);
    let identity = position(&consumer, "wait_for_identity(");
    let receive = position(&consumer, "messages.recv()");
    let handler = position(&consumer, "handle_message_create(");

    assert!(
        identity < receive && receive < handler,
        "the typed consumer must establish Ready identity before receiving and admitting messages"
    );
    assert!(consumer.contains("handle_message_create(envelope.event.0,"));
    assert!(!consumer.contains("event.clone()"));
    assert!(!consumer.contains("message.clone()"));
    assert!(!compact(GATEWAY_LOOP).contains("Event::MessageCreate("));
}

use std::error::Error;

use cdr_runtime::discord_runtime::DiscordRuntimeError;
use cdr_runtime::message_worker::MessageAdmissionError;
use cdr_store::StoreError;
