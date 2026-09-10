use cdr_app_server::{AppServerConfig, ResidentAppServer};
use std::path::Path;

pub async fn start(temp: &tempfile::TempDir, log: &Path) -> ResidentAppServer {
    ResidentAppServer::start(config(temp, log)).await.unwrap()
}

pub fn config(temp: &tempfile::TempDir, log: &Path) -> AppServerConfig {
    let script = temp.path().join("approval-server.py");
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
    config.arguments.push(script.to_string_lossy().into());
    config.environment.insert("PYTHONUTF8".into(), "1".into());
    config
        .environment
        .insert("CDR_APPROVAL_TEST_LOG".into(), log.to_string_lossy().into());
    config
}

const SCRIPT: &str = r"
import json, os, sys
for line in sys.stdin:
    r = json.loads(line)
    with open(os.environ['CDR_APPROVAL_TEST_LOG'], 'a', encoding='utf-8') as log:
        log.write(json.dumps(r) + '\n')
    if 'id' not in r or 'method' not in r:
        continue
    method = r['method']
    if method == 'initialize':
        result = {'userAgent':'approval-test/1'}
    elif method == 'test/pending':
        p = r['params']
        print(json.dumps({'method':'turn/started','params':{'threadId':'thread-b','turn':{'id':'turn-b','status':'inProgress'}}}), flush=True)
        print(json.dumps({'id':p.get('requestId','approval-1'),'method':p.get('method','item/commandExecution/requestApproval'),
            'params':{'threadId':'thread-b','turnId':'turn-b','command':p.get('command','echo fixture'),
            'questions':p.get('questions',[{'id':'q','question':'Choose one','options':[{'label':'First'},{'label':'Second'}]}])}}), flush=True)
        result = {}
    elif method == 'test/finish':
        print(json.dumps({'method':'turn/completed','params':{'threadId':'thread-b','turn':{'id':'turn-b','status':'completed'}}}), flush=True)
        result = {}
    else:
        print(json.dumps({'id':r['id'],'error':{'code':-32601,'message':'unexpected test RPC'}}), flush=True)
        continue
    print(json.dumps({'id':r['id'],'result':result}), flush=True)
";
