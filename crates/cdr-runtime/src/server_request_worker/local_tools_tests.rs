use std::{path::PathBuf, sync::Arc, time::Duration};

use cdr_app_server::{RequestId, ResidentAppServer, ServerRequest, ServerRequestOccurrence};
use cdr_store::queue;
use serde_json::{Value, json};

use super::{SentRequestCache, ServerRequestWorker, local_tools};

mod all_policy;

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
fn high_risk_local_access_waits_for_bound_user_approval() {
    for connector in ["computer-use", "browser"] {
        for mode in ["form", "openai/form"] {
            let mut request = request();
            request.params["mode"] = json!(mode);
            request.params["message"] = json!("Allow Codex to use Google Chrome?");
            request.params["_meta"]["connector_id"] = json!(connector);
            request.params["_meta"]["riskLevel"] = json!("high");
            request.params["_meta"]["persist"] = json!(["session"]);
            request.params["_meta"]["tool_params"]["app"] = json!("Chrome");

            assert!(
                local_tools::response(&request).is_none(),
                "a recognized high-risk approval must reach the user, not be answered automatically"
            );
            let prompt = crate::server_prompt::build_server_prompt(&request, 7).unwrap();
            assert!(prompt.text.contains("Allow Codex to use Google Chrome?"));
            let row = serde_json::to_value(&prompt.components[0]).unwrap();
            let button = row["components"][0]["custom_id"].as_str().unwrap();
            let component = cdr_discord::components::parse_component_id(button).unwrap();
            let response = crate::component_worker::build_component_response(
                &component,
                std::slice::from_ref(&request),
                7,
            )
            .unwrap();
            assert_eq!(response.request_id, request.id);
            assert_eq!(response.occurrence, request.occurrence);
            assert_eq!(response.generation, 7);
            assert_eq!(
                response.payload,
                json!({"action":"accept","content":{},"_meta":null})
            );
            assert!(
                crate::component_worker::build_component_response(
                    &component,
                    std::slice::from_ref(&request),
                    8,
                )
                .is_err(),
                "an approval button must not survive a server generation change"
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
        ("/_meta/riskLevel", json!("unknown")),
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
        if pointer != "/_meta/riskLevel" {
            request.params["_meta"]["riskLevel"] = json!("high");
            assert_eq!(
                local_tools::response(&request).unwrap().0["action"],
                "cancel"
            );
            assert!(crate::server_prompt::build_server_prompt(&request, 1).is_err());
        }
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
        request.params["_meta"]["riskLevel"] = json!("high");
        assert_eq!(
            local_tools::response(&request).unwrap().0["action"],
            "cancel"
        );
        assert!(crate::server_prompt::build_server_prompt(&request, 1).is_err());
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
        Self::with_request(owner, request()).await
    }

    async fn with_request(owner: Option<i64>, approval: ServerRequest) -> Self {
        Self::with_config(owner, approval, json!({})).await
    }

    async fn with_config(owner: Option<i64>, approval: ServerRequest, app_config: Value) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("rpc.jsonl");
        let mut config = crate::soak::native_fixture::config("approval");
        config
            .environment
            .insert("CDR_APPROVAL_TEST_CONFIG".into(), app_config.to_string());
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
                json!({"method":approval.method,"elicitationParams":approval.params}),
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
                local_tool_approval_policy: crate::config::LocalToolApprovalPolicy::Ask,
                approval_policy: crate::config::ApprovalPolicy::Ask,
            },
            request,
            listener,
            log,
        }
    }
}

