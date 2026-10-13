//! An open attempt retains original commands, authority and bounded restore fallback.
use super::SessionAuthority;
use crate::thread::Restore;
use nocterm_ai::{
    AgentCommands, AgentError, acp,
    session::{RESTORE_ATTEMPTS, RestoreFallback, restore_fallback},
};
use std::{path::PathBuf, sync::Arc, time::Duration};
pub(in crate::thread) struct OpenSessionOperation {
    commands: Arc<dyn AgentCommands>,
    authority: SessionAuthority,
    restore: Option<Restore>,
    workdir: PathBuf,
    servers: Vec<acp::McpServer>,
    executor: gpui_kit::BackgroundExecutor,
}
pub(in crate::thread) struct OpenSessionCompletion {
    authority: SessionAuthority,
    result: Result<acp::NewSessionResponse, AgentError>,
    restored: Option<bool>,
    workdir: PathBuf,
    pending_restore: Option<Restore>,
}
impl OpenSessionCompletion {
    pub(in crate::thread) fn authority(&self) -> &SessionAuthority {
        &self.authority
    }
    pub(in crate::thread) fn orphan(&self) -> Option<acp::SessionId> {
        self.result
            .as_ref()
            .ok()
            .map(|response| response.session_id.clone())
    }
    pub(in crate::thread) fn into_parts(
        self,
    ) -> (
        SessionAuthority,
        Result<acp::NewSessionResponse, AgentError>,
        Option<bool>,
        PathBuf,
        Option<Restore>,
    ) {
        (
            self.authority,
            self.result,
            self.restored,
            self.workdir,
            self.pending_restore,
        )
    }
}
impl OpenSessionOperation {
    pub(in crate::thread) fn new(
        commands: Arc<dyn AgentCommands>,
        authority: SessionAuthority,
        restore: Option<Restore>,
        workdir: PathBuf,
        servers: Vec<acp::McpServer>,
        executor: gpui_kit::BackgroundExecutor,
    ) -> Self {
        Self {
            commands,
            authority,
            restore,
            workdir,
            servers,
            executor,
        }
    }
    pub(in crate::thread) async fn run(self) -> OpenSessionCompletion {
        let (result, restored) = self.open().await;
        OpenSessionCompletion {
            authority: self.authority,
            result,
            restored,
            workdir: self.workdir,
            pending_restore: self.restore,
        }
    }
    async fn open(&self) -> (Result<acp::NewSessionResponse, AgentError>, Option<bool>) {
        if let Some(restore) = &self.restore {
            for attempt in 0..RESTORE_ATTEMPTS {
                let error = match self
                    .commands
                    .restore_session(nocterm_ai::RestoreSessionRequest {
                        session_id: restore.session.clone(),
                        cwd: restore.workdir.clone(),
                        mcp_servers: self.servers.clone(),
                        fork: restore.fork,
                    })
                    .await
                {
                    Ok(response) => return (Ok(response), Some(true)),
                    Err(error) => error,
                };
                match restore_fallback(&error, attempt, restore.fork) {
                    RestoreFallback::SignIn | RestoreFallback::Fail => return (Err(error), None),
                    RestoreFallback::Retry => self.executor.timer(Duration::from_millis(250)).await,
                    RestoreFallback::StartFresh => {
                        tracing::warn!(%error,fork=restore.fork,"agent could not reopen the saved session; starting a new one");
                        break;
                    }
                }
            }
        }
        let result = self
            .commands
            .new_session(
                acp::NewSessionRequest::new(self.workdir.clone()).mcp_servers(self.servers.clone()),
            )
            .await;
        (result, self.restore.is_some().then_some(false))
    }
}
