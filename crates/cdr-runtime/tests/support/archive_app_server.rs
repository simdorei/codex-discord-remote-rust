use cdr_app_server::{AppServerConfig, ResidentAppServer};
use std::path::Path;

pub async fn start(temp: &tempfile::TempDir, state: &Path, scenario: &str) -> ResidentAppServer {
    let script = temp.path().join("archive_fixture.py");
    std::fs::write(&script, SCRIPT).unwrap();
    let mut config = AppServerConfig::new(if cfg!(windows) {
        "C:/Windows/py.exe"
    } else {
        "python3"
    });
    config.arguments = if cfg!(windows) {
        vec!["-3".into()]
    } else {
        vec![]
    };
    config.arguments.push(script.to_string_lossy().into_owned());
    config.environment.insert("PYTHONUTF8".into(), "1".into());
    config
        .environment
        .insert("ARCHIVE_STATE".into(), state.to_string_lossy().into_owned());
    config.environment.insert(
        "ARCHIVE_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into_owned(),
    );
    config
        .environment
        .insert("ARCHIVE_SCENARIO".into(), scenario.into());
    ResidentAppServer::start(config).await.unwrap()
}

pub fn calls(temp: &tempfile::TempDir) -> Vec<serde_json::Value> {
    std::fs::read_to_string(temp.path().join("rpc.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

const SCRIPT: &str = r#"
import json, os, sys, sqlite3, time
scenario = os.environ['ARCHIVE_SCENARIO']
lists = 0
for line in sys.stdin:
    req=json.loads(line)
    if 'id' not in req: continue
    with open(os.environ['ARCHIVE_LOG'],'a',encoding='utf-8') as log: log.write(json.dumps(req)+'\n')
    method=req['method']
    thread=req.get('params',{}).get('threadId','thread-b')
    result={}
    if method=='initialize': result={'userAgent':'archive-test/1'}
    elif method=='thread/resume':
        if scenario=='descendant_writer' and thread=='child':
            print(json.dumps({'id':req['id'],'error':{'code':-32600,'message':'thread child already has an active writer'}}),flush=True)
            continue
        if scenario=='became_active':
            print(json.dumps({'method':'turn/started','params':{'threadId':'thread-b','turn':{'id':'late-turn','status':'inProgress'}}}),flush=True)
        result={'thread':{'id':thread,'status':{'type':'active' if scenario=='became_active' else 'idle'},'turns':[]}}
        if scenario in ('conflicting_resume', 'missing_nested_resume'):
            result['threadId'] = thread
            if scenario == 'conflicting_resume': result['thread']['id'] = 'other'
            else: del result['thread']['id']
    elif method=='thread/read':
        result={'thread':{'id':thread,'status':{'type':'active' if scenario=='active_without_event' else 'idle'},'turns':[]}}
        if scenario=='missing_status': del result['thread']['status']
        if scenario=='wrong_read_identity': result['thread']['id']='other'
        if scenario in ('conflicting_read', 'missing_nested_read'):
            result['threadId'] = thread
            if scenario == 'conflicting_read': result['thread']['id'] = 'other'
            else: del result['thread']['id']
    elif method=='thread/list':
        lists += 1
        if scenario=='stall_list': time.sleep(5)
        result={'data':[{'id':'child'}] if scenario.startswith('descendant_') else [],'nextCursor':None}
        if scenario=='scope_malformed': result={'data':{}}
        if scenario=='descendant_changed' and lists>1: result['data']=[]
        if scenario=='scope_cursor_repeat': result['nextCursor']='same'
        if scenario=='scope_missing_cursor': del result['nextCursor']
        if scenario=='scope_root': result['data']=[{'id':'thread-b'}]
        if scenario=='writer_gate' and lists==2:
            deadline = time.monotonic() + 8
            while not os.path.exists(os.environ['ARCHIVE_LOG']+'.list_release') and time.monotonic()<deadline: time.sleep(.01)
    elif method=='thread/archive' and scenario=='archive_writer_reject':
        print(json.dumps({'id':req['id'],'error':{'code':-32600,'message':'thread thread-b already has an active writer'}}),flush=True)
        continue
    elif method=='thread/archive' and scenario!='no_persistence':
        if scenario in ('archive_gate', 'descendant_gate'):
            gate = os.environ['ARCHIVE_LOG'] + '.release'
            deadline = time.monotonic() + 8
            while not os.path.exists(gate) and time.monotonic() < deadline: time.sleep(.01)
        with sqlite3.connect(os.environ['ARCHIVE_STATE']) as db:
            db.execute("UPDATE threads SET archived=1, archived_at=60 WHERE id=?", (thread,))
            if scenario in ('descendant_normal', 'descendant_gate'): db.execute("UPDATE threads SET archived=1, archived_at=60 WHERE id='child'")
        if scenario=='stall_archive': time.sleep(5)
        if scenario=='stall_archive_inner': time.sleep(12)
        if scenario=='archive_disconnect': sys.exit(0)
    print(json.dumps({'id':req['id'],'result':result}),flush=True)
    if scenario=='writer_gate' and method=='thread/list' and lists==2:
        deadline = time.monotonic() + 8
        while not os.path.exists(os.environ['ARCHIVE_LOG']+'.writer_release') and time.monotonic()<deadline: time.sleep(.01)
"#;
