//! Original draft publication retains preparation across a failed catalog write.
use super::{ProjectEditor, ProjectOpenError};
use gpui_kit::{AsyncWindowContext, WeakEntity};
use nocterm_explorer::directory::{Directory, DirectoryRead};
use nocterm_project_model::{ProjectDefinition, ProjectDraft};
use nocterm_project_service::WorkspaceSetup;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct ProjectPublication {
    candidate: ProjectDefinition,
    expected: Option<ProjectDefinition>,
    operation: Option<WorkspaceSetup>,
    directory_check: Option<(Arc<dyn DirectoryRead>, Directory)>,
    dismissed: Arc<AtomicBool>,
}
impl ProjectPublication {
    pub(super) fn new(
        candidate: ProjectDefinition,
        expected: Option<ProjectDefinition>,
        operation: Option<WorkspaceSetup>,
        directory_check: Option<(Arc<dyn DirectoryRead>, Directory)>,
        dismissed: Arc<AtomicBool>,
    ) -> Self {
        Self {
            candidate,
            expected,
            operation,
            directory_check,
            dismissed,
        }
    }
    pub(super) async fn run(
        self,
        editor: &WeakEntity<ProjectEditor>,
        cx: &mut AsyncWindowContext,
    ) -> Result<(), ProjectOpenError> {
        let result = self.publish(editor, cx).await;
        if let Some(operation) = self.operation {
            if result.is_ok() {
                operation.committed();
            } else {
                operation.publication_failed();
            }
            if self.dismissed.load(Ordering::Acquire)
                && let Err(error) = cx
                    .background_executor()
                    .spawn(async move { operation.discard().await })
                    .await
            {
                tracing::warn!(?error, "workspace cleanup failed");
            }
        }
        result
    }
    async fn publish(
        &self,
        editor: &WeakEntity<ProjectEditor>,
        cx: &mut AsyncWindowContext,
    ) -> Result<(), ProjectOpenError> {
        if let Some((reader, directory)) = self.directory_check.clone() {
            cx.background_executor()
                .spawn(async move { reader.list(directory, Default::default()).await })
                .await?;
        }
        let target = self.prepare(editor, cx).await?;
        if self.dismissed.load(Ordering::Acquire) {
            return Err(nocterm_project_providers::ProjectSetupError::Canceled.into());
        }
        let mut draft = ProjectDraft::from(self.candidate.clone());
        if let Some(target) = target {
            draft.target = target;
        }
        let candidate = ProjectDefinition::try_from(draft)?;
        if let Some(operation) = &self.operation {
            operation.publishing();
        }
        let saved = editor
            .update_in(cx, |this, _, cx| {
                this.installed
                    .model()
                    .save(candidate, self.expected.clone(), cx)
            })
            .map_err(|_| nocterm_project_providers::ProjectSetupError::Canceled)?;
        saved.await?;
        Ok(())
    }
    async fn prepare(
        &self,
        editor: &WeakEntity<ProjectEditor>,
        cx: &mut AsyncWindowContext,
    ) -> Result<Option<nocterm_project_model::ProjectTarget>, ProjectOpenError> {
        let Some(operation) = self.operation.clone() else {
            return Ok(None);
        };
        let (send, receive) = async_channel::bounded(1);
        let latest = receive.clone();
        let task = cx.background_executor().spawn(async move {
            operation
                .prepare(Arc::new(move |phase| {
                    if let Err(async_channel::TrySendError::Full(phase)) = send.try_send(phase) {
                        let _ = latest.try_recv();
                        let _ = send.try_send(phase);
                    }
                }))
                .await
        });
        while let Ok(phase) = receive.recv().await {
            let _ = editor.update_in(cx, |this, _, cx| {
                this.phase = Some(phase);
                cx.notify();
            });
        }
        Ok(Some(task.await?))
    }
}
