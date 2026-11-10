use std::{cell::Cell, rc::Rc, sync::Arc};

use futures::future::BoxFuture;
use gpui::{App, AppContext as _, Context, Entity, EntityId, Subscription, Task};
use nocterm_project_model::{
    Catalog, CatalogError, CatalogState, ProjectDefinition, ProjectId, ProjectIdentity,
    ProjectMetadataSource, ProjectStamp, ProjectStorageError,
};
use nocterm_store::{Persisted, Publication, SnapshotWriter, StoreError};

use crate::{PreparedProjects, ProjectsError, claims::ProjectClaims};

const MAX_PENDING: usize = 32;

/// Original committed folder and parent connection generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectBinding {
    project: ProjectId,
    stamp: ProjectStamp,
    owner: EntityId,
}

impl ProjectBinding {
    pub fn project(self) -> ProjectId {
        self.project
    }

    pub fn stamp(self) -> ProjectStamp {
        self.stamp
    }
}

pub(crate) struct ProjectCatalog {
    state: Entity<Persisted<CatalogState>>,
    metadata: Arc<dyn ProjectMetadataSource>,
    load_error: Option<Arc<ProjectStorageError>>,
    persistent: bool,
    _subscription: Subscription,
    identity: EntityId,
}

impl ProjectCatalog {
    pub(crate) fn new(
        prepared: PreparedProjects,
        writer: SnapshotWriter<Catalog>,
        metadata: Arc<dyn ProjectMetadataSource>,
        claims: Entity<ProjectClaims>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (catalog, load_error, persistent) = prepared.into_parts();
        let writer: SnapshotWriter<CatalogState> =
            Arc::new(move |state| writer(state.into_catalog()));
        let claims = claims.downgrade();
        let state = cx.new(|_| {
            Persisted::new(
                CatalogState::new(catalog, 1),
                writer,
                Publication::Durable,
                MAX_PENDING,
            )
            .with_publication_hook(move |_, next, cx| {
                let _ = claims.update(cx, |claims, cx| claims.published(next.catalog(), cx));
            })
        });
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            metadata,
            load_error,
            persistent,
            _subscription: subscription,
            identity: cx.entity_id(),
        }
    }

    pub(crate) fn catalog<'a>(&self, cx: &'a App) -> &'a Catalog {
        self.state.read(cx).visible().catalog()
    }

    pub(crate) fn binding(
        &self,
        project: ProjectId,
        cx: &App,
    ) -> Result<ProjectBinding, ProjectsError> {
        self.check_open(cx)?;
        self.state
            .read(cx)
            .visible()
            .stamp(project)
            .map(|stamp| ProjectBinding {
                project,
                stamp,
                owner: self.identity,
            })
            .ok_or_else(|| CatalogError::Missing(project).into())
    }

    pub(crate) fn check_binding(
        &self,
        binding: ProjectBinding,
        cx: &App,
    ) -> Result<(), ProjectsError> {
        if self.binding(binding.project, cx)? != binding {
            return Err(ProjectsError::StaleBinding(binding.project));
        }
        Ok(())
    }

    pub(crate) fn check_open(&self, cx: &App) -> Result<(), ProjectsError> {
        if self.state.read(cx).is_closing() {
            return Err(ProjectsError::Closing);
        }
        if let Some(error) = &self.load_error {
            return Err(ProjectsError::read_only(error.clone()));
        }
        Ok(())
    }

    pub(crate) fn error(&self, cx: &App) -> Option<ProjectsError> {
        self.load_error
            .as_ref()
            .map(|error| ProjectsError::read_only(error.clone()))
            .or_else(|| {
                self.state
                    .read(cx)
                    .error()
                    .cloned()
                    .map(ProjectsError::from)
            })
    }

    pub(crate) fn timestamp(&self, cx: &App) -> Result<u64, ProjectsError> {
        self.check_open(cx)?;
        Ok(self.metadata.timestamp())
    }
    pub(crate) fn new_identity(&self, cx: &App) -> Result<ProjectIdentity, ProjectsError> {
        self.check_open(cx)?;
        self.metadata.new_identity().map_err(Into::into)
    }

    pub(crate) fn save(
        &mut self,
        project: ProjectDefinition,
        expected: Option<ProjectDefinition>,
        cx: &mut Context<Self>,
    ) -> Task<Result<ProjectBinding, ProjectsError>> {
        let id = project.id();
        let owner = self.identity;
        self.edit(
            move |state| {
                state
                    .save(project, expected.as_ref())
                    .map(|stamp| ProjectBinding {
                        project: id,
                        stamp,
                        owner,
                    })
            },
            cx,
        )
    }

    pub(crate) fn remove(
        &mut self,
        expected: ProjectDefinition,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ProjectsError>> {
        self.edit(move |state| state.remove(&expected), cx)
    }

    pub(crate) fn remove_bound(
        &mut self,
        expected: ProjectDefinition,
        binding: ProjectBinding,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), ProjectsError>> {
        if let Err(error) = self.check_binding(binding, cx) {
            return Task::ready(Err(error));
        }
        self.edit(
            move |state| {
                if state.stamp(expected.id()) != Some(binding.stamp) {
                    return Err(CatalogError::Conflict(expected.id()));
                }
                state.remove(&expected)
            },
            cx,
        )
    }

    pub(crate) fn capture(
        &mut self,
        cx: &mut Context<Self>,
    ) -> BoxFuture<'static, Result<(), StoreError>> {
        self.state.update(cx, |state, _| state.capture_shutdown())
    }

    fn edit<T: 'static>(
        &mut self,
        edit: impl FnOnce(&mut CatalogState) -> Result<T, CatalogError> + 'static,
        cx: &mut Context<Self>,
    ) -> Task<Result<T, ProjectsError>> {
        if let Err(error) = self.check_open(cx) {
            return Task::ready(Err(error));
        }
        let result = Rc::new(Cell::new(None));
        let completed = result.clone();
        let apply = move |state: &mut CatalogState| {
            completed.set(Some(edit(state).map_err(StoreError::edit)?));
            Ok(())
        };
        if !self.persistent {
            let applied = self
                .state
                .update(cx, |state, cx| state.update_immediate_checked(apply, cx));
            return Task::ready(
                applied
                    .map_err(ProjectsError::from)
                    .and_then(|()| result.take().ok_or(ProjectsError::Interrupted)),
            );
        }
        match self
            .state
            .update(cx, |state, cx| state.update_checked(apply, cx))
        {
            Err(error) => Task::ready(Err(error.into())),
            Ok(receipt) => cx.spawn(async move |_, _| {
                receipt.wait().await.map_err(ProjectsError::from)?;
                result.take().ok_or(ProjectsError::Interrupted)
            }),
        }
    }
}
