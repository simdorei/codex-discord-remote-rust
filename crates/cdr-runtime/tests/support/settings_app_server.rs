use cdr_app_server::{AppServerConfig, ResidentAppServer};
use std::path::Path;

pub async fn start(temp: &tempfile::TempDir, log: &Path, mode: &str) -> ResidentAppServer {
    let script = temp.path().join("settings-server.py");
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
        .insert("SETTINGS_TEST_LOG".into(), log.to_string_lossy().into());
    config
        .environment
        .insert("SETTINGS_TEST_MODE".into(), mode.into());
    ResidentAppServer::start(config).await.unwrap()
}

const SCRIPT: &str = r"
import json,os,sys,time
settings={'model':'model-a','effort':'high','serviceTier':None}
mode=os.environ['SETTINGS_TEST_MODE']
gate_root=os.path.dirname(os.environ['SETTINGS_TEST_LOG'])
def gate(name): return os.path.join(gate_root,name)
def signal(name):
    with open(gate(name),'w',encoding='utf-8') as f: f.write('ready')
def observe(thread):
    print(json.dumps({'method':'thread/settings/updated','params':{'threadId':thread,'threadSettings':settings}}),flush=True)
for line in sys.stdin:
    r=json.loads(line)
    with open(os.environ['SETTINGS_TEST_LOG'],'a',encoding='utf-8') as log: log.write(json.dumps(r)+'\n')
    if 'id' not in r or 'method' not in r: continue
    method=r['method']; p=r.get('params',{}); thread=p.get('threadId','thread-b')
    if method=='initialize': result={'userAgent':'settings-test/1'}
    elif method=='test/observe': observe(thread); result={}
    elif method in ['test/pauseReads','test/blocked']: result={}
    elif method=='model/list': result={'data':[
        {'model':'model-a','displayName':'Model A','supportedReasoningEfforts':[{'reasoningEffort':'high'},{'reasoningEffort':'low'}]},
        {'model':'model-b','displayName':'Model B','supportedReasoningEfforts':[{'reasoningEffort':'medium'}]}]}
    elif method=='thread/read': result={'thread':{'id':thread,'status':{'type':'idle'},'turns':[]}}
    elif method=='thread/resume':
        result={'thread':{'id':thread,'status':{'type':'idle'},'turns':[]},'model':settings['model'],'reasoningEffort':settings['effort'],'serviceTier':settings['serviceTier']}
        if mode=='wrong-thread': result['thread']['id']='other-thread'
        if mode=='incomplete-resume': del result['serviceTier']
        if mode=='watermark':
            signal('resume-ready')
            while not os.path.exists(gate('resume-release')): time.sleep(0.01)
    elif method=='thread/settings/update':
        if mode=='reject':
            print(json.dumps({'id':r['id'],'error':{'code':-32600,'message':'fixture settings rejection'}}),flush=True);continue
        for key in ['model','effort','serviceTier']:
            if key in p: settings[key]=p[key]
        if mode=='mismatch': settings['model']='model-a'
        if mode not in ['missing','watermark']: observe(thread)
        result={}
    else:
        print(json.dumps({'id':r['id'],'error':{'code':-32601,'message':'unexpected fixture method'}}),flush=True);continue
    print(json.dumps({'id':r['id'],'result':result}),flush=True)
    if method=='thread/resume' and mode=='watermark':
        observe(thread)
        emitted=False
        while not os.path.exists(gate('writer-release')):
            if not emitted and os.path.exists(gate('premature-emit')):
                settings['model']='model-b'; observe(thread); emitted=True
            time.sleep(0.01)
    if method=='test/pauseReads':
        while not os.path.exists(p['releasePath']): time.sleep(0.01)
";
