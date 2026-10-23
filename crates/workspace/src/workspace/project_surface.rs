//! Retained original inner dock with source-bound UI operations.
use super::*;
use nocterm_project_model::{ProjectId, RootPath};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectSurfaceEvent {
    Changed(ProjectSurfaceReceipt),
    OpenTerminal {
        selection: ContextSelectionReceipt,
        destination: Option<crate::LocalTerminalTarget>,
    },
}

/// Original destination captured before an ordinary terminal is opened.
pub struct ProjectTerminalTarget {
    surface: ProjectSurface,
    selection: ContextSelectionReceipt,
    destination: Option<crate::LocalTerminalTarget>,
    initial: bool,
}
impl ProjectTerminalTarget {
    pub fn surface(&self) -> &ProjectSurface {
        &self.surface
    }
    pub fn selection(&self) -> ContextSelectionReceipt {
        self.selection
    }
    pub fn destination(&self) -> Option<crate::LocalTerminalTarget> {
        self.destination
    }
    pub fn validate(&self, window: &Window, cx: &App) -> Result<(), crate::ContextSelectionError> {
        self.surface.owner.check_window(window)?;
        if self.initial {
            self.surface.owner.resolve(self.selection.surface(), cx)?;
            if !self.surface.owner.registered(self.selection.surface(), cx) {
                return Err(crate::ContextSelectionError::StaleSurface);
            }
        } else if !self.surface.accepts_selection(self.selection, cx) {
            return Err(crate::ContextSelectionError::StaleSelection);
        }
        if let Some(crate::LocalTerminalTarget::Beside(anchor)) = self.destination
            && self
                .surface
                .entity
                .read(cx)
                .content()
                .tab_location(anchor, cx)
                .is_none()
        {
            return Err(crate::ContextSelectionError::StaleSurface);
        }
        Ok(())
    }
}

pub(super) struct ProjectPresentation {
    surface: ProjectSurface,
    _events: Subscription,
}
impl ProjectPresentation {
    pub(super) fn surface(&self) -> ProjectSurface {
        self.surface.clone()
    }
}