#[tokio::test]
async fn saved_chrome_access_uses_its_bound_response_without_a_prompt() {
    let mut approval = request();
    approval.params["_meta"]["riskLevel"] = json!("high");
    approval.params["_meta"]["tool_params"]["app"] = json!("Chrome");
    let config = json!({"layers":[{"name":{"type":"user","file":"C:/Users/test/.codex/config.toml"},
        "config":{"computer_use":{"windows":{"always_allowed_app_ids":["Chrome"]}}}}]});
    let mut fixture = Fixture::with_config(Some(3), approval, config).await;
    let generation = fixture.worker.server.generation();
    let outcome = tokio::time::timeout(
        Duration::from_secs(1),
        fixture.worker.process(generation, fixture.request.clone()),
    )
    .await;
    let pending = fixture
        .worker
        .server
        .pending_server_requests(None)
        .await
        .unwrap();
    fixture.worker.server.close().await.unwrap();
    assert!(
        matches!(outcome, Ok(Ok(()))),
        "saved Chrome access must not wait on a Discord prompt: {outcome:?}"
    );
    assert!(pending.is_empty());
    assert_eq!(
        fixture.listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let log = std::fs::read_to_string(&fixture.log).unwrap();
    let responses: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|message| message["id"] == "approval-1" && message.get("result").is_some())
        .collect();
    assert_eq!(responses.len(), 1);
    assert_eq!(
        responses[0]["result"],
        json!({"action":"accept","content":{},"_meta":{"persist":"always"}})
    );
}

#[tokio::test]
async fn high_risk_local_access_is_delivered_and_stays_pending_until_the_owner_replies() {
    let mut approval = request();
    approval.params["message"] = json!("Allow Codex to use Google Chrome?");
    approval.params["_meta"]["riskLevel"] = json!("high");
    let mut fixture = Fixture::with_request(Some(3), approval).await;
    let generation = fixture.worker.server.generation();
    let gate = crate::test_support::http_gate::start().await;
    fixture.worker.http = Arc::new(
        twilight_http::Client::builder()
            .proxy(gate.address.clone(), true)
            .ratelimiter(None)
            .build(),
    );
    gate.release.send(()).unwrap();
    fixture
        .worker
        .process(generation, fixture.request.clone())
        .await
        .unwrap();
    let pending = fixture
        .worker
        .server
        .pending_server_requests(None)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1, "delivery must not grant or cancel access");
    tokio::time::timeout(Duration::from_secs(2), gate.entered)
        .await
        .unwrap()
        .unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert_eq!(posts.len(), 1);
    assert!(
        posts[0]["content"]
            .as_str()
            .unwrap()
            .contains("Google Chrome")
    );
    let component = cdr_discord::components::parse_component_id(
        posts[0]["components"][0]["components"][0]["custom_id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let authority = crate::server_prompt_authority::verify(
        &fixture.worker.mirror_db,
        &fixture.worker.server,
        &pending[0],
        generation,
    )
    .await
    .unwrap();
    assert!(authority.require_actor(42, 4).is_err());
    authority.require_actor(42, 3).unwrap();
    let response =
        crate::component_worker::build_component_response(&component, &pending, generation)
            .unwrap();
    fixture
        .worker
        .server
        .respond_current(
            &response.request_id,
            response.occurrence,
            response.payload,
            response.generation,
        )
        .await
        .unwrap();
    assert!(
        fixture
            .worker
            .server
            .pending_server_requests(None)
            .await
            .unwrap()
            .is_empty()
    );
    fixture.worker.server.close().await.unwrap();
    let log = std::fs::read_to_string(&fixture.log).unwrap();
    let responses: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|message| message["id"] == "approval-1" && message.get("result").is_some())
        .collect();
    assert_eq!(responses.len(), 1);
    assert_eq!(
        responses[0]["result"],
        json!({"action":"accept","content":{},"_meta":null})
    );
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

