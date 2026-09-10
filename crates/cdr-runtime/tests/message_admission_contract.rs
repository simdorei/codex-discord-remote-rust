use std::sync::{Arc, Barrier};
use std::thread;

use cdr_store::processed::{claim, is_processed, mark};

const MESSAGE_ADMISSION: &str = include_str!("../src/message_worker/admission.rs");
const MESSAGE_CLASSIFICATION: &str = include_str!("../src/message_worker/classification.rs");
const MESSAGE_WORKER: &str = include_str!("../src/message_worker.rs");
const MESSAGE_EXECUTION: &str = include_str!("../src/message_worker/execution.rs");
const MESSAGE_CREATE: &str = include_str!("../src/discord_runtime/message_create.rs");
const MESSAGE_DISCARD: &str = include_str!("../src/message_worker/discard.rs");

fn compact(source: &str) -> String {
    source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn position(source: &str, needle: &str) -> usize {
    source
        .find(needle)
        .unwrap_or_else(|| panic!("missing message admission marker: {needle}"))
}

fn assert_token_is_not_clone(source: &str, declaration: &str, type_name: &str) {
    let declaration_start = position(source, declaration);
    let preceding_item_end = source[..declaration_start]
        .rfind('}')
        .map_or(0, |offset| offset + 1);
    let declaration_attributes = &source[preceding_item_end..declaration_start];
    assert!(
        !declaration_attributes.contains("Clone"),
        "{type_name} must remain an owning, consuming token"
    );
    for manual_impl in [
        format!("implClonefor{type_name}"),
        format!("implstd::clone::Clonefor{type_name}"),
        format!("implcore::clone::Clonefor{type_name}"),
    ] {
        assert!(
            !source.contains(&manual_impl),
            "{type_name} must not gain a manual Clone implementation"
        );
    }
}

#[test]
fn dm_claim_01_concurrent_copies_have_exactly_one_admission_winner() {
    const WORKERS: usize = 16;

    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");
    assert!(!is_processed(&database, 41).expect("initialize message store"));

    let barrier = Arc::new(Barrier::new(WORKERS));
    let handles = (0..WORKERS)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            let database = database.clone();
            thread::spawn(move || {
                barrier.wait();
                claim(&database, 41, 100.0)
            })
        })
        .collect::<Vec<_>>();

    let winners = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .expect("message admission worker did not panic")
                .expect("message admission store operation succeeded")
        })
        .filter(|claimed| *claimed)
        .count();

    assert_eq!(winners, 1);
}

#[test]
fn dm_claim_02_downstream_failure_leaves_a_durable_replay_barrier() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");

    assert!(claim(&database, 42, 100.0).expect("first admission succeeds"));
    // Simulate any downstream error by intentionally doing no success refresh.
    assert!(is_processed(&database, 42).expect("claim survives connection reopen"));
    assert!(!claim(&database, 42, 101.0).expect("redelivery checks durable claim"));
}

#[test]
fn dm_claim_03_and_05_gateway_claims_before_every_side_effect_and_propagates_errors() {
    let admission = compact(MESSAGE_ADMISSION);
    let classification = compact(MESSAGE_CLASSIFICATION);
    let handler = compact(MESSAGE_CREATE);
    let worker = compact(MESSAGE_WORKER);

    assert!(!admission.contains("is_processed("));
    assert_eq!(admission.matches("admit(").count(), 1);
    assert!(admission.contains("letadmission=admit(&parts.database,&request)?;"));
    assert!(admission.contains("if!admission.created{"));
    assert!(!admission.contains("cdr_store::processed::claim"));

    let id = position(&classification, "i64::try_from(message.id.get())");
    let mirror = position(&classification, "new_thread_origin(");
    let plan = position(&classification, "letfrozen_plan=planner(");
    let candidate = position(
        &classification,
        "MessageClassification::Candidate(MessageCandidate{",
    );
    let clock = position(&admission, ".duration_since(UNIX_EPOCH)");
    let payload = position(
        &admission,
        "letrequest=custody::request(&parts,observed_at)?;",
    );
    let claim = position(&admission, "letadmission=admit(");
    let custody = position(&admission, "Box::new(MessageCustody::new(");
    let token = position(&admission, "Ok(Some(AdmittedMessage{");
    assert!(id < mirror && mirror < plan && plan < candidate);
    assert!(clock < payload && payload < claim && claim < custody && custody < token);
    assert!(admission.contains("custody:Box<MessageCustody>"));
    assert!(admission.contains("map_err(StoreError::from)?"));
    assert!(!admission.contains("release"));
    assert!(!admission.contains("unclaim"));

    let policy = position(&handler, "interaction_policy(");
    let classify = position(&handler, "classify_gateway_message(");
    let admit = position(&handler, "admit_message_candidate_at(");
    let process = position(&worker, "pub(crate)asyncfnprocess_admitted_gateway_message");
    assert!(policy < classify && classify < admit);
    assert!(worker[process..].contains("admitted.into_processing_parts("));
    assert!(!worker.contains("plan_message("));
    assert!(!worker.contains("claim("));
}

