use cdr_app_server::{AppServerConfig, ResidentAppServer};
use std::path::Path;

pub async fn start(root: &Path, log: &Path, mode: &str) -> ResidentAppServer {
    let script = root.join("usage_fixture.py");
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
        .insert("USAGE_TEST_MODE".into(), mode.into());
    config
        .environment
        .insert("USAGE_TEST_LOG".into(), log.to_string_lossy().into_owned());
    ResidentAppServer::start(config).await.unwrap()
}

const SCRIPT: &str = r"
import json, os, sys, datetime
mode = os.environ['USAGE_TEST_MODE']
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    method = req.get('method')
    with open(os.environ['USAGE_TEST_LOG'], 'a', encoding='utf-8') as log:
        log.write(json.dumps({'method': method}) + '\n')
    result = {}
    error = None
    if method == 'initialize': result = {'userAgent':'usage-fixture'}
    elif method == 'account/rateLimits/read':
        if mode == 'rates_error': error = {'code':-32000,'message':'fixture rate lookup failed'}
        else: result = {'rateLimits':{'planType':'pro','primary':{'usedPercent':25,'windowDurationMins':300}}}
    elif method == 'account/usage/read':
        if mode == 'usage_error': error = {'code':-32001,'message':'fixture usage lookup failed'}
        elif mode == 'empty': result = {'dailyUsageBuckets':[]}
        else:
            today = datetime.datetime.now(datetime.timezone.utc).date()
            result = {'dailyUsageBuckets':[
                {'startDate':today.isoformat(),'tokens':1234},
                {'startDate':(today-datetime.timedelta(days=40)).isoformat(),'tokens':999999}
            ],'summary':{'lifetimeTokens':54321}}
    else: error = {'code':-32601,'message':'unexpected method in readonly usage fixture'}
    print(json.dumps({'id':req['id'], **({'error':error} if error else {'result':result})}), flush=True)
";
