//! Shared bounded journal facades for injected retained native job owners.
use super::services::CoordinatorStorage;
use futures::{FutureExt, future::BoxFuture};
use nocterm_cloud_model::{
    coordinator::CoordinatorError,
    journal::{CleanupLease, JournalError},
    tool::*,
};
use nocterm_core::SharedCause;

pub(super) struct BackgroundJournal {
    storage: CoordinatorStorage,
}
impl BackgroundJournal {
    pub(super) fn new(storage: CoordinatorStorage) -> Self {
        Self { storage }
    }
}
impl BackgroundCommandJournal for BackgroundJournal {
    fn retain(
        &self,
        command: BackgroundCommand,
    ) -> BoxFuture<'static, Result<(), ToolExecutionError>> {
        let storage = self.storage.clone();
        async move {
            if !storage
                .binding()
                .contains(&command.dispatch().intent().authority().run().stream())
            {
                return Err(ToolExecutionError::Revoked);
            }
            storage
                .io()
                .tool(move |journal| journal.retain(command))
                .await
                .map_err(tool_error)
        }
        .boxed()
    }
    fn finish(
        &self,
        command: BackgroundCommand,
        outcome: BackgroundCommandOutcome,
    ) -> BoxFuture<'static, Result<(), ToolExecutionError>> {
        let storage = self.storage.clone();
        async move {
            if !storage
                .binding()
                .contains(&command.dispatch().intent().authority().run().stream())
            {
                return Err(ToolExecutionError::Revoked);
            }
            storage
                .io()
                .tool(move |journal| journal.finish(command, outcome))
                .await
                .map_err(tool_error)
        }
        .boxed()
    }
}
pub(super) struct BackgroundCleanup {
    storage: CoordinatorStorage,
}
impl BackgroundCleanup {
    pub(super) fn new(storage: CoordinatorStorage) -> Self {
        Self { storage }
    }
}
impl BackgroundCommandCleanup for BackgroundCleanup {
    fn finish(
        &self,
        command: BackgroundCommand,
        outcome: BackgroundCommandOutcome,
    ) -> BoxFuture<'static, Result<(), ToolExecutionError>> {
        let storage = self.storage.clone();
        async move {
            let stream = command.dispatch().intent().authority().run().stream();
            if !storage.binding().contains(&stream) {
                return Err(CoordinatorError::StaleSource);
            }
            // The original admitted dispatch is not current authority. Only an
            // actually acquired writer owned by this dispatcher supplies a view;
            // a SQL head observation never silently adopts another live worker.
            let lease = storage
                .io()
                .owned_writer(&stream)
                .ok_or(JournalError::Fenced)?;
            let disk = storage.io().acquire().await?;
            let lookup = stream.clone();
            let head = storage
                .io()
                .locked(&disk, move |journal| journal.head(lookup))
                .await?
                .ok_or(JournalError::NotFound)?;
            if head.lease() != &lease {
                return Err(JournalError::Fenced.into());
            }
            storage
                .io()
                .cleanup_locked(&disk, move |cleanup| {
                    cleanup.finish_background(CleanupLease::for_writer(lease), command, outcome)
                })
                .await
        }
        .map(|result| result.map_err(tool_error))
        .boxed()
    }
}
fn tool_error(error: CoordinatorError) -> ToolExecutionError {
    match error {
        CoordinatorError::Tool(error) => error,
        CoordinatorError::Overloaded | CoordinatorError::Journal(JournalError::Capacity) => {
            ToolExecutionError::Capacity
        }
        CoordinatorError::Journal(JournalError::Fenced)
        | CoordinatorError::StaleSource
        | CoordinatorError::Stopped => ToolExecutionError::Revoked,
        error => ToolExecutionError::Execution(SharedCause::new(error)),
    }
}
