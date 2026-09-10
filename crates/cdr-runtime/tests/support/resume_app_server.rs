use cdr_app_server::{AppServerConfig, ResidentAppServer};
use std::path::Path;

pub async fn start(temp: &tempfile::TempDir, log: &Path, scenario: &str) -> ResidentAppServer {
    let script = temp.path().join("resume_fixture.py");
    std::fs::write(&script, SCRIPT).unwrap();
    let mut config = if cfg!(windows) {
        let mut config = AppServerConfig::new("C:/Windows/py.exe");
        config.arguments = vec!["-3".into()];
        config
    } else {
        let mut config = AppServerConfig::new("python3");
        config.arguments.clear();
        config
    };
    config.arguments.push(script.to_string_lossy().into_owned());
    config.environment.insert("PYTHONUTF8".into(), "1".into());
    config
        .environment
        .insert("RESUME_SCENARIO".into(), scenario.into());
    config
        .environment
        .insert("RESUME_LOG".into(), log.to_string_lossy().into_owned());
    ResidentAppServer::start(config).await.unwrap()
}

pub fn calls(log: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

const SCRIPT: &str = r"
import json, os, sys, time
scenario = os.environ['RESUME_SCENARIO']
resumed = False
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    with open(os.environ['RESUME_LOG'],'a',encoding='utf-8') as log:
        log.write(json.dumps(req)+'\n')
    method = req['method']
    result = {}
    if method == 'initialize':
        result = {'userAgent':'resume-contract/1'}
    elif method == 'thread/resume':
        resumed = True
        if scenario == 'writer':
            print(json.dumps({'id':req['id'],'error':{'code':-32600,'message':'thread thread-b already has an active writer'}}),flush=True)
            continue
        result = {'thread':{'id':'other' if scenario == 'wrong_resume' else 'thread-b'}}
        if scenario in ('conflicting_resume', 'missing_nested_resume'):
            result['threadId'] = 'thread-b'
            if scenario == 'conflicting_resume': result['thread']['id'] = 'other'
            else: del result['thread']['id']
    elif method == 'thread/read':
        if scenario == 'slow_read': time.sleep(5)
        if scenario == 'gated_read':
            while not os.path.exists(os.environ['RESUME_LOG']+'.release'): time.sleep(0.005)
        status = 'notLoaded'
        if scenario == 'gated_read': status = 'idle'
        if scenario in ('idle','active','systemError'): status = scenario
        elif resumed and scenario != 'still_unloaded': status = 'idle'
        result = {'thread':{'id':'other' if scenario == 'wrong_read' else 'thread-b','status':{'type':status}}}
        if scenario == 'missing_status': del result['thread']['status']
        if scenario == 'unknown_status': result['thread']['status'] = {'type':'futureUnknown'}
        if scenario in ('conflicting_read', 'missing_nested_read'):
            result['threadId'] = 'thread-b'
            if scenario == 'conflicting_read': result['thread']['id'] = 'other'
            else: del result['thread']['id']
    print(json.dumps({'id':req['id'],'result':result}),flush=True)
";
