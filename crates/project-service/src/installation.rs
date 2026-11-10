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
    setups: crate::setup::SetupAdmission,
    environments: crate::environment::EnvironmentAdmission,
    removals: crate::removal::RemovalAdmission,
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
            setups: Default::default(),
            environments: Default::default(),
            removals: Default::default(),
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
            let catalog = catalog.update(cx, |catalog, cx| catalog.capture(cx));
            let setups = retained.update(cx, |installed, _| installed.setups.capture());
            let environments = retained.read(cx).environments.capture();
            let removals = retained.update(cx, |installed, _| installed.removals.capture());
            Box::pin(async move {
                let (catalog, setups, environments, removals) =
                    futures::join!(catalog, setups, environments, removals);
                catalog?;
                setups?;
                environments?;
                removals
            })
        })?;
        Ok(Self(installed))
    }

    pub fn environment_operation(
        &self,
        port: Arc<dyn nocterm_project_providers::WorkspaceEnvironment>,
        source: nocterm_project_providers::EnvironmentSource,
        cx: &mut App,
    ) -> Result<crate::EnvironmentOperation, nocterm_project_providers::EnvironmentError> {
        self.0
            .read(cx)
            .catalog
            .read(cx)
            .check_open(cx)
            .map_err(|_| nocterm_project_providers::EnvironmentError::Unavailable)?;
        self.0
            .read(cx)
            .environments
            .admit(port, source, cx.background_executor().clone())
    }

    pub fn prepare_workspace(
        &self,
        preparation: Arc<dyn nocterm_project_providers::ProjectPreparation>,
        exec: Arc<dyn nocterm_session::HostExec>,
        request: nocterm_project_providers::ProjectSetupRequest,
        cx: &mut App,
    ) -> Result<crate::WorkspaceSetup, ProjectsError> {
        self.0.read(cx).catalog.read(cx).check_open(cx)?;
        self.0.update(cx, |installed, _| {
            installed.setups.admit(preparation, exec, request)
        })
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

    /// Admit one exact, explicitly chosen action before removing the original definition.
    pub fn remove_workspace(
        &self,
        expected: ProjectDefinition,
        binding: ProjectBinding,
        action: Option<crate::RemovalContainerAction>,
        cx: &mut App,
    ) -> Result<crate::RemovalOperation, crate::WorkspaceRemovalError> {
        self.check_removal(&expected, binding, cx)?;
        if let Some(action) = &action {
            let location =
                nocterm_project_providers::ContainerLocation::from_target(expected.target())
                    .map_err(nocterm_project_providers::ContainerActionError::from)?;
            if &location != action.source().location() {
                return Err(nocterm_project_providers::ContainerActionError::StaleSource.into());
            }
        }
        let operation = self.0.update(cx, |installed, _| {
            installed.removals.admit(
                expected,
                binding,
                action.as_ref().map(|action| action.source()),
            )
        })?;
        let native = action.map(|action| operation.execute(action, cx.background_executor()));
        self.publish_removal(operation.clone(), native, cx);
        Ok(operation)
    }

    /// Retry only the catalog step. An acknowledged container action is never repeated.
    pub fn retry_removal(
        &self,
        operation: &crate::RemovalOperation,
        cx: &mut App,
    ) -> Result<(), crate::WorkspaceRemovalError> {
        self.check_removal(operation.expected(), operation.binding(), cx)?;
        self.0.update(cx, |installed, _| {
            installed.removals.retain_retry(operation)
        })?;
        self.publish_removal(operation.clone(), None, cx);
        Ok(())
    }

    fn check_removal(
        &self,
        expected: &ProjectDefinition,
        binding: ProjectBinding,
        cx: &App,
    ) -> Result<(), crate::WorkspaceRemovalError> {
        self.0.read(cx).removals.check_open()?;
        self.check_binding(binding, cx)?;
        if binding.project() != expected.id()
            || self.catalog(cx).get(expected.id()) != Some(expected)
        {
            return Err(ProjectsError::StaleBinding(expected.id()).into());
        }
        Ok(())
    }

    fn publish_removal(
        &self,
        operation: crate::RemovalOperation,
        native: Option<Task<Result<(), crate::WorkspaceRemovalError>>>,
        cx: &mut App,
    ) {
        let handle = self.clone();
        cx.spawn(async move |cx| {
            let result = async {
                if let Some(native) = native {
                    native.await?;
                }
                let publication = cx.update(|cx| {
                    handle.check_removal(operation.expected(), operation.binding(), cx)?;
                    Ok::<_, crate::WorkspaceRemovalError>(handle.0.read(cx).catalog.clone().update(
                        cx,
                        |catalog, cx| {
                            catalog.remove_bound(
                                operation.expected().clone(),
                                operation.binding(),
                                cx,
                            )
                        },
                    ))
                })?;
                publication.await?;
                Ok(())
            }
            .await;
            operation.finish(result);
        })
        .detach();
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
