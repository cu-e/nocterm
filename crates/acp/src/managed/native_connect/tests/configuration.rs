//! Legacy model lists map to a native selection control backed by `set_model`.
use super::*;
use nocterm_ai::{
    model::{
        controls::{ConfigKind, ConfigOption},
        ids::ConfigId,
        notice::SessionUpdate,
        session::ConfigurationSelection,
    },
    session_config::ConfigValue,
};
const SCRIPT: &str = r#"
import json, sys
for line in sys.stdin:
    req=json.loads(line)
    if 'id' not in req: continue
    method=req['method']; params=req.get('params', {})
    if method=='initialize': result={'protocolVersion':1,'agentCapabilities':{},'authMethods':[]}
    elif method=='session/new': result={'sessionId':'s1','models':{'currentModelId':'first','availableModels':[{'modelId':'first','name':'First'},{'modelId':'second','name':'Second'},{'modelId':'fails','name':'Fails'}]}}
    elif method=='session/set_model':
        assert params['sessionId']=='s1'
        assert params['modelId'] in ['second','fails']
        if params['modelId']=='fails':
            print(json.dumps({'jsonrpc':'2.0','id':req['id'],'error':{'code':-32603,'message':'selection failed'}}),flush=True)
            continue
        result={}
        print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'s1','update':{'sessionUpdate':'config_option_update','configOptions':[{'id':'effort','name':'Effort','type':'select','currentValue':'low','options':[{'value':'low','name':'Low'}]}]}}}),flush=True)
    elif method=='session/set_config_option':
        assert params['configId']=='other'
        result={'configOptions':[]}
    else: raise Exception('Unexpected method '+method)
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;
fn current(option: &ConfigOption) -> &str {
    let ConfigKind::Select { current, .. } = &option.kind else {
        panic!("select")
    };
    current.as_str()
}
#[test]
fn legacy_models_use_real_set_model_and_preserve_selection_after_errors() {
    let directory = tempfile::tempdir().unwrap();
    let mut agent = Connected::new(directory.path(), &python(SCRIPT, []));
    let opened = agent.open(directory.path());
    let session = opened.opened().session.clone();
    let options = opened.opened().configuration.clone().unwrap();
    assert_eq!(options.len(), 1);
    let id = options[0].id.clone();
    let select = |option: ConfigId, value: &str| {
        block_on(
            agent
                .commands
                .set_config_option(&ConfigurationSelection {
                    session: session.clone(),
                    option,
                    value: ConfigValue::Value(value.into()),
                })
                .unwrap(),
        )
    };
    let changed = select(id.clone(), "second").unwrap();
    let selected = changed.get().iter().find(|option| option.id == id).unwrap();
    assert_eq!(current(selected), "second");
    assert!(select(id.clone(), "fails").is_err());
    assert!(select(id.clone(), "unadvertised").is_err());
    let retained = select(ConfigId::new("other"), "a").unwrap();
    assert_eq!(current(&retained.get()[0]), "second");
    let Some(ManagedAgentEvent::Session(notice)) = block_on(agent.next_event()) else {
        panic!("session update")
    };
    let SessionUpdate::Configuration(update) = &notice.notice().update else {
        panic!("config update")
    };
    assert_eq!(update.len(), 2);
    assert!(update.iter().any(|option| option.id == id));
    agent.finish();
}
