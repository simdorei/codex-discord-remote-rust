use cdr_app_server::{AppServerConfig, ResidentAppServer};
use std::path::Path;

pub async fn start(root: &Path, mode: &str) -> ResidentAppServer {
    let script = root.join("display_fixture.py");
    std::fs::write(&script, SCRIPT).unwrap();
    #[cfg(windows)]
    let mut config = AppServerConfig::new(
        std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("py.exe"),
    );
    #[cfg(not(windows))]
    let mut config = AppServerConfig::new("python3");
    config.arguments.clear();
    #[cfg(windows)]
    config.arguments.push("-3".into());
    config.arguments.push(script.to_string_lossy().into_owned());
    config.environment.insert("PYTHONUTF8".into(), "1".into());
    config
        .environment
        .insert("DISPLAY_MODE".into(), mode.into());
    config.environment.insert(
        "DISPLAY_LOG".into(),
        root.join("display-rpc.jsonl")
            .to_string_lossy()
            .into_owned(),
    );
    ResidentAppServer::start(config).await.unwrap()
}

const SCRIPT: &str = r"
import json, os, sys
mode = os.environ['DISPLAY_MODE']
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    method = req.get('method')
    with open(os.environ['DISPLAY_LOG'], 'a', encoding='utf-8') as log:
        log.write(json.dumps(req) + '\n')
    result = {}
    error = None
    if method == 'initialize': result = {'userAgent':'display-fixture'}
    elif method == 'thread/read':
        thread = req['params']['threadId']
        result = {'thread': {'id':'wrong' if mode == 'wrong_id' else thread, 'status':{'type':'idle'}}}
        if mode == 'conflicting_id':
            result['threadId'] = thread
            result['thread']['id'] = 'wrong'
        if mode == 'missing_nested_id':
            result['conversationId'] = thread
            del result['thread']['id']
    elif method == 'thread/goal/get':
        if mode == 'goal_hang': continue
        if mode == 'goal_error': error = {'code':-32001,'message':'fixture goal lookup failed'}
        elif mode == 'goal_missing': result = {}
        else: result = {'goal':None}
    else: error = {'code':-32601,'message':'unexpected mutating method in display fixture'}
    print(json.dumps({'id':req['id'], **({'error':error} if error else {'result':result})}), flush=True)
";
