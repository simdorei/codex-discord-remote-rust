//! Opt-in G0 diagnostic. Creates only its own QA thread and closes its own clients.
//! This is not a Discord feature implementation or a mocked end-to-end proof.
use std::time::Duration;

use cdr_app_server::{AppServerClient, AppServerConfig, Notification, requests};
use serde_json::{Value, json};
use tokio::sync::broadcast;

type ProbeResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
const RPC_WAIT: Duration = Duration::from_secs(15);

#[tokio::main]
async fn main() -> ProbeResult<()> {
    let executable = std::env::var_os("CDR_LIVE_CODEX_EXE")
        .ok_or("CDR_LIVE_CODEX_EXE must be explicitly configured")?;
    let cwd = std::env::var("CDR_QUESTION_QA_CWD")?;
    let mode = std::env::var("CDR_QUESTION_QA_MODE")?;
    if !matches!(mode.as_str(), "active" | "completed" | "cross_writer") {
        return Err("unsupported explicit probe mode".into());
    }
    let mut config = AppServerConfig::new(executable);
    config.client_name = "cdr_question_transport_qa".into();
    config.client_title = "Question transport QA".into();
    let owner = AppServerClient::start(config.clone()).await?;
    println!("qa_owner_pid={:?}", owner.lifecycle_snapshot().process_id);
    let other = if mode == "cross_writer" {
        match AppServerClient::start(config).await {
            Ok(client) => Some(client),
            Err(error) => {
                owner.close().await?;
                return Err(error.into());
            }
        }
    } else {
        None
    };
    let result = tokio::time::timeout(
        Duration::from_secs(100),
        probe(&owner, other.as_ref(), &cwd, &mode),
    )
    .await;
    if let Some(other) = other {
        other.close().await?;
        println!("qa_other_closed=true");
    }
    owner.close().await?;
    println!("qa_owner_closed=true");
    result??;
    Ok(())
}

async fn probe(
    owner: &AppServerClient,
    other: Option<&AppServerClient>,
    cwd: &str,
    mode: &str,
) -> ProbeResult<()> {
    let mut events = owner.subscribe_notifications();
    let started = owner.request("thread/start", json!({
        "cwd":cwd,"ephemeral":false,"approvalPolicy":"never","sandbox":"read-only",
        "developerInstructions":"This is a narrowly scoped transport QA. Do not read or write files, inspect environment, call external services, delegate, or execute commands. Only request_user_input_async is needed. Never use another question tool as a substitute. Report ASYNC_TOOL_UNAVAILABLE if unavailable."
    }), RPC_WAIT).await?;
    let thread = started
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .ok_or("thread/start did not return an ID")?;
    println!(
        "{}",
        json!({"probe_mode":mode,"thread_id":thread,"model":started.get("model")})
    );
    let prompt = "This is question transport QA. Call request_user_input_async exactly once with two questions: title 'QA-A: choose for project A' and title 'QA-B: choose for project B'. Each question has exactly the string options ['Allow', 'Hold']. Do not ask in ordinary prose. After asking, write a 250-word commentary on safe testing to allow an asynchronous answer to arrive. When an answer arrives, state which question it answered, its answer, and which question remains unanswered. Do not infer an answer to the other question. If no answer arrives, end with WAITING_FOR_REPLY. Use no tools other than request_user_input_async.";
    let request = requests::start_turn(thread, prompt);
    let turn_started = owner
        .request(request.method, request.params, RPC_WAIT)
        .await?;
    let turn = turn_started
        .pointer("/turn/id")
        .and_then(Value::as_str)
        .ok_or("turn/start did not return an ID")?;
    println!("qa_original_turn={turn}");
    let question = wait_question(&mut events, thread, turn).await?;
    println!("{}", json!({"qa_question":question}));
    let item_id = question
        .get("id")
        .and_then(Value::as_str)
        .ok_or("question item has no immutable ID")?;
    let answer = format!(
        "Discord answer to one specific question. Original turn: {turn}. Question occurrence item: {item_id}. Question index: 2. Exact question: QA-B: choose for project B. Selected answer: Hold. This answers only QA-B; QA-A remains unanswered. Report the answered question and selection, without inventing QA-A's answer."
    );
    if mode == "cross_writer" {
        let other = other.ok_or("cross-writer client missing")?;
        let read = other
            .request(
                "thread/read",
                json!({"threadId":thread,"includeTurns":true}),
                RPC_WAIT,
            )
            .await;
        match read {
            Ok(history) => println!(
                "{}",
                json!({"other_can_read":true,"turn_count":history.pointer("/thread/turns").and_then(Value::as_array).map(Vec::len)})
            ),
            Err(error) => println!("other_read_error={error}"),
        }
        let resume = other
            .request("thread/resume", json!({"threadId":thread}), RPC_WAIT)
            .await;
        match resume {
            Ok(_) => println!("other_resume_accepted=true"),
            Err(error) => {
                println!("other_resume_error={error}");
                return Err(
                    "G0 other writer resume rejected; no answer or fallback dispatched".into(),
                );
            }
        }
        return Err(
            "G0 other resume accepted; inspect before any cross-writer answer dispatch".into(),
        );
    }
    let request = if mode == "completed" {
        wait_completion(&mut events, thread, turn).await?;
        requests::start_turn(thread, &answer)
    } else {
        requests::steer_turn(thread, &answer, turn)
    };
    println!(
        "{}",
        json!({"answer_method":request.method,"answer_payload":request.params})
    );
    let accepted = owner
        .request(request.method, request.params, RPC_WAIT)
        .await?;
    println!("{}", json!({"answer_accepted":accepted}));
    let reply_turn = accepted
        .pointer("/turn/id")
        .and_then(Value::as_str)
        .unwrap_or(turn);
    wait_completion(&mut events, thread, reply_turn).await?;
    println!(
        "qa_transport_completed=true; question-specific answer requires independent inspection"
    );
    Ok(())
}