#[test]
fn saved_app_grants_require_exact_user_config_and_native_persistence_support() {
    let mut approval = request();
    approval.params["_meta"]["riskLevel"] = json!("high");
    approval.params["_meta"]["tool_params"]["app"] = json!("Chrome");
    let user = json!({"name":{"type":"user"},"config":{"computer_use":{"windows":{"always_allowed_app_ids":["Chrome"]}}}});
    let config = json!({"layers":[user.clone()]});
    assert!(cdr_app_server::requests::is_observational("config/read"));
    assert!(local_tools::saved_access_response(&approval, &config).is_some());
    for (pointer, value) in [
        ("/layers/0/name/type", json!("project")),
        (
            "/layers/0/config/computer_use/windows/always_allowed_app_ids",
            json!([]),
        ),
        (
            "/layers/0/config/computer_use/windows/always_allowed_app_ids",
            json!(["Other"]),
        ),
        (
            "/layers/0/config/computer_use/windows/always_allowed_app_ids",
            json!(["Chrome", 1]),
        ),
    ] {
        let mut invalid = config.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert!(
            local_tools::saved_access_response(&approval, &invalid).is_none(),
            "{pointer}"
        );
    }
    let mut disabled = config.clone();
    disabled["layers"][0]["disabledReason"] = json!("untrusted");
    let mut restricted = config.clone();
    restricted["layers"][0]["config"]["computer_use"]["allow_persistent_approval"] = json!(false);
    for invalid in [
        json!({}),
        json!({"layers":[user.clone(),user]}),
        disabled,
        restricted,
    ] {
        assert!(local_tools::saved_access_response(&approval, &invalid).is_none());
    }
    for (pointer, value) in [
        ("/_meta/persist", json!(["session"])),
        ("/_meta/tool_params/app", json!("computer-audio")),
        ("/_meta/tool_params/app", json!("Other")),
        (
            "/_meta/tool_params",
            json!({"app":"Chrome","command":"other"}),
        ),
        ("/_meta/connector_id", json!("browser")),
        ("/_meta/riskLevel", json!("unknown")),
        (
            "/requestedSchema/properties",
            json!({"secret":{"type":"string"}}),
        ),
    ] {
        let mut invalid = approval.clone();
        *invalid.params.pointer_mut(pointer).unwrap() = value;
        assert!(
            local_tools::saved_access_response(&invalid, &config).is_none(),
            "{pointer}"
        );
    }
    for key in ["tool_name", "codex_request_type"] {
        let mut invalid = approval.clone();
        invalid.params["_meta"][key] = json!("other");
        assert!(local_tools::saved_access_response(&invalid, &config).is_none());
    }
}

fn allow_policy() -> crate::config::LocalToolApprovalPolicy {
    crate::config::RuntimeConfig::from_map(
        &std::collections::BTreeMap::from([
            ("DISCORD_BOT_TOKEN".into(), "fixture".into()),
            ("DISCORD_ALLOW_ALL_CHANNELS".into(), "1".into()),
            ("DISCORD_LOCAL_TOOL_APPROVAL_POLICY".into(), "allow".into()),
        ]),
        crate::config::CliOptions::default(),
    )
    .unwrap()
    .local_tool_approval_policy
}

#[test]
fn allow_policy_accepts_all_known_local_approvals_without_persistence() {
    for connector in ["computer-use", "browser"] {
        for risk in ["low", "high"] {
            for mode in ["form", "openai/form"] {
                let mut approval = request();
                approval.params["mode"] = json!(mode);
                approval.params["_meta"]["connector_id"] = json!(connector);
                approval.params["_meta"]["riskLevel"] = json!(risk);
                approval.params["_meta"]["persist"] = json!(["session"]);
                assert_eq!(
                    local_tools::policy_response(&approval, allow_policy())
                        .unwrap()
                        .0,
                    json!({"action":"accept","content":{},"_meta":null})
                );
                assert!(
                    local_tools::policy_response(
                        &approval,
                        crate::config::LocalToolApprovalPolicy::Ask
                    )
                    .is_none()
                );
            }
        }
    }
}