#[test]
fn dm_claim_04_success_refresh_does_not_reopen_admission() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("mirror.sqlite");

    assert!(claim(&database, 43, 100.0).expect("first admission succeeds"));
    mark(&database, 43, 150.0).expect("successful processing refreshes timestamp");
    assert!(!claim(&database, 43, 151.0).expect("refresh keeps the message claimed"));

    let connection = rusqlite::Connection::open(&database).expect("reopen message database");
    let seen_at = connection
        .query_row(
            "SELECT seen_at FROM discord_processed_messages WHERE message_id = 43",
            [],
            |row| row.get::<_, f64>(0),
        )
        .expect("read refreshed timestamp");
    assert!((seen_at - 150.0).abs() < f64::EPSILON);
}

#[test]
fn dm_claim_05_store_errors_are_errors_not_admission_success() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let database = temp.path().join("missing-parent").join("mirror.sqlite");

    claim(&database, 44, 100.0).expect_err("missing parent must surface a store error");
}

#[test]
fn dm_claim_06_processing_has_no_post_side_effect_processed_store_write() {
    let worker = compact(MESSAGE_WORKER);
    let execution = compact(MESSAGE_EXECUTION);
    let handler = compact(MESSAGE_CREATE);
    let processing = &worker[position(&worker, "pub(crate)asyncfnprocess_admitted_gateway_message")
        ..position(&worker, "asyncfnhandle_pending_reply")];
    assert!(!processing.contains("mark("));

    let pending = &worker[position(&worker, "asyncfnhandle_pending_reply")
        ..position(&worker, "#[cfg(test)]moddrain_tests")];
    assert!(!pending.contains("mark("));
    assert!(!worker.contains("cdr_store::processed"));
    for source in [&worker, &execution, &handler] {
        assert!(!source.contains("mark("));
        assert!(!source.contains("release"));
        assert!(!source.contains("unclaim"));
        assert!(!source.contains("cdr_store::processed"));
    }
}

#[test]
fn dm_claim_07_tokens_are_consuming_crate_private_and_database_bound() {
    let admission = compact(MESSAGE_ADMISSION);
    let classification = compact(MESSAGE_CLASSIFICATION);
    assert!(classification.contains(
        "pub(crate)structMessageCandidate{message:Box<Message>,database:PathBuf,persisted_id:i64,"
    ));
    assert!(admission.contains(
        "pub(crate)structAdmittedMessage{message:Box<Message>,database:PathBuf,persisted_id:i64,"
    ));
    assert!(!classification.contains("structMessageCandidate<'"));
    assert!(!admission.contains("structAdmittedMessage<'"));
    assert!(classification.contains("fninto_admission_parts(self)"));
    assert!(admission.contains("fninto_processing_parts(self,context_database:&Path,"));
    assert_token_is_not_clone(
        &classification,
        "pub(crate)structMessageCandidate{",
        "MessageCandidate",
    );
    assert_token_is_not_clone(
        &admission,
        "pub(crate)structAdmittedMessage{",
        "AdmittedMessage",
    );
    assert!(admission.contains("ifself.database!=context_database{"));
    assert_eq!(classification.matches("planner(").count(), 1);
    assert!(classification.contains(
        "classify_gateway_message_with(message,database,config,policy,bot_user_id,plan_message)"
    ));
    assert!(!classification.contains("plan_message(&"));
    for source in [
        &classification,
        &admission,
        &compact(MESSAGE_WORKER),
        &compact(MESSAGE_CREATE),
    ] {
        assert!(!source.contains("message.clone()"));
        assert!(!source.contains("(*message).clone()"));
        assert!(!source.contains("Clone::clone(&message)"));
        assert!(!source.contains("message.to_owned()"));
    }
}

#[test]
fn dm_claim_08_processed_ids_have_no_in_memory_admission_cache() {
    let admission = compact(MESSAGE_ADMISSION).to_ascii_lowercase();
    for forbidden in ["hashset", "hashmap", "vecdeque", "lru", "cache", "capacity"] {
        assert!(
            !admission.contains(forbidden),
            "processed-ID admission must remain SQLite-only: found {forbidden}"
        );
    }
    assert!(admission.contains("cdr_store::ingress::{admit,record_processing_mode}"));
    assert!(!admission.contains("cdr_store::processed::claim"));
}

#[test]
fn history_discard_is_separate_and_cannot_issue_an_executable_admission_token() {
    let discard = compact(MESSAGE_DISCARD);
    assert!(discard.contains("Result<bool,MessageAdmissionError>"));
    assert!(discard.contains("cdr_store::processed::claim("));
    assert!(!discard.contains("AdmittedMessage"));
    assert!(!discard.contains("MessageCustody"));
    assert!(!discard.contains("ingress::"));
    assert!(!discard.contains("admit("));
    assert!(!compact(MESSAGE_ADMISSION).contains("discard_message_candidate_at("));
}
