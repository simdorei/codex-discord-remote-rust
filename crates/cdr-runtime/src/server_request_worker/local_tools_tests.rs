use std::{path::PathBuf, sync::Arc, time::Duration};

use cdr_app_server::{RequestId, ResidentAppServer, ServerRequest, ServerRequestOccurrence};
use cdr_store::queue;
use serde_json::{Value, json};

use super::{SentRequestCache, ServerRequestWorker, local_tools};

fn request() -> ServerRequest {
    ServerRequest {
        id: RequestId::String("approval-1".into()),
        occurrence: ServerRequestOccurrence::from_bytes([1; 16]),
        method: "mcpServer/elicitation/request".into(),
        params: json!({
            "threadId":"thread-b", "turnId":"turn-b", "serverName":"node_repl",
            "mode":"form", "message":"Allow local tool access?",
            "_meta": {
                "codex_approval_kind":"mcp_tool_call", "connector_id":"computer-use",
                "riskLevel":"low", "persist":["session","always"],
                "tool_params":{"app":"fixture-app"}
            },
            "requestedSchema":{"type":"object","properties":{}}
        }),
    }
}

#[test]
fn local_access_is_accepted_without_persistent_grants() {
    for connector in ["computer-use", "browser"] {
        for mode in ["form", "openai/form"] {
            let mut request = request();
            request.params["_meta"]["connector_id"] = json!(connector);
            request.params["mode"] = json!(mode);
            let (response, _) = local_tools::response(&request).unwrap();
            assert_eq!(
                response,
                json!({"action":"accept","content":{},"_meta":null})
            );
        }
    }
}

#[test]
fn command_file_permission_input_and_url_requests_keep_existing_handlers() {
    for method in [
        "item/commandExecution/requestApproval",
        "item/fileChange/requestApproval",
        "item/permissions/requestApproval",
        "item/tool/requestUserInput",
    ] {
        let mut request = request();
        request.method = method.into();
        assert!(local_tools::response(&request).is_none(), "{method}");
    }
    let mut request = request();
    request.params["mode"] = json!("url");
    assert!(local_tools::response(&request).is_none());
}

#[test]
fn unknown_server_connector_risk_and_approval_kind_are_not_accepted() {
    for (pointer, value) in [
        ("/serverName", json!("other-server")),
        ("/_meta/connector_id", json!("other-tool")),
        ("/_meta/connector_id", Value::Null),
        ("/_meta/riskLevel", json!("high")),
        ("/_meta/riskLevel", Value::Null),
        ("/_meta/codex_approval_kind", json!("permissions")),
    ] {
        let mut request = request();
        *request.params.pointer_mut(pointer).unwrap() = value;
        let (response, _) = local_tools::response(&request).unwrap();
        assert_eq!(
            response,
            json!({"action":"cancel","content":null,"_meta":null}),
            "{pointer}"
        );
    }
}

#[test]
fn data_entry_or_unrecognized_schema_constraints_are_never_auto_accepted() {
    for schema in [
        Value::Null,
        json!({"type":"string"}),
        json!({"type":"object","properties":{"password":{"type":"string"}}}),
        json!({"type":"object","properties":{},"required":["confirmation"]}),
        json!({"type":"object","properties":{},"required":null}),
        json!({"type":"object","properties":{},"minProperties":1}),
        json!({"type":"object","properties":{},"additionalProperties":{"type":"string"}}),
    ] {
        let mut request = request();
        request.params["requestedSchema"] = schema;
        assert_eq!(
            local_tools::response(&request).unwrap().0["action"],
            "cancel"
        );
    }
    let mut request = request();
    request.params["requestedSchema"]["required"] = json!([]);
    request.params["requestedSchema"]["additionalProperties"] = json!(false);
    assert_eq!(
        local_tools::response(&request).unwrap().0["action"],
        "accept"
    );
}

struct Fixture {
    temp: tempfile::TempDir,
    worker: ServerRequestWorker,
    request: ServerRequest,
    listener: std::net::TcpListener,
    log: PathBuf,
}