#[test]
fn allow_policy_preserves_unrecognized_forms_and_other_request_handlers() {
    for (pointer, value) in [
        ("/serverName", json!("unrecognized")),
        ("/mode", json!("url")),
        ("/_meta/connector_id", json!("other")),
        ("/_meta/riskLevel", json!("unknown")),
        ("/_meta/codex_approval_kind", json!("other")),
        (
            "/requestedSchema",
            json!({"type":"object","properties":{"answer":{"type":"string"}}}),
        ),
    ] {
        let mut approval = request();
        *approval.params.pointer_mut(pointer).unwrap() = value;
        assert!(
            local_tools::policy_response(&approval, allow_policy()).is_none(),
            "{pointer}"
        );
    }
    for method in [
        "item/commandExecution/requestApproval",
        "item/fileChange/requestApproval",
        "item/permissions/requestApproval",
        "item/tool/requestUserInput",
    ] {
        let mut approval = request();
        approval.method = method.into();
        assert!(local_tools::policy_response(&approval, allow_policy()).is_none());
    }
}

#[tokio::test]
async fn allow_policy_replies_to_high_risk_without_saved_app_or_discord_prompt() {
    for connector in ["computer-use", "browser"] {
        let mut approval = request();
        approval.params["_meta"]["connector_id"] = json!(connector);
        approval.params["_meta"]["riskLevel"] = json!("high");
        approval.params["_meta"]
            .as_object_mut()
            .unwrap()
            .remove("persist");
        let mut fixture = Fixture::with_request(Some(3), approval).await;
        fixture.worker.local_tool_approval_policy = allow_policy();
        let generation = fixture.worker.server.generation();
        let outcome = tokio::time::timeout(
            Duration::from_secs(1),
            fixture.worker.process(generation, fixture.request.clone()),
        )
        .await;
        let pending = fixture
            .worker
            .server
            .pending_server_requests(None)
            .await
            .unwrap();
        fixture.worker.server.close().await.unwrap();
        assert!(
            matches!(outcome, Ok(Ok(()))),
            "allow policy must answer without HTTP: {outcome:?}"
        );
        assert!(pending.is_empty());
        assert_eq!(
            fixture.listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        let log = std::fs::read_to_string(&fixture.log).unwrap();
        let responses: Vec<Value> = log
            .lines()
            .map(|s| serde_json::from_str::<Value>(s).unwrap())
            .filter(|v| v["id"] == "approval-1" && v.get("result").is_some())
            .collect();
        assert_eq!(responses.len(), 1);
        assert_eq!(
            responses[0]["result"],
            json!({"action":"accept","content":{},"_meta":null})
        );
        assert!(
            !log.contains("config/read"),
            "explicit all-request policy is independent of saved grants"
        );
    }
}

#[tokio::test]
async fn allow_policy_still_requires_original_authority() {
    for case in [
        "unowned",
        "wrong-generation",
        "remapped",
        "expired",
        "finished",
    ] {
        let mut approval = request();
        approval.params["_meta"]["riskLevel"] = json!("high");
        let mut fixture = Fixture::with_request((case != "unowned").then_some(3), approval).await;
        fixture.worker.local_tool_approval_policy = allow_policy();
        let generation = fixture.worker.server.generation();
        if case == "wrong-generation" {
            cdr_store::schema::open_initialized(&fixture.worker.mirror_db)
                .unwrap()
                .execute("UPDATE codex_turn_queue SET app_server_generation=999", [])
                .unwrap();
        } else if case == "remapped" {
            cdr_store::mapping::upsert_thread(
                &fixture.worker.mirror_db,
                "other",
                "project",
                "title",
                100,
                42,
                2.0,
            )
            .unwrap();
        } else if case == "expired" {
            fixture.request.occurrence = ServerRequestOccurrence::from_bytes([99; 16]);
        } else if case == "finished" {
            fixture
                .worker
                .server
                .request("test/finish", json!({}), Duration::from_secs(2), None)
                .await
                .unwrap();
        }
        assert!(
            fixture
                .worker
                .process(generation, fixture.request.clone())
                .await
                .is_err(),
            "{case}"
        );
        fixture.worker.server.close().await.unwrap();
        let log = std::fs::read_to_string(&fixture.log).unwrap();
        assert!(
            !log.lines()
                .map(|s| serde_json::from_str::<Value>(s).unwrap())
                .any(|v| v["id"] == "approval-1" && v.get("result").is_some()),
            "{case}"
        );
        assert_eq!(
            fixture.listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}
