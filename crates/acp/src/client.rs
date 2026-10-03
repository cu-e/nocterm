use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_client_protocol::{Agent, Client, ConnectionTo, Lines};
use futures::{
    AsyncReadExt, FutureExt,
    channel::oneshot,
    future::{BoxFuture, Either, select},
};
use nocterm_ai::{
    AgentCommands, AgentConnection, AgentConnector, AgentError, AgentEvent, AgentInfo,
    ConnectRequest, acp,
};

use crate::{lines, process};

/// Runtime-neutral ACP connector. Each live subprocess has one protocol driver.
#[derive(Clone, Default)]
pub struct AcpConnector;

struct ConnectingGuard(Option<Arc<process::ProcessGroup>>);
impl Drop for ConnectingGuard {
    fn drop(&mut self) {
        if let Some(group) = &self.0 {
            group.kill();
        }
    }
}

impl AgentConnector for AcpConnector {
    fn connect(
        &self,
        request: ConnectRequest,
    ) -> BoxFuture<'static, Result<AgentConnection, AgentError>> {
        async move {
            let mut spawned = process::spawn(&request)?;
            let mut connecting = ConnectingGuard(Some(spawned.group.clone()));
            let stdin = spawned.child.stdin.take().ok_or_else(|| AgentError::Io("Agent stdin unavailable".into()))?;
            let stdout = spawned.child.stdout.take().ok_or_else(|| AgentError::Io("Agent stdout unavailable".into()))?;
            let stderr = spawned.child.stderr.take().ok_or_else(|| AgentError::Io("Agent stderr unavailable".into()))?;
            let stderr_tail = Arc::new(Mutex::new(Vec::new()));
            let tail = stderr_tail.clone();
            std::thread::Builder::new().name("nocterm-agent-stderr".into()).spawn(move || {
                futures::executor::block_on(async move {
                    let mut stderr = stderr;
                    let mut buffer = [0u8; 4096];
                    while let Ok(count) = stderr.read(&mut buffer).await {
                        if count == 0 { break; }
                        let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
                        tail.extend_from_slice(&buffer[..count]);
                        let excess = tail.len().saturating_sub(64 * 1024);
                        tail.drain(..excess);
                    }
                });
            }).map_err(|e| AgentError::Io(e.to_string()))?;
            let (events_tx, events_rx) = async_channel::bounded(256);
            let drain_events = events_rx.clone();
            let (stop_tx, stop_rx) = async_channel::bounded(1);
            let (ready_tx, ready_rx) = oneshot::channel();
            let group = spawned.group.clone();
            let command_secrets=spawned.secrets.clone();
            let name = request.launch.name;
            std::thread::Builder::new().name("nocterm-acp".into()).spawn(move || {
                futures::executor::block_on(async move {
                    let protocol = client_builder(events_tx.clone())
                        .connect_with(Lines::new(Box::pin(lines::outgoing(stdin)), Box::pin(lines::incoming(stdout))), async move |connection: ConnectionTo<Agent>| {
                            let capabilities = acp::ClientCapabilities::default().session(acp::ClientSessionCapabilities::default().config_options(acp::SessionConfigOptionsCapabilities::default().boolean(acp::BooleanConfigOptionCapabilities::default())));
                            let initialize = connection.send_request(acp::InitializeRequest::new(agent_client_protocol::schema::ProtocolVersion::V1).client_capabilities(capabilities).client_info(acp::Implementation::new("nocterm", "0"))).block_task();
                            let response = match select(Box::pin(initialize), Box::pin(async_io::Timer::after(Duration::from_secs(60)))).await {
                                Either::Left((response, _)) => response.map_err(|error|map_error_with(error,&command_secrets)),
                                Either::Right(_) => Err(AgentError::Io("Agent initialization timed out".into())),
                            };
                            match response {
                                Ok(response) if response.protocol_version == agent_client_protocol::schema::ProtocolVersion::V1 => {
                                    let implementation = response.agent_info;
                                    let info = AgentInfo {name:implementation.as_ref().map_or(name, |v| v.name.clone()), version:implementation.map_or_else(String::new, |v| v.version), capabilities:response.agent_capabilities, auth_methods:response.auth_methods};
                                    let close_supported=info.capabilities.session_capabilities.close.is_some();
                                    let commands = Arc::new(Commands {connection:connection.clone(), stop:stop_tx, group,secrets:command_secrets,close_supported});
                                    if ready_tx.send(Ok(AgentConnection {info, commands, events:events_rx})).is_err() { return Ok(()); }
                                    let _ = select(Box::pin(stop_rx.recv()), Box::pin(connection.incoming_closed())).await;
                                    Ok(())
                                }
                                Ok(_) => { let _ = ready_tx.send(Err(AgentError::Io("Unsupported ACP protocol version".into()))); Ok(()) }
                                Err(error) => { let _ = ready_tx.send(Err(error)); Ok(()) }
                            }
                        });
                    let (result, status) = match select(Box::pin(protocol), Box::pin(spawned.child.status())).await {
                        Either::Left((result, status)) => {
                            spawned.group.finish();
                            (result, status.await.ok())
                        }
                        Either::Right((status, protocol)) => {
                            // Descendants can retain stdout after the leader exits.
                            // Drain final protocol events, then terminate the entire group.
                            let _ = async_io::Timer::after(Duration::from_millis(250)).await;
                            spawned.group.finish();
                            (protocol.await, status.ok())
                        }
                    };
                    let _ = async_io::Timer::after(Duration::from_millis(50)).await;
                    let bytes = stderr_tail.lock().unwrap_or_else(|e| e.into_inner());
                    let stderr_tail = scrub(&String::from_utf8_lossy(&bytes),&spawned.secrets);
                    drop(bytes);
                    let stderr_tail = if stderr_tail.is_empty() { result.err().map_or_else(String::new, |e| scrub(&e.to_string(),&spawned.secrets)) } else { stderr_tail };
                    if let Err(async_channel::TrySendError::Full(event)) = events_tx.try_send(AgentEvent::Exited {code:status.and_then(|v|v.code()), stderr_tail}) {
                        let _ = drain_events.try_recv();
                        let _ = events_tx.try_send(event);
                    }
                    events_tx.close();
                });
            }).map_err(|e| AgentError::Io(e.to_string()))?;
            let result = ready_rx.await.map_err(|_| AgentError::Io("Agent exited during initialization".into()))?;
            if result.is_ok() {connecting.0.take();}
            result
        }.boxed()
    }
}

