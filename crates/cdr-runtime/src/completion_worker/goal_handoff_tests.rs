//! GH1: a held progress receipt must not prevent durable goal ownership handoff.
use super::*;
use cdr_app_server::AppServerConfig;
use cdr_store::{delivery_receipt, observed_completion, queue};
use sha2::{Digest, Sha256};

pub(super) async fn make_worker(temp: &tempfile::TempDir) -> CompletionWorker {
    let script = temp.path().join("goal-server.py");
    std::fs::write(&script, r"
import json, sys, time, pathlib
goal_done = False
early_goal = False
def advance_goal():
    global goal_done
    goal_done = True
    with open(sys.argv[2], 'w', encoding='utf-8') as rollout:
        rollout.write(json.dumps({'timestamp':'1','type':'event_msg','payload':{'type':'task_complete','turn_id':'T2','last_agent_message':'goal final'}}) + '\n')
    for method, status in [('turn/started','inProgress'),('turn/completed','completed')]:
        print(json.dumps({'method':method,'params':{'threadId':'thread','turn':{'id':'T2','status':status}}}),flush=True)
for line in sys.stdin:
    r = json.loads(line)
    if 'id' not in r: continue
    m = r['method']
    with open(sys.argv[1], 'a', encoding='utf-8') as log: log.write(m + '\n')
    if m == 'initialize': result = {'userAgent':'goal-test'}
    elif m == 'test/early-goal':
        early_goal = True
        result = {}
    elif m == 'test/advance-goal':
        advance_goal()
        result = {}
    elif m == 'thread/goal/get' and early_goal:
        early_goal = False
        advance_goal()
        deadline = time.monotonic() + 5
        while not pathlib.Path(sys.argv[2] + '.release').exists():
            if time.monotonic() > deadline: raise RuntimeError('test goal barrier timed out')
            time.sleep(0.005)
        result = {'goal':{'threadId':'thread','status':'active'}}
    elif m == 'thread/goal/get': result = {'goal':{'threadId':'thread','status':'complete' if goal_done or pathlib.Path(sys.argv[2] + '.goal-complete').exists() else 'active'}}
    elif m == 'thread/read':
        turns = [] if pathlib.Path(sys.argv[2] + '.omit-history').exists() else ([('T1','progress'),('T2','goal final')] if goal_done else [('T1','progress')])
        result = {'thread':{'id':'thread','turns':[{'id':turn,'status':'completed','items':[{'type':'agentMessage','text':text,'phase':'final_answer'}]} for turn,text in turns]}}
    else: result = {}
    print(json.dumps({'id':r['id'],'result':result}), flush=True)
").unwrap();
    #[cfg(windows)]
    let mut config = AppServerConfig::new(
        std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("py.exe"),
    );
    #[cfg(not(windows))]
    let mut config = AppServerConfig::new("python3");
    config.arguments = vec![
        script.to_string_lossy().into_owned(),
        temp.path()
            .join("goal-rpc.log")
            .to_string_lossy()
            .into_owned(),
        temp.path()
            .join("goal-rollout.jsonl")
            .to_string_lossy()
            .into_owned(),
    ];
    let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
    CompletionWorker {
        queue: Arc::new(QueueCoordinator::new(
            temp.path().join("mirror.sqlite"),
            Arc::new(AppServerTurnBackend::new(Arc::clone(&server))),
        )),
        server,
        http: Arc::new(
            Client::builder()
                .token("test-token".into())
                .proxy("127.0.0.1:1".into(), true)
                .ratelimiter(None)
                .build(),
        ),
        commentary_enabled: false,
        history_read_timeout: Duration::from_secs(2),
        commentary: Mutex::new(CommentaryBuffer::default()),
        terminal_fence: terminal_fence::TerminalFence::default(),
    }
}

pub(super) fn setup_running(worker: &CompletionWorker) {
    let db = worker.queue.db_path();
    let generation = i64::try_from(worker.server.generation()).unwrap();
    queue::enqueue(
        db,
        queue::NewQueueJob {
            job_id: "job",
            target_thread_id: "thread",
            channel_id: 42,
            owner_user_id: Some(1),
            discord_message_id: None,
            app_server_generation: generation,
            prompt: "input",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    queue::begin_attempt(db, "job", &[], generation).unwrap();
    queue::mark_running(db, "job", "T1", generation).unwrap();
    observed_completion::record(
        db,
        "thread",
        "T1",
        generation,
        r#"{"threadId":"thread","turn":{"id":"T1","status":"completed"}}"#,
    )
    .unwrap();
}

async fn held_progress_still_hands_off(blocked: bool) {
    let temp = tempfile::tempdir().unwrap();
    let worker = make_worker(&temp).await;
    setup_running(&worker);
    let db = worker.queue.db_path();
    // A prior attempt's durable outcome, at the same real sender identity.
    let key = serde_json::to_string(&(42_u64, "completion/goal-progress/v1", "6:thread;2:T1;", 0))
        .unwrap();
    let hash = hex::encode(Sha256::digest(b"[Goal progress]\nprogress"));
    delivery_receipt::begin(db, &key, &hash).unwrap();
    if blocked {
        delivery_receipt::block_rejected(db, &key, "403 Missing Permissions").unwrap();
    }
    let completion = TurnCompletion {
        thread_id: "thread".into(),
        turn_id: "T1".into(),
        status: TurnStatus::Completed,
        error_message: String::new(),
        interrupt_origin: None,
        duration_ms: None,
    };
    let result = worker
        .finish(
            worker.server.generation(),
            i64::try_from(worker.server.generation()).unwrap(),
            &completion,
        )
        .await;
    worker.server.close().await.unwrap();
    assert!(result.is_err(), "delivery failure must remain visible");
    let error = result.unwrap_err().to_string();
    assert!(
        error.contains(if blocked {
            "requires correction"
        } else {
            "outcome unknown"
        }),
        "{error}"
    );
    assert!(
        queue::list(db).unwrap()[0].goal_waiting,
        "GH1: progress failure must not prevent goal ownership handoff"
    );
    assert!(!observed_completion::contains(db, "thread", "T1").unwrap());
    assert_eq!(
        delivery_receipt::blocked_count(db).unwrap(),
        i64::from(blocked)
    );
    assert_eq!(
        delivery_receipt::unknown_count(db).unwrap(),
        i64::from(!blocked)
    );
    let pending = cdr_store::goal_progress::pending(db).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].content, "[Goal progress]\nprogress");
    assert!(!pending[0].last_error.is_empty());
    assert!(
        cdr_store::mirror::has_event(
            db,
            &cdr_store::mirror::turn_origin_marker("thread", "T1"),
            "thread"
        )
        .unwrap()
    );
    // Reconstruct both app-server and worker; no in-memory delivery state survives.
    let restarted = make_worker(&temp).await;
    assert!(
        restarted
            .finish(
                restarted.server.generation(),
                i64::try_from(restarted.server.generation()).unwrap(),
                &completion,
            )
            .await
            .is_err()
    );
    assert!(
        restarted
            .queue
            .goal_turn_started("thread", "T2")
            .await
            .unwrap()
    );
    for _ in 0..2 {
        assert!(restarted.recover_goal_progress().await.is_err());
    }
    assert_eq!(queue::list(db).unwrap()[0].turn_id.as_deref(), Some("T2"));
    assert!(!queue::list(db).unwrap()[0].goal_waiting);
    assert_eq!(cdr_store::goal_progress::pending(db).unwrap().len(), 1);
    assert_eq!(
        delivery_receipt::blocked_count(db).unwrap(),
        i64::from(blocked)
    );
    assert_eq!(
        delivery_receipt::unknown_count(db).unwrap(),
        i64::from(!blocked)
    );
    restarted.server.close().await.unwrap();
}

#[tokio::test]
async fn blocked_progress_does_not_block_goal_handoff() {
    held_progress_still_hands_off(true).await;
}

#[tokio::test]
async fn unknown_progress_does_not_block_goal_handoff() {
    held_progress_still_hands_off(false).await;
}
