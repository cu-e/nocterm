//! An open attempt retains original commands, authority and bounded restore fallback.
use super::{SessionAuthority, settle};
use crate::thread::Restore;
use nocterm_ai::{
    connection::ManagedCommandsHandle,
    history::source::{ManagedCommandError, ManagedOpened},
    model::{
        ids::SessionId,
        session::{ExternalToolEndpoint, RestoreSession, SessionOpen},
    },
    session::{RESTORE_ATTEMPTS, RestoreFallback, managed_restore_fallback},
};
use std::{path::PathBuf, time::Duration};
pub(in crate::thread) struct OpenSessionOperation {
    commands: ManagedCommandsHandle,
    authority: SessionAuthority,
    restore: Option<Restore>,
    workdir: PathBuf,
    servers: Vec<ExternalToolEndpoint>,
    executor: gpui_kit::BackgroundExecutor,
}
pub(in crate::thread) struct OpenSessionCompletion {
    authority: SessionAuthority,
    result: Result<ManagedOpened, ManagedCommandError>,
    restored: Option<bool>,
    workdir: PathBuf,
    pending_restore: Option<Restore>,
}
/// The parts of a finished open attempt: authority, result, whether it was a
/// restore, its working directory and the restore still pending.
pub(in crate::thread) type OpenedParts = (
    SessionAuthority,
    Result<ManagedOpened, ManagedCommandError>,
    Option<bool>,
    PathBuf,
    Option<Restore>,
);
impl OpenSessionCompletion {
    pub(in crate::thread) fn authority(&self) -> &SessionAuthority {
        &self.authority
    }
    pub(in crate::thread) fn orphan(&self) -> Option<SessionId> {
        self.result
            .as_ref()
            .ok()
            .map(|response| response.opened().session.clone())
    }
    pub(in crate::thread) fn into_parts(self) -> OpenedParts {
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
        commands: ManagedCommandsHandle,
        authority: SessionAuthority,
        restore: Option<Restore>,
        workdir: PathBuf,
        servers: Vec<ExternalToolEndpoint>,
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
    async fn open(&self) -> (Result<ManagedOpened, ManagedCommandError>, Option<bool>) {
        if let Some(restore) = &self.restore {
            for attempt in 0..RESTORE_ATTEMPTS {
                let request = RestoreSession {
                    session: restore.session.clone(),
                    working_directory: restore.workdir.clone(),
                    endpoints: self.servers.clone(),
                    fork: restore.fork,
                };
                let error = match settle(self.commands.restore_session(&request)).await {
                    Ok(response) => return (Ok(response), Some(true)),
                    Err(error) => error,
                };
                match managed_restore_fallback(&error, attempt, restore.fork) {
                    RestoreFallback::SignIn | RestoreFallback::Fail => return (Err(error), None),
                    RestoreFallback::Retry => self.executor.timer(Duration::from_millis(250)).await,
                    RestoreFallback::StartFresh => {
                        tracing::warn!(%error,fork=restore.fork,"agent could not reopen the saved session; starting a new one");
                        break;
                    }
                }
            }
        }
        let request = SessionOpen {
            working_directory: self.workdir.clone(),
            endpoints: self.servers.clone(),
        };
        let result = settle(self.commands.new_session(&request)).await;
        (result, self.restore.is_some().then_some(false))
    }
}
