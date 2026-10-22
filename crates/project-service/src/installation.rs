use std::sync::Arc;

use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Subscription, Task, WindowId};
use nocterm_project_model::{
    Catalog, ProjectDefinition, ProjectId, ProjectIdentity, ProjectMetadataSource,
};
use nocterm_store::{ShutdownCoordinator, SnapshotWriter};

use crate::{
    ClaimOutcome, PreparedProjects, ProjectBinding, ProjectClaim, ProjectsError,
    catalog::ProjectCatalog, claims::ProjectClaims,
};

struct InstalledProjects {
    catalog: Entity<ProjectCatalog>,
    claims: Entity<ProjectClaims>,
    _subscriptions: [Subscription; 2],
}

#[derive(Clone, Copy)]
struct FocusRequest {
    claim: ProjectClaim,
}
impl EventEmitter<FocusRequest> for InstalledProjects {}

impl InstalledProjects {
    fn new(
        catalog: Entity<ProjectCatalog>,
        claims: Entity<ProjectClaims>,
        cx: &mut Context<Self>,
    ) -> Self {
        let catalog_subscription = cx.observe(&catalog, |_, _, cx| cx.notify());
        let claim_subscription = cx.observe(&claims, |_, _, cx| cx.notify());
        Self {
            catalog,
            claims,
            _subscriptions: [catalog_subscription, claim_subscription],
        }
    }
}

/// Retained installed handle shared by all windows, with no mutable entity access.
#[derive(Clone)]
pub struct ProjectsHandle(Entity<InstalledProjects>);

impl ProjectsHandle {
    pub fn install(
        prepared: PreparedProjects,
        writer: SnapshotWriter<Catalog>,
        metadata: Arc<dyn ProjectMetadataSource>,
        shutdown: &ShutdownCoordinator,
        cx: &mut App,
    ) -> Result<Self, ProjectsError> {
        let claims = cx.new(ProjectClaims::new);
        let catalog =
            cx.new(|cx| ProjectCatalog::new(prepared, writer, metadata, claims.clone(), cx));
        let installed = cx.new(|cx| InstalledProjects::new(catalog, claims, cx));
        let retained = installed.clone();
        shutdown.register_capture(move |cx| {
            let catalog = retained.read(cx).catalog.clone();
            let claims = retained.read(cx).claims.clone();
            claims.update(cx, |claims, cx| claims.close(cx));
            catalog.update(cx, |catalog, cx| catalog.capture(cx))
        })?;
        Ok(Self(installed))
    }

    pub fn catalog<'a>(&self, cx: &'a App) -> &'a Catalog {
        self.0.read(cx).catalog.read(cx).catalog(cx)
    }

    pub fn error(&self, cx: &App) -> Option<ProjectsError> {
        self.0.read(cx).catalog.read(cx).error(cx)
    }

    /// Catalog mutation availability.
    pub fn writable(&self, cx: &App) -> bool {
        self.0.read(cx).catalog.read(cx).check_open(cx).is_ok()
    }

    pub fn new_identity(&self, cx: &App) -> Result<ProjectIdentity, ProjectsError> {
        self.0.read(cx).catalog.read(cx).new_identity(cx)
    }
    pub fn timestamp(&self, cx: &App) -> Result<u64, ProjectsError> {
        self.0.read(cx).catalog.read(cx).timestamp(cx)
    }

    pub fn binding(&self, project: ProjectId, cx: &App) -> Result<ProjectBinding, ProjectsError> {
        self.0.read(cx).catalog.read(cx).binding(project, cx)
    }

    pub fn check_binding(&self, binding: ProjectBinding, cx: &App) -> Result<(), ProjectsError> {
        self.0.read(cx).catalog.read(cx).check_binding(binding, cx)
    }

    pub fn save(
        &self,
        project: ProjectDefinition,
        expected: Option<ProjectDefinition>,
        cx: &mut App,
    ) -> Task<Result<ProjectBinding, ProjectsError>> {
        let catalog = self.0.read(cx).catalog.clone();
        catalog.update(cx, |catalog, cx| catalog.save(project, expected, cx))
    }

    pub fn remove(
        &self,
        expected: ProjectDefinition,
        cx: &mut App,
    ) -> Task<Result<(), ProjectsError>> {
        let catalog = self.0.read(cx).catalog.clone();
        catalog.update(cx, |catalog, cx| catalog.remove(expected, cx))
    }

    /// Reserve one window surface, including an unavailable/offline history view.
    pub fn claim(
        &self,
        project: ProjectId,
        window: WindowId,
        cx: &mut App,
    ) -> Result<ClaimOutcome, ProjectsError> {
        self.binding(project, cx)?;
        let claims = self.0.read(cx).claims.clone();
        claims.update(cx, |claims, cx| claims.claim(project, window, cx))
    }

    pub fn release_claim(&self, claim: ProjectClaim, cx: &mut App) -> Result<(), ProjectsError> {
        let claims = self.0.read(cx).claims.clone();
        claims.update(cx, |claims, cx| claims.release(claim, cx))?;
        Ok(())
    }

    pub fn check_claim(&self, claim: ProjectClaim, cx: &App) -> Result<(), ProjectsError> {
        self.0.read(cx).claims.read(cx).check(claim)
    }

    /// Ask the exact retained owner to focus its actual Item.
    pub fn request_focus(
        &self,
        project: ProjectId,
        window: WindowId,
        cx: &mut App,
    ) -> Result<(), ProjectsError> {
        let claim = self.0.read(cx).claims.read(cx).owner(project, window)?;
        self.0.update(cx, |_, cx| cx.emit(FocusRequest { claim }));
        Ok(())
    }

    pub fn observe_focus(
        &self,
        cx: &mut App,
        mut focus: impl FnMut(ProjectClaim, &mut App) + 'static,
    ) -> Subscription {
        cx.subscribe(&self.0, move |_, request: &FocusRequest, cx| {
            focus(request.claim, cx)
        })
    }

    pub fn observe(
        &self,
        cx: &mut App,
        mut changed: impl FnMut(&mut App) + 'static,
    ) -> Subscription {
        cx.observe(&self.0, move |_, cx| changed(cx))
    }
}