struct Commands {
    connection: ConnectionTo<Agent>,
    stop: async_channel::Sender<()>,
    group: Arc<process::ProcessGroup>,
    secrets: Arc<Vec<String>>,
    close_supported: bool,
}
impl AgentCommands for Commands {
    fn new_session(
        &self,
        request: acp::NewSessionRequest,
    ) -> BoxFuture<'static, Result<acp::NewSessionResponse, AgentError>> {
        let connection = self.connection.clone();
        let secrets = self.secrets.clone();
        async move {
            connection
                .send_request(request)
                .block_task()
                .await
                .map_err(|error| map_error_with(error, &secrets))
        }
        .boxed()
    }
    fn prompt(
        &self,
        request: acp::PromptRequest,
    ) -> BoxFuture<'static, Result<acp::PromptResponse, AgentError>> {
        let connection = self.connection.clone();
        let secrets = self.secrets.clone();
        async move {
            connection
                .send_request(request)
                .block_task()
                .await
                .map_err(|error| map_error_with(error, &secrets))
        }
        .boxed()
    }
    fn cancel(&self, session: acp::SessionId) {
        let _ = self
            .connection
            .send_notification(acp::CancelNotification::new(session));
    }
    fn set_mode(
        &self,
        request: acp::SetSessionModeRequest,
    ) -> BoxFuture<'static, Result<(), AgentError>> {
        let connection = self.connection.clone();
        let secrets = self.secrets.clone();
        async move {
            connection
                .send_request(request)
                .block_task()
                .await
                .map(|_| ())
                .map_err(|error| map_error_with(error, &secrets))
        }
        .boxed()
    }
    fn set_config_option(
        &self,
        request: acp::SetSessionConfigOptionRequest,
    ) -> BoxFuture<'static, Result<Vec<acp::SessionConfigOption>, AgentError>> {
        let connection = self.connection.clone();
        let secrets = self.secrets.clone();
        async move {
            connection
                .send_request(request)
                .block_task()
                .await
                .map(|r| r.config_options)
                .map_err(|error| map_error_with(error, &secrets))
        }
        .boxed()
    }
    fn authenticate(
        &self,
        method: acp::AuthMethodId,
    ) -> BoxFuture<'static, Result<(), AgentError>> {
        let connection = self.connection.clone();
        let secrets = self.secrets.clone();
        async move {
            connection
                .send_request(acp::AuthenticateRequest::new(method))
                .block_task()
                .await
                .map(|_| ())
                .map_err(|error| map_error_with(error, &secrets))
        }
        .boxed()
    }
    fn close_session(&self, session: acp::SessionId) {
        queue_close_session(self.connection.clone(), self.close_supported, session);
    }
    fn shutdown(&self) {
        let _ = self.stop.try_send(());
        self.group.kill();
    }
}
impl Drop for Commands {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn queue_close_session(connection: ConnectionTo<Agent>, supported: bool, session: acp::SessionId) {
    if !supported {
        return;
    }
    let request_connection = connection.clone();
    let _ = connection.spawn(async move {
        // Closing a UI thread must not block the foreground or the inbound dispatcher.
        let _ = request_connection
            .send_request(acp::CloseSessionRequest::new(session))
            .block_task()
            .await;
        Ok(())
    });
}

fn map_error_with(mut error: acp::Error, secrets: &[String]) -> AgentError {
    if let Some(data) = error.data.as_ref()
        && let Some(detail) = data
            .as_str()
            .or_else(|| data.get("message").and_then(serde_json::Value::as_str))
            .or_else(|| data.get("details").and_then(serde_json::Value::as_str))
            .or_else(|| {
                data.get("error").and_then(|v| {
                    v.as_str()
                        .or_else(|| v.get("message").and_then(serde_json::Value::as_str))
                })
            })
    {
        let detail = scrub(detail, secrets);
        error.message.push_str(": ");
        error.message.extend(detail.chars().take(2048));
    }
    error.message = scrub(&error.message, secrets);
    // Remote error data can contain arbitrary sensitive output.
    error.data = None;
    if error.code == acp::Error::auth_required().code {
        AgentError::AuthRequired(error.message.to_string())
    } else {
        AgentError::Rpc(error)
    }
}

fn scrub(text: &str, secrets: &[String]) -> String {
    let mut text = text.to_owned();
    for secret in secrets {
        text = text.replace(secret, "[REDACTED]");
    }
    nocterm_ai::redact::redact(&text)
}
#[cfg(test)]
fn map_error(error: acp::Error) -> AgentError {
    map_error_with(error, &[])
}

fn client_builder(
    events_tx: async_channel::Sender<AgentEvent>,
) -> agent_client_protocol::Builder<Client, impl agent_client_protocol::HandleDispatchFrom<Agent>> {
    let notification_tx = events_tx.clone();
    let permission_tx = events_tx;
    Client
        .builder()
        .name("nocterm")
        .on_receive_notification(
            async move |notification: acp::SessionNotification, _| {
                notification_tx
                    .try_send(AgentEvent::Session(notification))
                    .map_err(|_| acp::Error::internal_error().data("Agent event queue overflow"))
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: acp::RequestPermissionRequest, responder, connection| {
                let (respond, answer) = oneshot::channel();
                permission_tx
                    .try_send(AgentEvent::Permission { request, respond })
                    .map_err(|_| acp::Error::internal_error())?;
                connection.clone().spawn(async move {
                    let cancellation = responder.cancellation();
                    let wait = cancellation.run_until_cancelled(async {
                        Ok(answer
                            .await
                            .unwrap_or(acp::RequestPermissionOutcome::Cancelled))
                    });
                    let outcome = match select(
                        Box::pin(wait),
                        Box::pin(connection.incoming_closed()),
                    )
                    .await
                    {
                        Either::Left((Ok(answer), _)) => answer,
                        _ => acp::RequestPermissionOutcome::Cancelled,
                    };
                    responder.respond(acp::RequestPermissionResponse::new(outcome))
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::{Channel, schema::ProtocolVersion};
    use serde_json::json;

    #[test]
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
                            let _ = respond.send(acp::RequestPermissionOutcome::Cancelled);
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
            let result = AcpConnector.connect(ConnectRequest {
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
                        connection.send_notification(acp::CancelNotification::new(
                            acp::SessionId::new("s1"),
                        ))
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
                        queue_close_session(
                            connection.clone(),
                            info.agent_capabilities.session_capabilities.close.is_some(),
                            acp::SessionId::new("s1"),
                        );
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
}
