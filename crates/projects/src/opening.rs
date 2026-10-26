//! One original claim creates one real outer Item; repeated opens focus it.
use crate::{
    Installed, ProjectOpenError, ProjectView, context,
    view::{ProjectLifetime, ProjectPresentation},
};
use gpui_kit::{App, AppContext as _, Context, Window, WindowId};
use nocterm_project_model::{ProjectDefinition, ProjectId};
use nocterm_project_service::{ClaimOutcome, ProjectBinding, ProjectClaim};
use nocterm_workspace::Workspace;

pub(crate) fn open(
    installed: &Installed,
    project: ProjectId,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<(), ProjectOpenError> {
    let existing = workspace.items(cx).find_map(|item| {
        item.view()
            .downcast::<ProjectView>()
            .ok()
            .filter(|view| view.read(cx).project() == project)
    });
    if let Some(existing) = existing {
        workspace.activate_item_by_id(existing.entity_id(), window, cx);
        return Ok(());
    }
    if let Some(intent) =
        ProjectOpenIntent::capture(installed, project, window.window_handle().window_id(), cx)?
    {
        intent.publish(workspace, window, cx)?;
    }
    Ok(())
}

struct ProjectOpenIntent {
    installed: Installed,
    definition: ProjectDefinition,
    binding: ProjectBinding,
    claim: ProjectClaim,
}
impl ProjectOpenIntent {
    fn capture(
        installed: &Installed,
        project: ProjectId,
        window: WindowId,
        cx: &mut App,
    ) -> Result<Option<Self>, ProjectOpenError> {
        let definition = installed
            .catalog(cx)
            .get(project)
            .cloned()
            .ok_or(ProjectOpenError::Removed)?;
        let binding = installed.model().binding(project, cx)?;
        let claim = match installed.model().claim(project, window, cx)? {
            ClaimOutcome::Acquired(claim) => claim,
            ClaimOutcome::Retained(_) => {
                installed.model().request_focus(project, window, cx)?;
                return Ok(None);
            }
            ClaimOutcome::FocusOwner { project, window } => {
                installed.model().request_focus(project, window, cx)?;
                return Ok(None);
            }
        };
        Ok(Some(Self {
            installed: installed.clone(),
            definition,
            binding,
            claim,
        }))
    }
    fn publish(
        self,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Result<(), ProjectOpenError> {
        let reservation = cx.reserve_entity::<ProjectView>();
        let surface = match workspace.project_surface_factory().create(
            self.definition.id(),
            reservation.entity_id(),
            window,
            cx,
        ) {
            Ok(surface) => surface,
            Err(error) => {
                self.installed.model().release_claim(self.claim, cx)?;
                return Err(error.into());
            }
        };
        let (presentation, lifetime) = match self.presentation(workspace, window, cx) {
            Ok(presentation) => presentation,
            Err(error) => {
                self.installed.model().release_claim(self.claim, cx)?;
                return Err(error);
            }
        };
        let workspace_owner = cx.entity().downgrade();
        let item = cx.insert_entity(reservation, |cx| {
            ProjectView::new(
                self.installed.clone(),
                presentation,
                lifetime,
                surface.clone(),
                workspace_owner,
                window,
                cx,
            )
        });
        match workspace.add_project_workspace(item.clone(), surface, window, cx) {
            Ok(true) => {}
            outcome => {
                self.installed.model().release_claim(self.claim, cx)?;
                return Err(outcome
                    .err()
                    .unwrap_or(nocterm_workspace_ports::ContextSelectionError::StaleSurface)
                    .into());
            }
        }
        item.update(cx, |item, cx| item.refresh(window, cx));
        Ok(())
    }
    fn presentation(
        &self,
        workspace: &Workspace,
        window: &Window,
        cx: &mut App,
    ) -> Result<(ProjectPresentation, ProjectLifetime), ProjectOpenError> {
        let host = crate::location::host(self.definition.target());
        let source = host
            .as_ref()
            .ok_or(nocterm_workspace_ports::ProjectParentError::UnavailableParent)
            .and_then(|host| context::capture_host(workspace, &self.installed, host, window, cx));
        let parent_item = source
            .as_ref()
            .ok()
            .and_then(|source| source.expected())
            .map(|expected| expected.item());
        let parent = host.as_ref().and_then(|host| {
            context::server(
                workspace,
                host,
                source.as_ref().ok().map(|source| source.parent()),
                cx,
            )
            .ok()
        });
        let session = capture_session(
            &self.installed,
            self.binding,
            self.claim,
            self.definition.target(),
            workspace,
            window,
            cx,
        );
        Ok((
            ProjectPresentation::new(self.definition.clone(), parent, parent_item),
            ProjectLifetime::new(self.claim, session),
        ))
    }
}

pub(crate) fn capture_session(
    installed: &Installed,
    binding: ProjectBinding,
    claim: ProjectClaim,
    location: &nocterm_project_model::ProjectTarget,
    workspace: &Workspace,
    window: &Window,
    cx: &App,
) -> Result<std::rc::Rc<dyn nocterm_workspace_ports::ProjectTerminalSession>, ProjectOpenError> {
    installed.model().check_binding(binding, cx)?;
    installed.model().check_claim(claim, cx)?;
    if let Some(host) = crate::location::host(location)
        && let Ok(source) = context::capture_host(workspace, installed, &host, window, cx)
        && let Some(expected) = source.expected()
        && let Some(item) = workspace.general_item(expected.item(), cx)
        && let Some(capabilities) = item.terminal_capabilities(cx)
        && let Ok(session) =
            installed
                .terminals()
                .capture(item.view(), expected, capabilities, location, cx)
    {
        return Ok(session);
    }
    installed
        .resolver()
        .ok_or(nocterm_workspace_ports::ProjectConnectionError::Unavailable)?
        .resolve(location, cx)
        .map_err(Into::into)
}
