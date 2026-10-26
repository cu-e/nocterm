use super::{Progress, RemovalOperation, RemovalReceipt};
use crate::{ProjectBinding, ProjectsError, WorkspaceRemovalError};
use nocterm_project_model::ProjectDefinition;
use nocterm_project_providers::{ContainerActionError, ContainerActionSource};
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct RemovalAdmission {
    closing: bool,
    operations: Vec<RemovalOperation>,
}
impl RemovalAdmission {
    pub(crate) fn admit(
        &mut self,
        expected: ProjectDefinition,
        binding: ProjectBinding,
        source: Option<&ContainerActionSource>,
    ) -> Result<RemovalOperation, WorkspaceRemovalError> {
        self.check_open()?;
        self.operations.retain(RemovalOperation::is_pending);
        let key = source.map(ContainerActionSource::key);
        if self.operations.len() >= 32
            || key.as_ref().is_some_and(|key| {
                self.operations
                    .iter()
                    .any(|op| op.key.as_ref() == Some(key))
            })
        {
            return Err(ContainerActionError::Busy.into());
        }
        let operation = RemovalOperation {
            expected,
            binding,
            key,
            receipt: Arc::new(RemovalReceipt {
                state: std::sync::Mutex::new(Progress {
                    pending: true,
                    native_running: source.is_some(),
                    ..Default::default()
                }),
                cancel: Default::default(),
                changed: event_listener::Event::new(),
            }),
        };
        self.operations.push(operation.clone());
        Ok(operation)
    }
    pub(crate) fn check_open(&self) -> Result<(), WorkspaceRemovalError> {
        if self.closing {
            return Err(ProjectsError::Closing.into());
        }
        Ok(())
    }
    pub(crate) fn retain_retry(
        &mut self,
        operation: &RemovalOperation,
    ) -> Result<(), WorkspaceRemovalError> {
        self.check_open()?;
        self.operations.retain(RemovalOperation::is_pending);
        if self.operations.len() >= 32
            || operation.key.as_ref().is_some_and(|key| {
                self.operations
                    .iter()
                    .any(|op| op.key.as_ref() == Some(key))
            })
        {
            return Err(ContainerActionError::Busy.into());
        }
        operation.reserve_retry()?;
        self.operations.push(operation.clone());
        Ok(())
    }
    pub(crate) fn capture(
        &mut self,
    ) -> futures::future::BoxFuture<'static, Result<(), nocterm_store::StoreError>> {
        self.closing = true;
        let receipts: Vec<_> = self.operations.drain(..).map(|op| op.receipt).collect();
        for receipt in &receipts {
            receipt.cancel.cancel();
        }
        Box::pin(async move {
            for receipt in receipts {
                loop {
                    let listener = receipt.changed.listen();
                    if !receipt.state.lock().is_ok_and(|state| state.native_running) {
                        break;
                    }
                    listener.await;
                }
            }
            Ok(())
        })
    }
}
