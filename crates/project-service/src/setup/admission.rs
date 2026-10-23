//! Installation-scoped admission freezes with the catalog and drains owned cleanup.
use super::{SetupOwner, WorkspaceSetup};
use crate::ProjectsError;
use nocterm_project_providers::{ProjectPreparation, ProjectSetupRequest};
use nocterm_session::HostExec;
use std::sync::{Arc, Weak};

#[derive(Default)]
pub(crate) struct SetupAdmission {
    closing: bool,
    operations: Vec<Weak<SetupOwner>>,
}
impl SetupAdmission {
    pub(crate) fn admit(
        &mut self,
        preparation: Arc<dyn ProjectPreparation>,
        exec: Arc<dyn HostExec>,
        request: ProjectSetupRequest,
    ) -> Result<WorkspaceSetup, ProjectsError> {
        if self.closing {
            return Err(ProjectsError::Closing);
        }
        self.operations
            .retain(|operation| operation.strong_count() > 0);
        if self.operations.len() >= 32 {
            return Err(ProjectsError::Full);
        }
        let operation = WorkspaceSetup::new(preparation, exec, request);
        self.operations.push(Arc::downgrade(&operation.0));
        Ok(operation)
    }
    pub(crate) fn capture(
        &mut self,
    ) -> futures::future::BoxFuture<'static, Result<(), nocterm_store::StoreError>> {
        self.closing = true;
        let operations: Vec<_> = self
            .operations
            .drain(..)
            .filter_map(|operation| operation.upgrade().map(WorkspaceSetup))
            .collect();
        for operation in &operations {
            operation.0.cancel.cancel();
        }
        Box::pin(async move {
            let results = futures::future::join_all(
                operations
                    .into_iter()
                    .map(|operation| async move { operation.drain().await }),
            )
            .await;
            for result in results {
                result.map_err(nocterm_store::StoreError::backend)?;
            }
            Ok(())
        })
    }
}