async fn next_event(
    events: &mut broadcast::Receiver<Notification>,
    thread: &str,
    turn: &str,
) -> ProbeResult<Notification> {
    loop {
        let event = events.recv().await?;
        if event.params.get("threadId").and_then(Value::as_str) != Some(thread) {
            continue;
        }
        let event_turn = event
            .params
            .get("turnId")
            .and_then(Value::as_str)
            .or_else(|| event.params.pointer("/turn/id").and_then(Value::as_str));
        if event_turn != Some(turn) {
            continue;
        }
        if event.method == "item/completed" {
            let item = &event.params["item"];
            if item["type"] == "agentMessage" {
                println!("{}", json!({"agent_message":item,"original_turn":turn}));
            }
        } else if event.method == "error" {
            return Err(format!("QA turn error: {}", event.params["error"]).into());
        }
        return Ok(event);
    }
}

async fn wait_question(
    events: &mut broadcast::Receiver<Notification>,
    thread: &str,
    turn: &str,
) -> ProbeResult<Value> {
    loop {
        let event = next_event(events, thread, turn).await?;
        if event.method == "item/completed" {
            let item = &event.params["item"];
            if item["type"] == "agentMessage" && item["delivery"] == "async" {
                if item["questions"]
                    .as_array()
                    .is_none_or(|questions| questions.len() != 2)
                {
                    return Err(
                        "G0 actual async question did not match the two-question fixture".into(),
                    );
                }
                return Ok(item.clone());
            }
        }
        if event.method == "turn/completed" {
            println!(
                "{}",
                json!({"completed_before_question":event.params["turn"]})
            );
            return Err("G0 producer completed without an async question item".into());
        }
    }
}

async fn wait_completion(
    events: &mut broadcast::Receiver<Notification>,
    thread: &str,
    turn: &str,
) -> ProbeResult<()> {
    loop {
        let event = next_event(events, thread, turn).await?;
        if event.method == "turn/completed" {
            let result = &event.params["turn"];
            println!("{}", json!({"qa_turn_completed":result}));
            if result["status"] != "completed" {
                return Err("QA turn did not complete successfully".into());
            }
            return Ok(());
        }
    }
}
