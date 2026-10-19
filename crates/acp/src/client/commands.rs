//! Agent commands over a live protocol connection.
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_client_protocol::{Agent, ConnectionTo, UntypedMessage};
use agent_client_protocol_schema::v1 as acp;
use futures::{
    FutureExt,
    channel::oneshot,
    future::{BoxFuture, Either, select},
};
use nocterm_ai::{
    AgentCommands, AgentError, AgentEvent,
    model::{
        controls::ConfigOption,
        ids::{AuthMethodId, SessionId},
        session::{
            PromptOutcome, PromptRequest, RestoreSession, SessionOpen, SessionOpened,
            SetConfigOption, SetMode,
        },
    },
};

use super::{
    handshake::SessionSupport,
    remote_errors::{map_error_with, restore_error},
};
use crate::{mapping, models::Models, process};

/// How the agent reopens earlier sessions, if it can.
#[derive(Clone, Copy)]
enum Restore {
    Resume,
    Load,
    Unsupported,
}

pub(super) struct Commands {
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
impl Commands {
    pub(super) fn new(
        connection: ConnectionTo<Agent>,
        stop: async_channel::Sender<()>,
        group: Arc<process::ProcessGroup>,
        secrets: Arc<Vec<String>>,
        support: SessionSupport,
        models: Arc<Mutex<Models>>,
        events: async_channel::Sender<AgentEvent>,
    ) -> Self {
        let restore = if support.resume {
            Restore::Resume
        } else if support.load {
            Restore::Load
        } else {
            Restore::Unsupported
        };
        Self {
            connection,
            stop,
            group,
            secrets,
            close_supported: support.close,
            restore,
            fork_supported: support.fork,
            models,
            events,
        }
    }
}
impl AgentCommands for Commands {
    fn new_session(
        &self,
        request: SessionOpen,
    ) -> BoxFuture<'static, Result<SessionOpened, AgentError>> {
        let connection = self.connection.clone();
        let secrets = self.secrets.clone();
        let models = self.models.clone();
        async move {
            let message = UntypedMessage::new("session/new", mapping::new_session(request))
                .map_err(|error| map_error_with(error, &secrets))?;
            let raw = connection
                .send_request(message)
                .block_task()
                .await
                .map_err(|error| map_error_with(error, &secrets))?;
            let response = models
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .session(raw)?;
            Ok(mapping::opened(response))
        }
        .boxed()
    }
    fn restore_session(
        &self,
        request: RestoreSession,
    ) -> BoxFuture<'static, Result<SessionOpened, AgentError>> {
        let connection = self.connection.clone();
        let secrets = self.secrets.clone();
        let models = self.models.clone();
        let restore = self.restore;
        let fork_supported = self.fork_supported;
        async move {
            let session_id = mapping::wire_session(&request.session);
            let working_directory = request.working_directory;
            let servers = mapping::servers(request.endpoints);
            if request.fork {
                if !fork_supported {
                    return Err(AgentError::RestoreUnavailable(
                        "The agent cannot fork chats.".into(),
                    ));
                }
                let message = UntypedMessage::new(
                    "session/fork",
                    acp::ForkSessionRequest::new(session_id, working_directory)
                        .mcp_servers(servers),
                )
                .map_err(|error| restore_error(map_error_with(error, &secrets)))?;
                let raw = connection
                    .send_request(message)
                    .block_task()
                    .await
                    .map_err(|error| restore_error(map_error_with(error, &secrets)))?;
                let response = models
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .session(raw)?;
                return Ok(mapping::opened(response));
            }
            // Resuming skips the replay of history the panel already shows.
            let message = match restore {
                Restore::Resume => UntypedMessage::new(
                    "session/resume",
                    acp::ResumeSessionRequest::new(session_id.clone(), working_directory)
                        .mcp_servers(servers),
                ),
                Restore::Load => UntypedMessage::new(
                    "session/load",
                    acp::LoadSessionRequest::new(session_id.clone(), working_directory)
                        .mcp_servers(servers),
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
            let response = models
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .session(raw)?;
            Ok(mapping::opened(response))
        }
        .boxed()
    }
    fn prompt(
        &self,
        request: PromptRequest,
    ) -> BoxFuture<'static, Result<PromptOutcome, AgentError>> {
        let connection = self.connection.clone();
        let secrets = self.secrets.clone();
        async move {
            let response = connection
                .send_request(mapping::prompt(request))
                .block_task()
                .await
                .map_err(|error| map_error_with(error, &secrets))?;
            Ok(mapping::prompt_outcome(response)?)
        }
        .boxed()
    }
    fn cancel(&self, session: SessionId) {
        let _ = self
            .connection
            .send_notification(acp::CancelNotification::new(mapping::wire_session(
                &session,
            )));
    }
    fn set_mode(&self, request: SetMode) -> BoxFuture<'static, Result<(), AgentError>> {
        let connection = self.connection.clone();
        let secrets = self.secrets.clone();
        async move {
            connection
                .send_request(mapping::set_mode(request))
                .block_task()
                .await
                .map(|_| ())
                .map_err(|error| map_error_with(error, &secrets))
        }
        .boxed()
    }
    fn set_config_option(
        &self,
        request: SetConfigOption,
    ) -> BoxFuture<'static, Result<Vec<ConfigOption>, AgentError>> {
        let connection = self.connection.clone();
        let secrets = self.secrets.clone();
        let models = self.models.clone();
        async move {
            let request = mapping::set_config(request);
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
                return Ok(mapping::options(
                    models
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .selected(&request),
                ));
            }
            let session = request.session_id.clone();
            let options = connection
                .send_request(request)
                .block_task()
                .await
                .map(|r| r.config_options)
                .map_err(|error| map_error_with(error, &secrets))?;
            Ok(mapping::options(
                models
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .retain(&session, options),
            ))
        }
        .boxed()
    }
    fn authenticate(&self, method: AuthMethodId) -> BoxFuture<'static, Result<(), AgentError>> {
        let connection = self.connection.clone();
        let secrets = self.secrets.clone();
        async move {
            connection
                .send_request(acp::AuthenticateRequest::new(method.to_string()))
                .block_task()
                .await
                .map(|_| ())
                .map_err(|error| map_error_with(error, &secrets))
        }
        .boxed()
    }
    fn close_session(
        &self,
        session: SessionId,
    ) -> BoxFuture<'static, Result<nocterm_ai::CloseSessionOutcome, AgentError>> {
        let session = mapping::wire_session(&session);
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

pub(super) async fn close_session(
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
