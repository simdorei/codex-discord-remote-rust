use super::*;
use crate::{
    message_worker, new_reply_worker,
    test_support::{app_fixture, new_reply_fixture as support},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

const BODY: &str = "attachment-payload-7319";

#[derive(Clone, Copy)]
enum Case {
    Success,
    HttpError,
    Disabled,
    Oversized,
    CommitError,
}

async fn source(ok: bool) -> (String, oneshot::Sender<()>, tokio::task::JoinHandle<usize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/notes.txt", listener.local_addr().unwrap());
    let (stop, mut stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut requests = 0;
        loop {
            let (mut socket, _) =
                tokio::select! { _=&mut stopped=>break, value=listener.accept()=>value.unwrap() };
            let mut header = [0; 4096];
            let size = socket.read(&mut header).await.unwrap();
            assert!(header[..size].starts_with(b"GET /notes.txt "));
            requests += 1;
            let status = if ok {
                "200 OK"
            } else {
                "503 Service Unavailable"
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
                BODY.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        requests
    });
    (url, stop, task)
}

async fn deliver_final(
    fixture: &crate::test_support::message_fixture::MessageFixture,
    temp: &tempfile::TempDir,
) {
    let db = fixture.executor.mirror_db();
    let job = cdr_store::queue::list(db).unwrap().remove(0);
    support::persist(temp, &job.prompt);
    assert_eq!(
        new_reply_worker::reconcile(db).unwrap()[0].state,
        "verified"
    );
    let worker = CompletionWorker {
        server: fixture.server.clone(),
        queue: fixture.queue.clone(),
        http: fixture.http.clone(),
        commentary_enabled: true,
        history_read_timeout: Duration::from_secs(1),
        commentary: Mutex::new(CommentaryBuffer::default()),
        terminal_fence: terminal_fence::TerminalFence::default(),
    };
    worker
        .queue
        .stage_turn_completion("new-thread", "first-turn", "Final\n첨부 확인")
        .await
        .unwrap();
    worker.deliver_pending().await.unwrap();
    worker.deliver_pending().await.unwrap();
}

fn assert_success(
    temp: &tempfile::TempDir,
    rpc: &[serde_json::Value],
    posts: &[serde_json::Value],
) {
    let input = rpc
        .iter()
        .find(|r| r["method"] == "turn/start")
        .unwrap()
        .pointer("/params/input/0/text")
        .unwrap()
        .as_str()
        .unwrap();
    let path = temp.path().join("42").join("801").join("01-notes.txt");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), BODY);
    assert!(input.contains(path.to_string_lossy().as_ref()));
    assert!(input.contains(BODY));
    assert!(input.contains(&format!(
        "sha256: {}",
        hex::encode(Sha256::digest(BODY.as_bytes()))
    )));
    assert_eq!(posts.len(), 3);
    assert_eq!(
        posts[1]["content"],
        "In progress\nmessage: first with file\n새 대화: <#43>"
    );
    assert_eq!(posts[2]["content"], "Final\n첨부 확인");
    assert_eq!(posts[2]["test_channel"], 43);
}

fn admit_attachment(
    url: &str,
    db: &std::path::Path,
    config: &crate::config::RuntimeConfig,
) -> message_worker::AdmittedMessage {
    let message = serde_json::from_value(json!({
        "attachments":[{"id":"900","filename":"notes.txt","size":BODY.len(),"content_type":"text/plain","url":url,"proxy_url":url}],
        "author":{"avatar":null,"bot":false,"discriminator":"0001","id":"3","username":"fixture"},
        "channel_id":"42","content":"first with file","edited_timestamp":null,"embeds":[],"id":"801",
        "mention_everyone":false,"mention_roles":[],"mentions":[],"pinned":false,
        "timestamp":"2020-02-02T02:02:02.020000+00:00","tts":false,"type":0
    })).unwrap();
    let message_worker::MessageClassification::Candidate(candidate) =
        message_worker::classify_gateway_message(
            message,
            db,
            config,
            &cdr_discord::interaction_access::InteractionAccessPolicy {
                allow_all_channels: true,
                ..Default::default()
            },
            None,
        )
        .unwrap()
    else {
        panic!("attachment candidate")
    };
    message_worker::admit_message_candidate_at(candidate, std::time::SystemTime::now())
        .unwrap()
        .unwrap()
}

