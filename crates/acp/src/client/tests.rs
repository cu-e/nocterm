use super::*;
use agent_client_protocol::{Channel, schema::ProtocolVersion};
use serde_json::json;

#[cfg(unix)]
#[test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn legacy_models_use_real_set_model_and_preserve_selection_after_errors() {
    futures::executor::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let script = r#"
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
        let connection = AcpConnector::unmanaged()
            .connect(ConnectRequest {
                terminal_auth: false,
                sandbox: None,
                resources: Default::default(),
                cancellation: Default::default(),
                launch: nocterm_ai::AgentLaunch {
                    id: "legacy-test".into(),
                    name: "Legacy".into(),
                    command: "/usr/bin/python3".into(),
                    args: vec!["-c".into(), script.into()],
                    env: Default::default(),
                    inherit_env: Vec::new(),
                },
                working_directory: directory.path().to_owned(),
            })
            .await
            .unwrap();
        let session = connection
            .commands
            .new_session(acp::NewSessionRequest::new(directory.path().to_owned()))
            .await
            .unwrap();
        let options = session.config_options.unwrap();
        assert_eq!(options.len(), 1);
        let id = options[0].id.clone();
        let changed = connection
            .commands
            .set_config_option(acp::SetSessionConfigOptionRequest::new(
                session.session_id.clone(),
                id.clone(),
                "second",
            ))
            .await
            .unwrap();
        let selected = changed.iter().find(|option| option.id == id).unwrap();
        let acp::SessionConfigKind::Select(select) = &selected.kind else {
            panic!("select")
        };
        assert_eq!(select.current_value.0.as_ref(), "second");
        let event = connection.events.recv().await.unwrap();
        let AgentEvent::Session(notification) = event else {
            panic!("session update")
        };
        let acp::SessionUpdate::ConfigOptionUpdate(update) = notification.update else {
            panic!("config update")
        };
        assert_eq!(update.config_options.len(), 2);
        assert!(update.config_options.iter().any(|option| option.id == id));
        assert!(
            connection
                .commands
                .set_config_option(acp::SetSessionConfigOptionRequest::new(
                    session.session_id.clone(),
                    id.clone(),
                    "fails"
                ))
                .await
                .is_err()
        );
        assert!(
            connection
                .commands
                .set_config_option(acp::SetSessionConfigOptionRequest::new(
                    session.session_id.clone(),
                    id,
                    "unadvertised"
                ))
                .await
                .is_err()
        );
        let retained = connection
            .commands
            .set_config_option(acp::SetSessionConfigOptionRequest::new(
                session.session_id,
                "other",
                "a",
            ))
            .await
            .unwrap();
        let acp::SessionConfigKind::Select(select) = &retained[0].kind else {
            panic!("select")
        };
        assert_eq!(select.current_value.0.as_ref(), "second");
        connection.commands.shutdown();
    });
}

#[cfg(unix)]
#[test]
fn forks_and_resumes_sessions_only_as_the_agent_advertises() {
    futures::executor::block_on(async {
        for fork in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let capabilities = if fork {
                "{'sessionCapabilities':{'fork':{},'resume':{}}}"
            } else {
                "{}"
            };
            let script = format!(
                r#"
import json, sys
for line in sys.stdin:
    req=json.loads(line)
    if 'id' not in req: continue
    method=req['method']; params=req.get('params', {{}})
    if method=='initialize': result={{'protocolVersion':1,'agentCapabilities':{capabilities},'authMethods':[]}}
    elif method=='session/fork':
        assert params['sessionId']=='s1' and params.get('mcpServers',[])==[]
        result={{'sessionId':'s2'}}
    elif method=='session/resume': result={{}}
    else: raise Exception('Unexpected method '+method)
    print(json.dumps({{'jsonrpc':'2.0','id':req['id'],'result':result}}),flush=True)
"#
            );
            let connection = AcpConnector::unmanaged()
                .connect(ConnectRequest {
                    terminal_auth: false,
                    sandbox: None,
                    resources: Default::default(),
                    cancellation: Default::default(),
                    launch: nocterm_ai::AgentLaunch {
                        id: "fork-test".into(),
                        name: "Fork".into(),
                        command: "/usr/bin/python3".into(),
                        args: vec!["-c".into(), script],
                        env: Default::default(),
                        inherit_env: Vec::new(),
                    },
                    working_directory: directory.path().to_owned(),
                })
                .await
                .unwrap();
            let request = |fork| nocterm_ai::RestoreSessionRequest {
                session_id: acp::SessionId::new("s1"),
                cwd: directory.path().to_owned(),
                mcp_servers: Vec::new(),
                fork,
            };
            let forked = connection.commands.restore_session(request(true)).await;
            let resumed = connection.commands.restore_session(request(false)).await;
            if fork {
                assert_eq!(forked.unwrap().session_id.to_string(), "s2");
                assert_eq!(resumed.unwrap().session_id.to_string(), "s1");
            } else {
                // Neither was advertised, so neither was sent.
                assert!(forked.is_err());
                assert!(resumed.is_err());
            }
            connection.commands.shutdown();
        }
    });
}

