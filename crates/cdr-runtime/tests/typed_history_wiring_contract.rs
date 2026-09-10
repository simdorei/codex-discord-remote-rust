const HISTORY_POLL: &str = include_str!("../src/history_poll/mod.rs");
const DISCORD_RUNTIME: &str = include_str!("../src/discord_runtime.rs");
const GATEWAY_LOOP: &str = include_str!("../src/discord_runtime/gateway_loop.rs");
const TYPED_INGRESS: &str = include_str!("../src/discord_runtime/typed_ingress.rs");
const TYPED_HISTORY: &str = include_str!("../src/discord_runtime/typed_ingress/history.rs");
const HISTORY_CYCLE: &str = include_str!("../src/discord_runtime/typed_ingress/history/cycle.rs");
const HISTORY_GAP: &str = include_str!("../src/discord_runtime/typed_ingress/history/gap.rs");
const TYPED_MESSAGE: &str = include_str!("../src/discord_runtime/typed_ingress/message.rs");
const TYPED_READY: &str = include_str!("../src/discord_runtime/typed_ingress/ready.rs");
const DISCORD_HISTORY_ADAPTER: &str = include_str!("../src/history_poll/discord_adapter.rs");
const DISCORD_HISTORY_CLAIM: &str = include_str!("../src/history_poll/discord_adapter/claim.rs");

fn compact(source: &str) -> String {
    source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

#[test]
fn thw_00_history_poll_declares_inclusive_gap_recovery() {
    assert!(
        compact(HISTORY_POLL).contains("modgap_runner;"),
        "inclusive gap recovery runner is missing"
    );
}

#[test]
fn thw_00_runtime_uses_paused_typed_consumer_activation() {
    let runtime = compact(DISCORD_RUNTIME);

    assert!(runtime.contains("GatewayRuntime::start_paused("));
    assert!(!runtime.contains("GatewayRuntime::start("));
    assert!(runtime.contains("TypedIngressWorkers::start("));
}

#[test]
fn thw_00_activation_follows_subscriptions_transfer_and_worker_readiness() {
    let ingress = compact(TYPED_INGRESS);
    let subscription = ingress
        .find("letready_identity=gateway.subscribe_identity();")
        .expect("identity subscription");
    let transfer = ingress
        .find("letreceivers=gateway.take_ingress_receivers()?;")
        .expect("typed receiver transfer");
    let readiness = ingress
        .find("ifreceiver.await.is_err()")
        .expect("consumer readiness barrier");
    let activation = ingress
        .find("gateway.activate_typed_consumers()")
        .expect("typed activation");

    assert!(subscription < transfer && transfer < readiness && readiness < activation);
}

#[test]
fn thw_00_gap_recovery_precedes_initial_poll_and_policy_follows_fixed_boundary() {
    let history = compact(TYPED_HISTORY);
    let gap = history.find("ifgap_due{").expect("gap priority");
    let poll = history.find("ifpoll_due{").expect("periodic poll");
    assert!(gap < poll);

    let cycle = compact(HISTORY_CYCLE);
    let boundary = cycle
        .find("letcycle=state.begin(target.channel_id,poll_started_at)?;")
        .expect("fixed poll boundary");
    let policy = cycle
        .find("letpolicy=matchinteraction_policy(")
        .expect("policy snapshot");
    assert!(boundary < policy);
}

#[test]
fn thw_00_message_processing_is_owned_sequential_and_cancellation_guarded() {
    let message = compact(TYPED_MESSAGE);
    assert!(!message.contains("tokio::spawn("));
    let receive = message.find("messages.recv()").expect("message receive");
    let process = message
        .find("handle_message_create(")
        .expect("message processing");
    let guard = message.find("guard(process,").expect("cancellation guard");
    assert!(receive < process && process < guard);
}

#[test]
fn thw_00_ready_retries_independently_and_gap_ack_requires_complete_coverage() {
    let ready = compact(TYPED_READY);
    assert!(ready.contains(
        "constRETRY_DELAYS:[Duration;6]=[Duration::from_secs(1),Duration::from_secs(2),Duration::from_secs(4),Duration::from_secs(8),Duration::from_secs(16),Duration::from_secs(30),];"
    ));
    assert!(ready.contains("tokio::time::sleep(delay).await"));
    assert!(ready.contains("guard(setup,"));

    let gap = compact(HISTORY_GAP);
    let reached = gap
        .find("outcome.coverage==HistoryGapCoverage::Reached")
        .expect("complete coverage gate");
    let acknowledge = gap.find("gaps.acknowledge(notice)?").expect("gap ack");
    let incomplete = gap
        .find("status=incomplete")
        .expect("degraded incomplete status");
    assert!(reached < acknowledge && acknowledge < incomplete);
}

#[test]
fn thw_00_unresolved_gap_blocks_normal_prime_before_any_boundary_or_claim() {
    let cycle = compact(HISTORY_CYCLE);
    let gap_check = cycle
        .find("letSome(gap_fence)=capture_clear_gap_fence(gaps,target.channel_id)?")
        .expect("pending gap fence");
    let boundary = cycle
        .find("SystemTime::now()")
        .expect("poll boundary clock");
    let begin = cycle.find("state.begin(").expect("history state begin");
    assert!(gap_check < boundary && boundary < begin);
}

#[test]
fn thw_00_gap_revision_fence_spans_fetch_and_guards_prime_discard_claims() {
    let cycle = compact(HISTORY_CYCLE);
    let capture = cycle
        .find("letSome(gap_fence)=capture_clear_gap_fence(")
        .expect("capture revision-bound gap fence");
    let boundary = cycle
        .find("SystemTime::now()")
        .expect("poll boundary clock");
    let fenced_io = cycle
        .find("DiscordHistoryCycleIo::new_fenced(")
        .expect("fenced production history adapter");
    assert!(capture < boundary && boundary < fenced_io);

    let adapter = compact(DISCORD_HISTORY_ADAPTER);
    assert!(adapter.contains("HistoryClaimPurpose::Discard"));
    assert!(compact(DISCORD_HISTORY_CLAIM).contains("with_current_fence("));
}

#[test]
fn thw_00_ready_registration_and_notice_have_independent_retry_loops() {
    let ready = compact(TYPED_READY);
    let registration = ready
        .find("letregistration=register_until_success(")
        .expect("registration retry future");
    let notice = ready
        .find("letnotice=send_notice_until_success(")
        .expect("notice retry future");
    let joined = ready
        .find("tokio::join!(registration,notice)")
        .expect("independent setup join");
    assert!(registration < notice && notice < joined);
    assert_eq!(ready.matches("letmutfailures=0_usize;").count(), 2);
}

#[test]
fn thw_00_oversized_gap_page_is_a_fatal_invariant() {
    let gap = compact(HISTORY_GAP);
    let oversized = gap
        .find("Err(HistoryGapRecoveryError::BatchTooLarge{actual})")
        .expect("oversized gap branch");
    let fatal = gap[oversized..]
        .find("returnErr(DiscordRuntimeError::HistoryGapBatchTooLarge{actual})")
        .expect("fatal invariant propagation");
    assert!(fatal < 120, "oversized branch must fail directly");
}

#[test]
fn thw_00_legacy_loop_no_longer_dispatches_typed_ingress_events() {
    let gateway = compact(GATEWAY_LOOP);

    assert!(!gateway.contains("GatewayItem"));
    assert!(!gateway.contains("Event::Ready("));
    assert!(!gateway.contains("Event::MessageCreate("));
    assert!(!gateway.contains("Event::InteractionCreate("));
}
