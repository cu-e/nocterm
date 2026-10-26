//! Retained explicit resource action followed by checked durable catalog removal.
mod admission;
use crate::{ProjectBinding, ProjectsError, WorkspaceRemovalError};
pub(crate) use admission::RemovalAdmission;
use futures::{FutureExt as _, future::BoxFuture};
use nocterm_project_model::ProjectDefinition;
use nocterm_project_providers::{
    ContainerAction, ContainerActionError, ContainerActionKey, ContainerActionOutcome,
    ContainerActionSource, WorkspaceContainerActions,
};
use nocterm_session::local_fs::CancelFlag;
use std::sync::{Arc, Mutex};

/// Explicit action plus captured original host. Absent means metadata only.
#[derive(Clone)]
pub struct RemovalContainerAction {
    port: Arc<dyn WorkspaceContainerActions>,
    source: ContainerActionSource,
    action: ContainerAction,
}
impl RemovalContainerAction {
    pub fn new(
        port: Arc<dyn WorkspaceContainerActions>,
        source: ContainerActionSource,
        action: ContainerAction,
    ) -> Self {
        Self {
            port,
            source,
            action,
        }
    }
    pub fn source(&self) -> &ContainerActionSource {
        &self.source
    }
}
#[derive(Default)]
struct Progress {
    pending: bool,
    native_running: bool,
    outcome: Option<ContainerActionOutcome>,
    result: Option<Result<(), WorkspaceRemovalError>>,
}
struct RemovalReceipt {
    state: Mutex<Progress>,
    cancel: CancelFlag,
    changed: event_listener::Event,
}
/// Survives view receivers; acknowledged actions can only retry metadata publication.
#[derive(Clone)]
pub struct RemovalOperation {
    expected: ProjectDefinition,
    binding: ProjectBinding,
    key: Option<ContainerActionKey>,
    receipt: Arc<RemovalReceipt>,
}
impl RemovalOperation {
    pub fn outcome(&self) -> Option<ContainerActionOutcome> {
        self.receipt.state.lock().ok()?.outcome
    }
    pub fn is_pending(&self) -> bool {
        self.receipt.state.lock().is_ok_and(|state| state.pending)
    }
    pub fn wait(&self) -> BoxFuture<'static, Result<(), WorkspaceRemovalError>> {
        let receipt = self.receipt.clone();
        async move {
            loop {
                let listener = receipt.changed.listen();
                let result = receipt
                    .state
                    .lock()
                    .map_err(|_| WorkspaceRemovalError::Catalog(ProjectsError::Interrupted))?
                    .result
                    .clone();
                if let Some(result) = result {
                    return result;
                }
                listener.await;
            }
        }
        .boxed()
    }
    pub(crate) fn expected(&self) -> &ProjectDefinition {
        &self.expected
    }
    pub(crate) fn binding(&self) -> ProjectBinding {
        self.binding
    }
    pub(crate) fn reserve_retry(&self) -> Result<(), WorkspaceRemovalError> {
        let mut state = self
            .receipt
            .state
            .lock()
            .map_err(|_| ProjectsError::Interrupted)?;
        if state.pending {
            return Err(ContainerActionError::Busy.into());
        }
        if !matches!(state.result, Some(Err(WorkspaceRemovalError::Catalog(_)))) {
            return Err(ProjectsError::Interrupted.into());
        }
        state.pending = true;
        state.result = None;
        Ok(())
    }
    pub(crate) fn finish(&self, result: Result<(), WorkspaceRemovalError>) {
        if let Ok(mut state) = self.receipt.state.lock() {
            state.pending = false;
            state.result = Some(result);
        }
        self.receipt.changed.notify(usize::MAX);
    }
    pub(crate) fn execute(
        &self,
        action: RemovalContainerAction,
        executor: &gpui::BackgroundExecutor,
    ) -> gpui::Task<Result<(), WorkspaceRemovalError>> {
        let receipt = self.receipt.clone();
        executor.spawn(async move {
            let result = action
                .port
                .apply(action.source, action.action, receipt.cancel.clone())
                .await;
            if let Ok(mut state) = receipt.state.lock() {
                state.native_running = false;
                if let Ok(outcome) = &result {
                    state.outcome = Some(*outcome);
                }
            }
            receipt.changed.notify(usize::MAX);
            result.map(|_| ()).map_err(WorkspaceRemovalError::Action)
        })
    }
}