impl Fixture {
    async fn new(owner: Option<i64>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("rpc.jsonl");
        let mut config = crate::soak::native_fixture::config("approval");
        config.environment.insert(
            "CDR_APPROVAL_TEST_LOG".into(),
            log.to_string_lossy().into_owned(),
        );
        let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
        let db = temp.path().join("mirror.sqlite");
        cdr_store::mapping::upsert_thread(&db, "thread-b", "project", "title", 100, 42, 1.0)
            .unwrap();
        queue::enqueue(
            &db,
            queue::NewQueueJob {
                job_id: "local-tool-job",
                target_thread_id: "thread-b",
                channel_id: 42,
                owner_user_id: owner,
                discord_message_id: Some(101),
                app_server_generation: i64::try_from(server.generation()).unwrap(),
                prompt: "use the local tool",
                queued: false,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        cdr_store::schema::open_initialized(&db)
            .unwrap()
            .execute(
                "UPDATE codex_turn_queue SET state='running',turn_id='turn-b',attempt_count=1",
                [],
            )
            .unwrap();
        server
            .request(
                "test/pending",
                json!({"method":"mcpServer/elicitation/request","elicitationParams":request().params}),
                Duration::from_secs(2),
                None,
            )
            .await
            .unwrap();
        let request = server.pending_server_requests(None).await.unwrap()[0].clone();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let http = twilight_http::Client::builder()
            .proxy(format!("http://{}", listener.local_addr().unwrap()), true)
            .ratelimiter(None)
            .build();
        Self {
            temp,
            worker: ServerRequestWorker {
                server,
                mirror_db: db,
                http: Arc::new(http),
                sent: SentRequestCache::default(),
            },
            request,
            listener,
            log,
        }
    }
}

#[tokio::test]
async fn automatic_reply_uses_real_owner_generation_and_occurrence_guards() {
    for case in [
        "current",
        "unowned",
        "wrong-generation",
        "remapped",
        "expired",
    ] {
        let mut fixture = Fixture::new((case != "unowned").then_some(3)).await;
        let generation = fixture.worker.server.generation();
        if case == "wrong-generation" {
            cdr_store::schema::open_initialized(&fixture.worker.mirror_db)
                .unwrap()
                .execute("UPDATE codex_turn_queue SET app_server_generation=999", [])
                .unwrap();
        } else if case == "remapped" {
            cdr_store::mapping::upsert_thread(
                &fixture.worker.mirror_db,
                "thread-other",
                "project",
                "title",
                100,
                42,
                2.0,
            )
            .unwrap();
        } else if case == "expired" {
            fixture.request.occurrence = ServerRequestOccurrence::from_bytes([99; 16]);
        }
        let result = fixture
            .worker
            .process(generation, fixture.request.clone())
            .await;
        assert_eq!(result.is_ok(), case == "current", "{case}: {result:?}");
        let pending = fixture
            .worker
            .server
            .pending_server_requests(None)
            .await
            .unwrap();
        assert_eq!(pending.is_empty(), case == "current", "{case}");
        if case == "current" {
            assert!(
                fixture
                    .worker
                    .process(generation, fixture.request.clone())
                    .await
                    .is_err(),
                "an already answered occurrence must not be replayed"
            );
        }
        assert_eq!(
            fixture.listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "automatic reply must not post a Discord approval prompt"
        );
        fixture.worker.server.close().await.unwrap();
        let log = std::fs::read_to_string(&fixture.log).unwrap();
        let responses: Vec<Value> = log
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|message| message["id"] == "approval-1" && message.get("result").is_some())
            .collect();
        assert_eq!(responses.len(), usize::from(case == "current"), "{case}");
        if let Some(response) = responses.first() {
            assert_eq!(
                response["result"],
                json!({"action":"accept","content":{},"_meta":null})
            );
        }
        assert!(fixture.temp.path().exists());
    }
}