#[derive(Clone)]
pub struct ProjectSurfaceFactory {
    owner: Rc<surface::window::SurfaceWindow>,
}
impl ProjectSurfaceFactory {
    pub(super) fn new(owner: Rc<surface::window::SurfaceWindow>) -> Self {
        Self { owner }
    }
    pub fn create(
        &self,
        project: ProjectId,
        outer: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<ProjectSurface, crate::ContextSelectionError> {
        let entity = self.owner.create_project(project, outer, window, cx)?;
        Ok(ProjectSurface {
            entity,
            owner: self.owner.clone(),
            project,
            monitor_target: Rc::new(std::cell::RefCell::new(None)),
        })
    }
}

/// Retains one original inner dock. No mutable entity or host is exposed.
#[derive(Clone)]
pub struct ProjectSurface {
    entity: Entity<surface::TabSurface>,
    owner: Rc<surface::window::SurfaceWindow>,
    project: ProjectId,
    monitor_target: Rc<std::cell::RefCell<Option<nocterm_project_model::ProjectTarget>>>,
}
impl ProjectSurface {
    pub(super) fn engine(&self) -> Entity<surface::TabSurface> {
        self.entity.clone()
    }
    pub(super) fn bind(
        &self,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> ProjectPresentation {
        let events = cx.subscribe_in(
            &self.entity,
            window,
            |workspace, _, event: &surface::SurfaceEvent, window, cx| {
                workspace.surface_event(event, window, cx)
            },
        );
        ProjectPresentation {
            surface: self.clone(),
            _events: events,
        }
    }
    pub(super) fn monitor_target(&self) -> Option<nocterm_project_model::ProjectTarget> {
        self.monitor_target.borrow().clone()
    }
    /// Sets inert monitoring identity on this original resource surface.
    pub fn set_monitor_target(&self, target: nocterm_project_model::ProjectTarget, cx: &mut App) {
        if self.monitor_target.borrow().as_ref() == Some(&target) {
            return;
        }
        *self.monitor_target.borrow_mut() = Some(target);
        self.entity.update(cx, |surface, cx| {
            cx.emit(surface::SurfaceEvent::Observed(
                surface.context_source(),
                WorkspaceEvent::ActiveSessionChanged,
            ))
        });
    }
    pub fn set_root(&self, root: RootPath, cx: &mut App) {
        self.entity
            .update(cx, |surface, cx| surface.set_root(root, cx));
    }
    pub fn project(&self) -> ProjectId {
        self.project
    }
    pub fn source(&self, cx: &App) -> ProjectSurfaceReceipt {
        self.entity.read(cx).context_source()
    }
    pub fn selection_receipt(&self, cx: &App) -> ContextSelectionReceipt {
        self.entity.read(cx).context_selection_receipt()
    }
    pub fn accepts_selection(&self, receipt: ContextSelectionReceipt, cx: &App) -> bool {
        self.entity.read(cx).accepts_context_selection(receipt, cx)
    }
    pub fn view(&self) -> AnyView {
        self.entity.clone().into()
    }
    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.entity.read(cx).preferred_focus(cx)
    }
    pub fn tabs(&self, cx: &App) -> Vec<TabSnapshot> {
        self.entity.read(cx).tabs().snapshots(cx)
    }
    pub fn find_item<T: Item>(&self, cx: &App) -> Option<Entity<T>> {
        self.entity.read(cx).tabs().find()
    }
    pub fn panes(&self, placement: DockPlacement, cx: &App) -> Vec<TabPane> {
        self.entity.read(cx).tabs().panes(placement, cx)
    }
    pub fn zoomed_pane(&self, cx: &App) -> Option<gpui_kit::component::dock::NodeId> {
        self.entity.read(cx).tabs().zoomed(cx)
    }
    pub fn local_terminal_is_visible(&self, cx: &App) -> bool {
        self.entity.read(cx).tabs().local_visible(cx)
    }
    pub fn active_session(&self, cx: &App) -> Option<SessionContext> {
        self.entity.read(cx).tabs().active_session()
    }
    pub fn terminal_target(
        &self,
        selection: ContextSelectionReceipt,
        destination: Option<crate::LocalTerminalTarget>,
        window: &Window,
        cx: &App,
    ) -> Result<ProjectTerminalTarget, crate::ContextSelectionError> {
        let target = ProjectTerminalTarget {
            surface: self.clone(),
            selection,
            destination,
            initial: false,
        };
        target.validate(window, cx)?;
        Ok(target)
    }
    /// One original opening intent may finish while this outer Item is hidden.
    pub fn initial_terminal_target(
        &self,
        window: &Window,
        cx: &mut App,
    ) -> Result<ProjectTerminalTarget, crate::ContextSelectionError> {
        let selection = self.entity.update(cx, |surface, cx| {
            surface.initial_terminal_target(window, cx)
        })?;
        Ok(ProjectTerminalTarget {
            surface: self.clone(),
            selection,
            destination: None,
            initial: true,
        })
    }
    pub fn observe(
        &self,
        cx: &mut App,
        mut observe: impl FnMut(ProjectSurfaceEvent, &mut App) + 'static,
    ) -> Subscription {
        cx.subscribe(
            &self.entity,
            move |_, event: &surface::SurfaceEvent, cx| match event {
                surface::SurfaceEvent::Observed(source, _) => {
                    observe(ProjectSurfaceEvent::Changed(*source), cx)
                }
                surface::SurfaceEvent::OpenTerminal(selection, destination) => observe(
                    ProjectSurfaceEvent::OpenTerminal {
                        selection: *selection,
                        destination: *destination,
                    },
                    cx,
                ),
                surface::SurfaceEvent::OpenLocal(_, _) => {}
            },
        )
    }
    pub fn add_item<T: Item>(
        &self,
        target: ContextSelectionReceipt,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<bool, crate::ContextSelectionError> {
        self.entity.update(cx, |surface, cx| {
            surface.add_project_item(target, item, window, cx)
        })
    }
    pub fn add_local_terminal<T: crate::LocalTerminal>(
        &self,
        target: ContextSelectionReceipt,
        item: Entity<T>,
        destination: crate::LocalTerminalTarget,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<bool, crate::ContextSelectionError> {
        self.entity.update(cx, |surface, cx| {
            surface.add_project_local(target, item, destination, window, cx)
        })
    }
    pub fn add_background_item<T: Item>(
        &self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<bool, crate::ContextSelectionError> {
        self.entity.update(cx, |surface, cx| {
            surface.add_project_background(item, window, cx)
        })
    }
    pub fn activate_item(
        &self,
        target: ContextSelectionReceipt,
        item: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<bool, crate::ContextSelectionError> {
        self.owner.check_window(window)?;
        if !self.accepts_selection(target, cx) {
            return Err(crate::ContextSelectionError::StaleSelection);
        }
        Ok(self.entity.update(cx, |surface, cx| {
            surface.activate_item_by_id(item, window, cx)
        }))
    }
    pub fn close_tabs(
        &self,
        target: ContextSelectionReceipt,
        item: EntityId,
        scope: TabCloseScope,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), crate::ContextSelectionError> {
        self.owner.check_window(window)?;
        if !self.accepts_selection(target, cx) {
            return Err(crate::ContextSelectionError::StaleSelection);
        }
        self.entity.update(cx, |surface, cx| {
            surface.close_tabs(item, scope, window, cx)
        });
        Ok(())
    }
    pub fn split_item(
        &self,
        target: ContextSelectionReceipt,
        item: EntityId,
        placement: gpui_kit::component::Placement,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), crate::ContextSelectionError> {
        self.owner.check_window(window)?;
        if !self.accepts_selection(target, cx) {
            return Err(crate::ContextSelectionError::StaleSelection);
        }
        self.entity.update(cx, |surface, cx| {
            surface.split_item(item, placement, window, cx)
        });
        Ok(())
    }
    pub fn close(&self, window: &mut Window, cx: &mut App) {
        self.entity
            .update(cx, |surface, cx| surface.close(window, cx));
    }
    pub(crate) fn set_visible(&self, visible: bool, cx: &mut App) {
        self.entity
            .update(cx, |surface, cx| surface.set_presented(visible, cx));
    }
    pub(super) fn validate_outer(
        &self,
        owner: ContextWindowReceipt,
        item: EntityId,
        window: &Window,
        cx: &App,
    ) -> Result<(), crate::ContextSelectionError> {
        self.owner.check_window(window)?;
        let source = self.source(cx);
        if source.window() != owner {
            return Err(crate::ContextSelectionError::StaleWindow);
        }
        if source.outer_item() != Some(item) {
            return Err(crate::ContextSelectionError::StaleSurface);
        }
        self.owner.resolve(source, cx)?;
        Ok(())
    }
}