async fn attachment_new(case: Case) {
    let ok = matches!(case, Case::Success);
    let temp = tempfile::tempdir().unwrap();
    let (fixture, remote, gate) = support::setup(&temp).await;
    let base_context = fixture.context(temp.path());
    let mut config = base_context.config.clone();
    if matches!(case, Case::Disabled) {
        config.attachments_enabled = false;
    }
    if matches!(case, Case::Oversized) {
        config.attachment_max_bytes = 1;
    }
    let context = message_worker::MessageContext {
        config: &config,
        ..base_context
    };
    gate.release.send(()).unwrap();
    message_worker::process_admitted_gateway_message(fixture.admit_id("!new", 800), &context)
        .await
        .unwrap();
    let (url, stop, download) = source(!matches!(case, Case::HttpError)).await;
    let db = fixture.executor.mirror_db();
    let admitted = admit_attachment(&url, db, context.config);
    let original = cdr_store::ingress::get(db, "message:801")
        .unwrap()
        .unwrap()
        .payload;
    if matches!(case, Case::CommitError) {
        rusqlite::Connection::open(db).unwrap().execute_batch(
            "CREATE TRIGGER reject_prepared_input BEFORE UPDATE OF outcome_json ON discord_ingress_journal
             WHEN NEW.ingress_id='message:801' AND json_extract(NEW.outcome_json,'$.new_input') IS NOT NULL
             BEGIN SELECT RAISE(ABORT,'fixture prepared input commit failure'); END;"
        ).unwrap();
    }
    let result = message_worker::process_admitted_gateway_message(admitted, &context).await;
    stop.send(()).unwrap();
    let downloads = download.await.unwrap();
    if ok {
        result.as_ref().unwrap();
        deliver_final(&fixture, &temp).await;
    }
    fixture.server.close().await.unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    let rpc = app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
    let expected = usize::from(ok);
    assert_eq!(
        downloads,
        usize::from(!matches!(case, Case::Disabled | Case::Oversized))
    );
    for method in ["thread/start", "turn/start"] {
        assert_eq!(
            rpc.iter().filter(|r| r["method"] == method).count(),
            expected
        );
    }
    assert_eq!(
        remote.creates.load(std::sync::atomic::Ordering::SeqCst),
        expected
    );
    assert_eq!(
        cdr_store::ingress::get(db, "message:801")
            .unwrap()
            .unwrap()
            .payload,
        original
    );
    if ok {
        assert_success(&temp, &rpc, &posts);
    } else {
        let error = result.unwrap_err().to_string();
        let expected = match case {
            Case::HttpError => "503",
            Case::Disabled => "attachments are disabled",
            Case::Oversized => "limit is 1 bytes",
            Case::CommitError => "fixture prepared input commit failure",
            Case::Success => unreachable!(),
        };
        assert!(
            error.contains(expected),
            "actual failure must be surfaced: {error}"
        );
        assert_eq!(posts.len(), 1);
        assert!(cdr_store::queue::list(db).unwrap().is_empty());
    }
}

#[tokio::test]
async fn bare_new_attachment_reaches_actual_rpc_and_exactly_one_final() {
    attachment_new(Case::Success).await;
}

#[tokio::test]
async fn bare_new_failed_attachment_starts_no_thread_or_turn() {
    attachment_new(Case::HttpError).await;
}

#[tokio::test]
async fn bare_new_disabled_attachment_starts_no_thread_or_turn() {
    attachment_new(Case::Disabled).await;
}

#[tokio::test]
async fn bare_new_oversized_attachment_starts_no_thread_or_turn() {
    attachment_new(Case::Oversized).await;
}

#[tokio::test]
async fn bare_new_prepared_input_commit_failure_starts_no_thread_or_turn() {
    attachment_new(Case::CommitError).await;
}