#[test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn sdk_channel_initializes_streams_permissions_and_config() {
    futures::executor::block_on(async {
        let (client_transport, agent_transport) = Channel::duplex();
        let (events_tx, events) = async_channel::bounded(256);
        let agent=Agent.builder()
            .on_receive_request(async |request:acp::InitializeRequest,responder,_| {
                assert!(!request.client_capabilities.terminal);
                assert!(!request.client_capabilities.fs.read_text_file);
                assert!(!request.client_capabilities.fs.write_text_file);
                responder.respond(acp::InitializeResponse::new(ProtocolVersion::V1))
            },agent_client_protocol::on_receive_request!())
            .on_receive_request(async |request:acp::NewSessionRequest,responder,_| {
                assert!(request.cwd.is_absolute());
                assert_eq!(request.mcp_servers.len(),1);
                let serialized=serde_json::to_value(request.mcp_servers).unwrap();
                assert_eq!(serialized[0]["name"],"nocterm");
                responder.respond(acp::NewSessionResponse::new(acp::SessionId::new("s1")))
            },agent_client_protocol::on_receive_request!())
            .on_receive_request(async |request:acp::PromptRequest,responder,connection| {
                connection.clone().spawn(async move {
                let notification:acp::SessionNotification=serde_json::from_value(json!({"sessionId":request.session_id,"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"streamed"}}})).unwrap();
                connection.send_notification(notification)?;
                let permission:acp::RequestPermissionRequest=serde_json::from_value(json!({"sessionId":"s1","toolCall":{"toolCallId":"tool1","title":"test","status":"pending"},"options":[{"optionId":"yes","name":"Allow","kind":"allow_once"}]})).unwrap();
                let outcome=connection.send_request(permission).block_task().await?;
                assert!(matches!(outcome.outcome,acp::RequestPermissionOutcome::Cancelled));
                responder.respond(acp::PromptResponse::new(acp::StopReason::EndTurn))

                })?;
                Ok(())
            },agent_client_protocol::on_receive_request!())
            .on_receive_request(async |_:acp::SetSessionModeRequest,responder,_|responder.respond(acp::SetSessionModeResponse::default()),agent_client_protocol::on_receive_request!())
            .on_receive_request(async |_:acp::SetSessionConfigOptionRequest,responder,_|responder.respond(acp::SetSessionConfigOptionResponse::new(Vec::new())),agent_client_protocol::on_receive_request!())
            .on_receive_request(async |_:acp::AuthenticateRequest,responder,_|responder.respond(acp::AuthenticateResponse::default()),agent_client_protocol::on_receive_request!())
            .connect_to(agent_transport);
        let client=client_builder(events_tx).connect_with(client_transport,async |connection:ConnectionTo<Agent>| {
            connection.send_request(acp::InitializeRequest::new(ProtocolVersion::V1)).block_task().await?;
            let server:acp::McpServer=serde_json::from_value(json!({"name":"nocterm","command":"/test/nocterm","args":["agent-bridge"],"env":[{"name":"NOCTERM_BRIDGE_ENDPOINT","value":"test"},{"name":"NOCTERM_BRIDGE_TOKEN","value":"test"}]})).unwrap();
            let session=connection.send_request(acp::NewSessionRequest::new(std::env::temp_dir()).mcp_servers(vec![server])).block_task().await?;
            connection.send_request(acp::SetSessionModeRequest::new(session.session_id.clone(),acp::SessionModeId::new("mode"))).block_task().await?;
            connection.send_request(acp::SetSessionConfigOptionRequest::new(session.session_id.clone(),acp::SessionConfigId::new("model"),acp::SessionConfigValueId::new("test"))).block_task().await?;
            connection.send_request(acp::AuthenticateRequest::new(acp::AuthMethodId::new("oauth"))).block_task().await?;
            connection.send_request(acp::PromptRequest::new(session.session_id,vec![acp::ContentBlock::Text(acp::TextContent::new("test"))])).block_task().await?;
            Ok(())
        });
        let foreground = async {
            let mut streamed = false;
            loop {
                match events
                    .recv()
                    .await
                    .map_err(|_| acp::Error::internal_error())?
                {
                    AgentEvent::Session(notification) => {
                        assert!(matches!(
                            notification.update,
                            acp::SessionUpdate::AgentMessageChunk(_)
                        ));
                        streamed = true;
                    }
                    AgentEvent::Permission { respond, .. } => {
                        assert!(streamed);
                        respond.respond(acp::RequestPermissionOutcome::Cancelled);
                        return Ok::<_, acp::Error>(());
                    }
                    _ => panic!("Unexpected event"),
                }
            }
        };
        let run = async {
            match select(
                Box::pin(futures::future::try_join(client, foreground)),
                Box::pin(agent),
            )
            .await
            {
                Either::Left((result, _)) => result.map(|_| ()),
                Either::Right((result, _)) => result,
            }
        };
        match select(
            Box::pin(run),
            Box::pin(async_io::Timer::after(Duration::from_secs(5))),
        )
        .await
        {
            Either::Left((result, _)) => result.unwrap(),
            Either::Right(_) => panic!("SDK roundtrip timed out"),
        }
    });
}
#[test]
fn remote_error_payloads_are_sanitized() {
    let error = acp::Error::auth_required().data(json!({"password":"marker"}));
    assert!(matches!(map_error(error), AgentError::AuthRequired(_)));
    let error = map_error(acp::Error::internal_error().data("marker"));
    let AgentError::Rpc(error) = error else {
        panic!("Wrong error variant")
    };
    assert!(error.data.is_none());
    let secret = "custom-auth-marker".to_owned();
    let error=map_error_with(acp::Error::internal_error().data(json!({"message":"Session limit reached; custom-auth-marker", "unrelated":"do not expose"})),&[secret]);
    let AgentError::Rpc(error) = error else {
        panic!("Wrong error variant")
    };
    assert!(error.message.contains("Session limit reached"));
    assert!(!error.message.contains("custom-auth-marker"));
    assert!(!error.message.contains("do not expose"));
    assert!(error.data.is_none());
}
#[cfg(unix)]
#[test]
fn oversized_subprocess_line_fails_initialization_and_stops_processes() {
    futures::executor::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let launch = nocterm_ai::AgentLaunch {
            id: "oversized".into(),
            name: "oversized".into(),
            command: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "head -c 16777217 /dev/zero | tr '\\000' x; printf '\\n'; sleep 60".into(),
            ],
            env: Default::default(),
            inherit_env: Vec::new(),
        };
        let result = AcpConnector::unmanaged().connect(ConnectRequest {
            terminal_auth: false,
            sandbox: None,
            resources: Default::default(),
            cancellation: Default::default(),
            launch,
            working_directory: directory.path().to_owned(),
        });
        match select(
            result,
            Box::pin(async_io::Timer::after(Duration::from_secs(5))),
        )
        .await
        {
            Either::Left((result, _)) => assert!(result.is_err()),
            Either::Right(_) => {
                panic!("Oversized protocol line did not terminate initialization")
            }
        }
    });
}
#[test]
fn sdk_cancel_remains_dispatchable_during_generation() {
    futures::executor::block_on(async {
        let (client_transport, agent_transport) = Channel::duplex();
        let (events_tx, events) = async_channel::bounded(256);
        let (cancel_tx, cancel_rx) = async_channel::bounded::<()>(1);
        let agent = Agent
            .builder()
            .on_receive_request(
                async move |request: acp::PromptRequest, responder, connection| {
                    let cancelled = cancel_rx.clone();
                    connection.clone().spawn(async move {
                        connection.send_notification(acp::SessionNotification::new(
                            request.session_id,
                            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                                acp::ContentBlock::Text(acp::TextContent::new("started")),
                            )),
                        ))?;
                        cancelled
                            .recv()
                            .await
                            .map_err(|_| acp::Error::internal_error())?;
                        responder.respond(acp::PromptResponse::new(acp::StopReason::Cancelled))
                    })?;
                    Ok(())
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_notification(
                async move |notification: acp::CancelNotification, _| {
                    assert_eq!(notification.session_id.to_string(), "s1");
                    cancel_tx
                        .try_send(())
                        .map_err(|_| acp::Error::internal_error())
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .connect_to(agent_transport);
        let client = client_builder(events_tx).connect_with(
            client_transport,
            async move |connection: ConnectionTo<Agent>| {
                let prompt = connection
                    .send_request(acp::PromptRequest::new(
                        acp::SessionId::new("s1"),
                        vec![acp::ContentBlock::Text(acp::TextContent::new("test"))],
                    ))
                    .block_task();
                let cancel = async {
                    assert!(matches!(
                        events.recv().await.unwrap(),
                        AgentEvent::Session(_)
                    ));
                    connection
                        .send_notification(acp::CancelNotification::new(acp::SessionId::new("s1")))
                };
                let (response, _) = futures::future::try_join(prompt, cancel).await?;
                assert_eq!(response.stop_reason, acp::StopReason::Cancelled);
                Ok(())
            },
        );
        let run = async {
            match select(Box::pin(client), Box::pin(agent)).await {
                Either::Left((result, _)) => result,
                Either::Right((result, _)) => result,
            }
        };
        match select(
            Box::pin(run),
            Box::pin(async_io::Timer::after(Duration::from_secs(5))),
        )
        .await
        {
            Either::Left((result, _)) => result.unwrap(),
            Either::Right(_) => panic!("Cancellation stalled the ACP connection"),
        }
    });
}
#[test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn closes_sessions_only_when_agent_advertises_support() {
    for advertised in [false, true] {
        futures::executor::block_on(async {
            let (client_transport, agent_transport) = Channel::duplex();
            let (events_tx, _) = async_channel::bounded(256);
            let (closed_tx, closed_rx) = async_channel::bounded(1);
            let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let received = count.clone();
            let agent = Agent
                .builder()
                .on_receive_request(
                    async move |_: acp::InitializeRequest, responder, _| {
                        let capabilities: acp::AgentCapabilities =
                            serde_json::from_value(if advertised {
                                json!({"sessionCapabilities":{"close":{}}})
                            } else {
                                json!({})
                            })
                            .unwrap();
                        responder.respond(
                            acp::InitializeResponse::new(ProtocolVersion::V1)
                                .agent_capabilities(capabilities),
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: acp::CloseSessionRequest, responder, _| {
                        assert_eq!(request.session_id.to_string(), "s1");
                        received.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        closed_tx
                            .try_send(())
                            .map_err(|_| acp::Error::internal_error())?;
                        responder.respond(acp::CloseSessionResponse::new())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async |_: acp::SetSessionModeRequest, responder, _| {
                        responder.respond(acp::SetSessionModeResponse::default())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(agent_transport);
            let client = client_builder(events_tx).connect_with(
                client_transport,
                async move |connection: ConnectionTo<Agent>| {
                    let info = connection
                        .send_request(acp::InitializeRequest::new(ProtocolVersion::V1))
                        .block_task()
                        .await?;
                    close_session(
                        connection.clone(),
                        info.agent_capabilities.session_capabilities.close.is_some(),
                        acp::SessionId::new("s1"),
                        &[],
                    )
                    .await
                    .unwrap();
                    if advertised {
                        closed_rx
                            .recv()
                            .await
                            .map_err(|_| acp::Error::internal_error())?;
                    }
                    // A subsequent roundtrip also proves the close task did not shut down the shared connection.
                    connection
                        .send_request(acp::SetSessionModeRequest::new(
                            acp::SessionId::new("s2"),
                            acp::SessionModeId::new("mode"),
                        ))
                        .block_task()
                        .await?;
                    assert_eq!(
                        count.load(std::sync::atomic::Ordering::SeqCst),
                        usize::from(advertised)
                    );
                    Ok(())
                },
            );
            let run = async {
                match select(Box::pin(client), Box::pin(agent)).await {
                    Either::Left((result, _)) => result,
                    Either::Right((result, _)) => result,
                }
            };
            match select(
                Box::pin(run),
                Box::pin(async_io::Timer::after(Duration::from_secs(5))),
            )
            .await
            {
                Either::Left((result, _)) => result.unwrap(),
                Either::Right(_) => panic!("Close-session dispatch timed out"),
            }
        });
    }
}

#[test]
fn missing_rollout_is_unavailable_only_for_restore_rpc_errors() {
    let raw = acp::Error::internal_error().data(json!({
        "message": "no rollout found for thread id 01a11ca8-4cde-72e3-8429-f13aaec4f58c"
    }));
    let mapped = map_error(raw.clone());
    assert!(matches!(mapped, AgentError::Rpc(_)));
    assert!(matches!(
        restore_error(mapped),
        AgentError::RestoreUnavailable(_)
    ));
    for message in [
        "storage temporarily unavailable",
        "rollout write failed",
        "thread id invalid",
        "no rollout found",
        "no rollout found for thread identity",
    ] {
        assert!(matches!(
            restore_error(map_error(acp::Error::internal_error().data(message))),
            AgentError::Rpc(_)
        ));
    }
    assert!(matches!(
        restore_error(map_error(
            acp::Error::auth_required().data("no rollout found for thread id old")
        )),
        AgentError::AuthRequired(_)
    ));
    assert!(matches!(
        restore_error(AgentError::Io("no rollout found for thread id old".into())),
        AgentError::Io(_)
    ));
}

#[cfg(unix)]
#[test]
fn resume_and_load_report_missing_rollout_as_unavailable() {
    for resume in [false, true] {
        futures::executor::block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let capabilities = if resume {
                r#"{"sessionCapabilities":{"resume":{}}}"#
            } else {
                r#"{"loadSession":true}"#
            };
            let script = format!(
                r#"
import json, sys
capabilities=json.loads('{capabilities}')
for line in sys.stdin:
    request=json.loads(line)
    if 'id' not in request: continue
    method=request['method']
    if method=='initialize':
        result={{'protocolVersion':1,'agentCapabilities':capabilities,'authMethods':[]}}
        answer={{'jsonrpc':'2.0','id':request['id'],'result':result}}
    else:
        assert method in ['session/load','session/resume']
        answer={{'jsonrpc':'2.0','id':request['id'],'error':{{'code':-32603,'message':'Internal error','data':'no rollout found for thread id stale'}}}}
    print(json.dumps(answer),flush=True)
"#
            );
            let launch = nocterm_ai::AgentLaunch {
                id: "restore-fixture".into(),
                name: "restore-fixture".into(),
                command: "python3".into(),
                args: vec!["-u".into(), "-c".into(), script],
                env: Default::default(),
                inherit_env: vec!["PATH".into()],
            };
            let connection = AcpConnector::unmanaged()
                .connect(ConnectRequest {
                    launch,
                    working_directory: directory.path().to_owned(),
                    terminal_auth: false,
                    sandbox: None,
                    resources: Default::default(),
                    cancellation: Default::default(),
                })
                .await
                .unwrap();
            let result = connection
                .commands
                .restore_session(nocterm_ai::RestoreSessionRequest {
                    session_id: acp::SessionId::new("stale"),
                    cwd: directory.path().to_owned(),
                    mcp_servers: Vec::new(),
                    fork: false,
                })
                .await;
            connection.commands.shutdown_gracefully().await.unwrap();
            assert!(
                matches!(result, Err(AgentError::RestoreUnavailable(message)) if message.contains("no rollout found for thread id stale"))
            );
        });
    }
}
