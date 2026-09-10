use std::fs;
use std::path::{Path, PathBuf};

use cdr_app_server::{AppServerConfig, ResidentAppServer};
use serde_json::Value;

#[allow(dead_code)]
pub async fn start_fake_server(temp: &tempfile::TempDir, log: &Path) -> ResidentAppServer {
    start_server(temp, log, None).await
}

#[allow(dead_code)]
pub async fn start_persisting_server(
    temp: &tempfile::TempDir,
    log: &Path,
    state: &Path,
) -> ResidentAppServer {
    start_server(temp, log, Some(state)).await
}

async fn start_server(
    temp: &tempfile::TempDir,
    log: &Path,
    state: Option<&Path>,
) -> ResidentAppServer {
    let script = temp.path().join("fake_app_server.py");
    fs::write(&script, FAKE_APP_SERVER).unwrap();
    let mut config = AppServerConfig::new(python_executable());
    config.arguments = python_arguments(&script);
    config.environment.insert("PYTHONUTF8".into(), "1".into());
    if let Some(state) = state {
        config.environment.insert(
            "CDR_NEW_STATE_FIXTURE".into(),
            state.to_string_lossy().into_owned(),
        );
    }
    config.environment.insert(
        "CDR_ACTION_RPC_LOG".into(),
        log.to_string_lossy().into_owned(),
    );
    ResidentAppServer::start(config).await.unwrap()
}

pub fn rpc_log(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[cfg(windows)]
fn python_executable() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("py.exe")
}

#[cfg(not(windows))]
fn python_executable() -> PathBuf {
    PathBuf::from("python3")
}

#[cfg(windows)]
fn python_arguments(script: &Path) -> Vec<String> {
    vec!["-3".into(), script.to_string_lossy().into_owned()]
}

#[cfg(not(windows))]
fn python_arguments(script: &Path) -> Vec<String> {
    vec![script.to_string_lossy().into_owned()]
}

const FAKE_APP_SERVER: &str = r#"
import json, os, sys, sqlite3
log_path = os.environ["CDR_ACTION_RPC_LOG"]
state_path = os.environ.get("CDR_NEW_STATE_FIXTURE")
new_cwd = None
new_turn = None
persisted_cwd_override = None
stale_steer = False
recovery_mode = False
drop_new_response = False
hide_new_history = False
for line in sys.stdin:
    request = json.loads(line)
    if "id" not in request:
        continue
    with open(log_path, "a", encoding="utf-8") as log:
        log.write(json.dumps(request) + "\n")
    method = request.get("method")
    thread_id = request.get("params", {}).get("threadId", "")
    if method == "initialize":
        response = {"id": request["id"], "result": {"userAgent": "action-test/1"}}
    elif method == "test/new-recovery-mode":
        recovery_mode = True
        drop_new_response = request["params"].get("drop_response", False)
        hide_new_history = request["params"].get("hide_history", False)
        if new_turn is None:
            with open(log_path, encoding="utf-8") as log:
                accepted = [json.loads(line) for line in log if 'fixture/turn-accepted' in line]
            if accepted:
                new_turn = accepted[-1]["turn"]
                new_cwd = accepted[-1]["cwd"]
        response = {"id":request["id"],"result":{}}
    elif method == "test/persist-new-in-other-project":
        persisted_cwd_override = request["params"]["cwd"]
        response = {"id":request["id"],"result":{}}
    elif method == "test/active-turn":
        turn_id = request["params"]["turnId"]
        print(json.dumps({"method":"turn/started","params":{"threadId":thread_id,"turn":{"id":turn_id,"status":"inProgress"}}}), flush=True)
        response = {"id":request["id"],"result":{}}
    elif method == "test/stale-next-steer":
        stale_steer = True
        response = {"id":request["id"],"result":{}}
    elif method == "test/finish-turn":
        print(json.dumps({"method":"turn/completed","params":{"threadId":thread_id,"turn":{"id":request["params"]["turnId"],"status":"completed"}}}), flush=True)
        response = {"id":request["id"],"result":{}}
    elif method == "turn/steer" and stale_steer:
        stale_steer = False
        print(json.dumps({"method":"turn/completed","params":{"threadId":thread_id,"turn":{"id":"current","status":"completed"}}}), flush=True)
        print(json.dumps({"method":"turn/started","params":{"threadId":thread_id,"turn":{"id":"next","status":"inProgress"}}}), flush=True)
        response = {"id":request["id"],"error":{"code":-32600,"message":"stale expectedTurnId: current; active turn is next"}}
    elif method == "thread/start":
        new_cwd = request.get("params", {}).get("cwd")
        response = {"id": request["id"], "result": {"thread": {"id": "new-thread", "turns": []}}}
    elif method == "model/list":
        response = {"id": request["id"], "result": {"data": [
            {"model": "gpt-6-astra", "displayName": "GPT-6-Astra", "supportedReasoningEfforts": [{"reasoningEffort": "max"}]},
            {"model": "gpt-5.6-sol"}, {"model": "model-a"}
        ]}}
    elif method in ("thread/resume", "thread/read") and thread_id == "new-thread":
        if recovery_mode:
            response = {"id":request["id"],"result":{"thread":{"id":"new-thread","cwd":new_cwd,"turns":[] if hide_new_history or not new_turn else [new_turn]}}}
        elif state_path and new_turn:
            response = {"id":request["id"],"result":{"thread":{"id":"new-thread","cwd":new_cwd,"turns":[new_turn]}}}
        else:
            response = {"id": request["id"], "error": {"code": -32600, "message": "no rollout found for thread id new-thread"}}
    elif method == "turn/start" and thread_id == "new-thread":
        if recovery_mode:
            new_turn = {"id":"first-turn","status":"inProgress","items":[{"id":"first-user","type":"userMessage","content":request["params"]["input"]}]}
            with open(log_path,"a",encoding="utf-8") as log:
                log.write(json.dumps({"method":"fixture/turn-accepted","turn":new_turn,"cwd":new_cwd})+"\n")
        if state_path:
            new_turn = {"id":"first-turn","status":"inProgress","items":[{"id":"first-user","type":"userMessage","content":request["params"]["input"]}]}
            rollout = os.path.join(os.path.dirname(state_path), "new-rollout.jsonl")
            with open(rollout,"w",encoding="utf-8") as saved:
                saved.write(json.dumps({"type":"session_meta","payload":{"id":"new-thread","cwd":new_cwd}},ensure_ascii=False)+"\n")
                saved.write(json.dumps({"type":"turn_context","payload":{"turn_id":"first-turn"}})+"\n")
                saved.write(json.dumps({"type":"response_item","payload":{"type":"message","role":"user","content":request["params"]["input"]}},ensure_ascii=False)+"\n")
            with sqlite3.connect(state_path) as state:
                state.execute("INSERT INTO threads VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",("new-thread","new",persisted_cwd_override or new_cwd,50,rollout,"model-a","high",0,0,0,"app-server","user"))
        if drop_new_response:
            continue
        response = {"id": request["id"], "result": {"turn": {"id": "first-turn", "status": "inProgress"}}}
    elif method == "thread/resume" and thread_id == "thread-a":
        response = {"id": request["id"], "error": {"code": -32600, "message": "thread thread-a already has an active writer"}}
    elif method in ("thread/resume", "thread/read"):
        response = {"id": request["id"], "result": {"thread": {"id": thread_id, "turns": []}}}
    elif method == "turn/start":
        response = {"id": request["id"], "result": {"turn": {"id": "existing-turn", "status": "inProgress"}}}
    else:
        response = {"id": request["id"], "result": {}}
    print(json.dumps(response), flush=True)
"#;
