//! Restore fallback classification is scoped to real restore RPCs.
use super::*;

#[test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn a_real_rpc_marks_only_missing_rollout_restoration_unavailable() {
    futures::executor::block_on(async {
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
        for (error, missing, auth) in [
            (
                acp::Error::internal_error().data("no rollout found for thread id legacy-thread"),
                true,
                false,
            ),
            (
                acp::Error::internal_error().data("Service temporarily overloaded"),
                false,
                false,
            ),
            (acp::Error::auth_required(), false, true),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let connection = AcpConnector::unmanaged()
                .connect(ConnectRequest {
                    launch: nocterm_ai::AgentLaunch {
                        id: "restore-classification".into(),
                        name: "Restore".into(),
                        command: "/usr/bin/python3".into(),
                        args: vec![
                            "-c".into(),
                            script.into(),
                            serde_json::to_string(&error).unwrap(),
                        ],
                        env: Default::default(),
                        inherit_env: Vec::new(),
                    },
                    working_directory: dir.path().to_owned(),
                    terminal_auth: false,
                    sandbox: None,
                    resources: Default::default(),
                    cancellation: Default::default(),
                })
                .await
                .unwrap();
            for fork in [false, true] {
                let result = connection
                    .commands
                    .restore_session(nocterm_ai::RestoreSessionRequest {
                        session_id: acp::SessionId::new("legacy-thread"),
                        cwd: dir.path().to_owned(),
                        mcp_servers: Vec::new(),
                        fork,
                    })
                    .await;
                if missing {
                    assert!(matches!(result, Err(AgentError::RestoreUnavailable(_))));
                } else if auth {
                    assert!(matches!(result, Err(AgentError::AuthRequired(_))));
                } else {
                    assert!(matches!(result, Err(AgentError::Rpc(_))));
                }
            }
            let new = connection
                .commands
                .new_session(acp::NewSessionRequest::new(dir.path()))
                .await;
            let prompt = connection
                .commands
                .prompt(acp::PromptRequest::new(
                    acp::SessionId::new("legacy-thread"),
                    Vec::new(),
                ))
                .await;
            for result in [new.map(|_| ()), prompt.map(|_| ())] {
                if auth {
                    assert!(matches!(result, Err(AgentError::AuthRequired(_))));
                } else {
                    assert!(
                        matches!(result, Err(AgentError::Rpc(_))),
                        "a non-restore operation was incorrectly classified as recoverable restore failure"
                    );
                }
            }
            connection.commands.shutdown_gracefully().await.unwrap();
        }
    });
}
