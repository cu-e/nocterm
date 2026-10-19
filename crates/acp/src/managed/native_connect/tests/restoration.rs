//! Native restore requests follow advertised capabilities and classify only
//! restore RPC faults as unavailable.
use super::*;
use nocterm_ai::{
    history::source::ManagedCommandError,
    model::{diagnostic::RemoteFaultKind, ids::SessionId, session::RestoreSession},
};
fn restore(
    agent: &Connected,
    root: &Path,
    session: &str,
    fork: bool,
) -> Result<ManagedOpened, ManagedCommandError> {
    block_on(
        agent
            .commands
            .restore_session(&RestoreSession {
                session: SessionId::new(session),
                working_directory: root.to_owned(),
                endpoints: vec![],
                fork,
            })
            .unwrap(),
    )
}
fn kind(error: ManagedCommandError) -> RemoteFaultKind {
    match error {
        ManagedCommandError::Remote(fault) => fault.get().kind(),
        other => panic!("expected a remote fault, got {other}"),
    }
}
#[test]
fn forks_and_resumes_sessions_only_as_the_agent_advertises() {
    let script = r#"
import json, sys
capabilities = json.loads(sys.argv[1])
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    method = req['method']; params = req.get('params', {})
    if method == 'initialize': result = {'protocolVersion':1,'agentCapabilities':capabilities,'authMethods':[]}
    elif method == 'session/fork':
        assert params['sessionId'] == 's1' and params.get('mcpServers', []) == []
        result = {'sessionId':'s2'}
    elif method == 'session/resume': result = {}
    else: raise Exception('Unexpected method ' + method)
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}), flush=True)
"#;
    for advertised in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let capabilities = if advertised {
            r#"{"sessionCapabilities":{"fork":{},"resume":{}}}"#
        } else {
            "{}"
        };
        let agent = Connected::new(directory.path(), &python(script, [capabilities]));
        let forked = restore(&agent, directory.path(), "s1", true);
        let resumed = restore(&agent, directory.path(), "s1", false);
        if advertised {
            assert_eq!(forked.unwrap().opened().session.as_str(), "s2");
            assert_eq!(resumed.unwrap().opened().session.as_str(), "s1");
        } else {
            // Neither was advertised, so neither was sent.
            assert!(matches!(
                forked,
                Err(ManagedCommandError::LocalRestoreUnsupported { fork: true })
            ));
            assert!(matches!(
                resumed,
                Err(ManagedCommandError::LocalRestoreUnsupported { fork: false })
            ));
        }
        agent.finish();
    }
}
#[test]
fn resume_and_load_report_missing_rollout_as_unavailable() {
    let script = r#"
import json, sys
capabilities = json.loads(sys.argv[1])
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request: continue
    method = request['method']
    if method == 'initialize':
        answer = {'result': {'protocolVersion':1,'agentCapabilities':capabilities,'authMethods':[]}}
    else:
        assert method in ['session/load', 'session/resume']
        answer = {'error': {'code':-32603,'message':'Internal error','data':'no rollout found for thread id stale'}}
    answer.update({'jsonrpc':'2.0','id':request['id']})
    print(json.dumps(answer), flush=True)
"#;
    for capabilities in [
        r#"{"sessionCapabilities":{"resume":{}}}"#,
        r#"{"loadSession":true}"#,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let agent = Connected::new(directory.path(), &python(script, [capabilities]));
        let Err(error) = restore(&agent, directory.path(), "stale", false) else {
            panic!("a missing rollout restored a session")
        };
        assert!(
            error
                .to_string()
                .contains("no rollout found for thread id stale")
        );
        assert_eq!(kind(error), RemoteFaultKind::RestoreUnavailable);
        agent.finish();
    }
}
#[test]
fn a_real_rpc_marks_only_missing_rollout_restoration_unavailable() {
    let script = r#"
import json, sys
error = json.loads(sys.argv[1])
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    if req['method'] == 'initialize':
        response = {'result': {'protocolVersion': 1, 'agentCapabilities': {'sessionCapabilities': {'resume': {}, 'fork': {}}}}}
    else:
        response = {'error': error}
    response.update({'jsonrpc': '2.0', 'id': req['id']})
    print(json.dumps(response), flush=True)
"#;
    for (error, restoring, other) in [
        (
            r#"{"code":-32603,"message":"Internal error","data":"no rollout found for thread id legacy-thread"}"#,
            RemoteFaultKind::RestoreUnavailable,
            RemoteFaultKind::Rpc,
        ),
        (
            r#"{"code":-32603,"message":"Internal error","data":"Service temporarily overloaded"}"#,
            RemoteFaultKind::Rpc,
            RemoteFaultKind::Rpc,
        ),
        (
            r#"{"code":-32000,"message":"Authentication required"}"#,
            RemoteFaultKind::AuthenticationRequired,
            RemoteFaultKind::AuthenticationRequired,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let launch = python(script, [error]);
        let agent = Connected::new(directory.path(), &launch);
        for fork in [false, true] {
            let result = restore(&agent, directory.path(), "legacy-thread", fork);
            assert_eq!(kind(result.err().unwrap()), restoring);
        }
        let opened = agent.commands.new_session(&SessionOpen {
            working_directory: directory.path().to_owned(),
            endpoints: vec![],
        });
        assert_eq!(kind(block_on(opened.unwrap()).err().unwrap()), other);
        agent.finish();
        let mut prompting = super::prompting::Prompting::new(directory.path(), &launch);
        let (prompt, _scope) = prompting.prompt("after restore");
        assert_eq!(
            kind(block_on(prompt).err().unwrap()),
            other,
            "a non-restore operation was classified as a recoverable restore failure"
        );
        prompting.finish();
    }
}
