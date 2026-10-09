use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_client_protocol::{Agent, Client, ConnectionTo, Lines, UntypedMessage};
use futures::{
    AsyncReadExt, FutureExt,
    channel::oneshot,
    future::{BoxFuture, Either, select},
};
use nocterm_ai::{
    AgentCommands, AgentConnection, AgentConnector, AgentError, AgentEvent, AgentInfo,
    ConnectRequest, acp,
};

use crate::{lines, models::Models, process};

/// Runtime-neutral ACP connector. Each live subprocess has one protocol driver.
#[derive(Clone, Default)]
pub struct AcpConnector {
    helper: Option<std::path::PathBuf>,
}
impl AcpConnector {
    pub fn managed(helper: std::path::PathBuf) -> Self {
        Self {
            helper: Some(helper),
        }
    }
    pub fn unmanaged() -> Self {
        Self::default()
    }
}

struct ConnectingGuard(Option<Arc<process::ProcessGroup>>);
impl Drop for ConnectingGuard {
    fn drop(&mut self) {
        if let Some(group) = &self.0 {
            group.kill();
        }
    }
}

impl AgentConnector for AcpConnector {
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn connect(
        &self,
        request: ConnectRequest,
    ) -> BoxFuture<'static, Result<AgentConnection, AgentError>> {
        let helper = self.helper.clone();
        async move {
            let cancellation = request.cancellation.clone();
            if cancellation.is_cancelled() { return Err(AgentError::Io("Agent startup cancelled".into())); }
            let mut spawned = process::spawn(&request, helper.as_deref())?;
            let startup_group = spawned.group.clone();
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
            let terminal_auth = request.terminal_auth;
            std::thread::Builder::new().name("nocterm-acp".into()).spawn(move || {
                futures::executor::block_on(async move {
                    let models = Arc::new(Mutex::new(Models::default()));
                    let command_events = events_tx.clone();
                    let protocol = client_builder_with_models(events_tx.clone(), models.clone())
                        .connect_with(Lines::new(Box::pin(lines::outgoing(stdin)), Box::pin(lines::incoming(stdout))), async move |connection: ConnectionTo<Agent>| {
                            let capabilities = acp::ClientCapabilities::default().auth(acp::AuthCapabilities::new().terminal(terminal_auth)).session(acp::ClientSessionCapabilities::default().config_options(acp::SessionConfigOptionsCapabilities::default().boolean(acp::BooleanConfigOptionCapabilities::default())));
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
                                    let restore = if info.capabilities.session_capabilities.resume.is_some() { Restore::Resume } else if info.capabilities.load_session { Restore::Load } else { Restore::Unsupported };
                                    let fork_supported=info.capabilities.session_capabilities.fork.is_some();
                                    let commands = Arc::new(Commands {connection:connection.clone(), stop:stop_tx, group,secrets:command_secrets,close_supported,restore,fork_supported,models,events:command_events});
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
                            let _ = spawned.group.shutdown().await;
                            (result, status.await.ok())
                        }
                        Either::Right((status, protocol)) => {
                            // Descendants can retain stdout after the leader exits.
                            // Drain final protocol events, then terminate the entire group.
                            let _ = async_io::Timer::after(Duration::from_millis(250)).await;
                            let _ = spawned.group.shutdown().await;
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
            let result = match select(Box::pin(ready_rx), cancellation.cancelled()).await {
                Either::Left((result, _)) => result.unwrap_or_else(|_| Err(AgentError::Io("Agent exited during initialization".into()))),
                Either::Right(_) => Err(AgentError::Io("Agent startup cancelled".into())),
            };
            if result.is_ok() {
                connecting.0.take();
            } else {
                startup_group.shutdown().await.map_err(|error| AgentError::CleanupUnconfirmed(error.to_string()))?;
            }
            result
        }.boxed()
    }
}

/// How the agent reopens earlier sessions, if it can.
#[derive(Clone, Copy)]
enum Restore {
    Resume,
    Load,
    Unsupported,
}

struct Commands {
    connection: ConnectionTo<Agent>,
    stop: async_channel::Sender<()>,
    group: Arc<process::ProcessGroup>,
    secrets: Arc<Vec<String>>,
    close_supported: bool,
    restore: Restore,
    fork_supported: bool,
    models: Arc<Mutex<Models>>,
    events: async_channel::Sender<AgentEvent>,
}
impl AgentCommands for Commands {
    fn new_session(
        &self,
        request: acp::NewSessionRequest,
    ) -> BoxFuture<'static, Result<acp::NewSessionResponse, AgentError>> {
        let connection = self.connection.clone();
        let secrets = self.secrets.clone();
        let models = self.models.clone();
        async move {
            let message = UntypedMessage::new("session/new", request)
                .map_err(|error| map_error_with(error, &secrets))?;
            let raw = connection
                .send_request(message)
                .block_task()
                .await
                .map_err(|error| map_error_with(error, &secrets))?;
            models
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .session(raw)
        }
        .boxed()
    }
    fn restore_session(
        &self,
        request: nocterm_ai::RestoreSessionRequest,
    ) -> BoxFuture<'static, Result<acp::NewSessionResponse, AgentError>> {
        let connection = self.connection.clone();
        let secrets = self.secrets.clone();
        let models = self.models.clone();
        let restore = self.restore;
        let fork_supported = self.fork_supported;
        async move {
            let session_id = request.session_id.clone();
            if request.fork {
                if !fork_supported {
                    return Err(AgentError::RestoreUnavailable(
                        "The agent cannot fork chats.".into(),
                    ));
                }
                let message = UntypedMessage::new(
                    "session/fork",
                    acp::ForkSessionRequest::new(request.session_id, request.cwd)
                        .mcp_servers(request.mcp_servers),
                )
                .map_err(|error| restore_error(map_error_with(error, &secrets)))?;
                let raw = connection
                    .send_request(message)
                    .block_task()
                    .await
                    .map_err(|error| restore_error(map_error_with(error, &secrets)))?;
                return models
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .session(raw);
            }
            // Resuming skips the replay of history the panel already shows.
            let message = match restore {
                Restore::Resume => UntypedMessage::new(
                    "session/resume",
                    acp::ResumeSessionRequest::new(request.session_id, request.cwd)
                        .mcp_servers(request.mcp_servers),
                ),
                Restore::Load => UntypedMessage::new(
                    "session/load",
                    acp::LoadSessionRequest::new(request.session_id, request.cwd)
                        .mcp_servers(request.mcp_servers),
                ),
                Restore::Unsupported => {
                    return Err(AgentError::RestoreUnavailable(
                        "The agent cannot reopen earlier chats.".into(),
                    ));
                }
            }
            .map_err(|error| restore_error(map_error_with(error, &secrets)))?;
            let mut raw = connection
                .send_request(message)
                .block_task()
                .await
                .map_err(|error| restore_error(map_error_with(error, &secrets)))?;
            // Restored sessions keep their id; the answer does not repeat it.
            if let Some(object) = raw.as_object_mut() {
                object.insert("sessionId".into(), serde_json::json!(session_id.0));
            }
            models
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .session(raw)
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
        let models = self.models.clone();
        async move {
            let selection = models
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .selection(&request)?;
            if let Some(model) = selection {
                let message = UntypedMessage::new(
                    "session/set_model",
                    serde_json::json!({"sessionId":request.session_id,"modelId":model}),
                )
                .map_err(|error| map_error_with(error, &secrets))?;
                connection
                    .send_request(message)
                    .block_task()
                    .await
                    .map_err(|error| map_error_with(error, &secrets))?;
                return Ok(models
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .selected(&request));
            }
            let session = request.session_id.clone();
            let options = connection
                .send_request(request)
                .block_task()
                .await
                .map(|r| r.config_options)
                .map_err(|error| map_error_with(error, &secrets))?;
            Ok(models
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(&session, options))
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
    fn close_session(
        &self,
        session: acp::SessionId,
    ) -> BoxFuture<'static, Result<nocterm_ai::CloseSessionOutcome, AgentError>> {
        let connection = self.connection.clone();
        let supported = self.close_supported;
        let secrets = self.secrets.clone();
        let models = self.models.clone();
        let events = self.events.clone();
        async move {
            let result = close_session(connection, supported, session.clone(), &secrets).await;
            models
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&session);
            if matches!(result, Ok(nocterm_ai::CloseSessionOutcome::Closed)) {
                let (ack, barrier) = oneshot::channel();
                let fence = async {
                    events.send(AgentEvent::Barrier(ack)).await.ok()?;
                    barrier.await.ok()
                };
                match select(
                    Box::pin(fence),
                    Box::pin(async_io::Timer::after(Duration::from_secs(5))),
                )
                .await
                {
                    Either::Left((Some(()), _)) => {}
                    _ => return Ok(nocterm_ai::CloseSessionOutcome::TimedOut),
                }
            }
            result
        }
        .boxed()
    }
    fn shutdown_gracefully(&self) -> BoxFuture<'static, Result<(), AgentError>> {
        let group = self.group.clone();
        let stop = self.stop.clone();
        async move {
            let result = group.shutdown().await;
            let _ = stop.try_send(());
            result
        }
        .boxed()
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

async fn close_session(
    connection: ConnectionTo<Agent>,
    supported: bool,
    session: acp::SessionId,
    secrets: &[String],
) -> Result<nocterm_ai::CloseSessionOutcome, AgentError> {
    if !supported {
        return Ok(nocterm_ai::CloseSessionOutcome::Unsupported);
    }
    let close = connection
        .send_request(acp::CloseSessionRequest::new(session))
        .block_task();
    match select(
        Box::pin(close),
        Box::pin(async_io::Timer::after(Duration::from_secs(5))),
    )
    .await
    {
        Either::Left((result, _)) => result
            .map(|_| nocterm_ai::CloseSessionOutcome::Closed)
            .map_err(|error| map_error_with(error, secrets)),
        Either::Right(_) => Ok(nocterm_ai::CloseSessionOutcome::TimedOut),
    }
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
fn restore_error(error: AgentError) -> AgentError {
    let unavailable = match &error {
        AgentError::Rpc(error) => {
            error.code == acp::ErrorCode::MethodNotFound || {
                let message = error.message.to_lowercase();
                message.contains("session not found")
                    || message.contains("unknown session")
                    || message.contains("session does not exist")
                    || message.contains("no rollout found for thread id ")
            }
        }
        _ => false,
    };
    if unavailable {
        AgentError::RestoreUnavailable(error.to_string())
    } else {
        error
    }
}

#[cfg(test)]
fn map_error(error: acp::Error) -> AgentError {
    map_error_with(error, &[])
}

#[cfg(test)]
fn client_builder(
    events_tx: async_channel::Sender<AgentEvent>,
) -> agent_client_protocol::Builder<Client, impl agent_client_protocol::HandleDispatchFrom<Agent>> {
    client_builder_with_models(events_tx, Arc::new(Mutex::new(Models::default())))
}

fn client_builder_with_models(
    events_tx: async_channel::Sender<AgentEvent>,
    models: Arc<Mutex<Models>>,
) -> agent_client_protocol::Builder<Client, impl agent_client_protocol::HandleDispatchFrom<Agent>> {
    let notification_tx = events_tx.clone();
    let permission_tx = events_tx;
    Client
        .builder()
        .name("nocterm")
        .on_receive_notification(
            async move |mut notification: acp::SessionNotification, _| {
                if let acp::SessionUpdate::ConfigOptionUpdate(update) = &mut notification.update {
                    update.config_options =
                        models.lock().unwrap_or_else(|e| e.into_inner()).retain(
                            &notification.session_id,
                            std::mem::take(&mut update.config_options),
                        );
                }
                notification_tx
                    .try_send(AgentEvent::Session(notification))
                    .map_err(|_| acp::Error::internal_error().data("Agent event queue overflow"))
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: acp::RequestPermissionRequest, responder, connection| {
                let (respond, answer) = nocterm_ai::PermissionResponder::channel();
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
mod tests;

#[cfg(test)]
mod lifecycle_tests;
#[cfg(all(test, unix))]
mod restoration_intersections;
