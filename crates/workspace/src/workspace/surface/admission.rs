//! Checked registration owns source resolution before any native dock mutation.
use super::*;
use crate::LocalTerminalTarget;
use gpui_kit::component::dock::NodeId;

pub(in crate::workspace) struct SurfaceItemAdmission {
    target: ContextSelectionReceipt,
    project: bool,
    location: Location,
    presentation: Option<project_surface::ProjectPresentation>,
}
enum Location {
    Central,
    Local(LocalTerminalTarget),
}

impl SurfaceItemAdmission {
    pub(in crate::workspace) fn general(target: ContextSelectionReceipt) -> Self {
        Self {
            target,
            project: false,
            location: Location::Central,
            presentation: None,
        }
    }
    pub(in crate::workspace) fn general_local(
        target: ContextSelectionReceipt,
        destination: LocalTerminalTarget,
    ) -> Self {
        Self {
            target,
            project: false,
            location: Location::Local(destination),
            presentation: None,
        }
    }
    pub(in crate::workspace) fn central(target: ContextSelectionReceipt) -> Self {
        Self {
            target,
            project: true,
            location: Location::Central,
            presentation: None,
        }
    }
    pub(in crate::workspace) fn local(
        target: ContextSelectionReceipt,
        destination: LocalTerminalTarget,
    ) -> Self {
        Self {
            target,
            project: true,
            location: Location::Local(destination),
            presentation: None,
        }
    }
    pub(in crate::workspace) fn workspace(
        target: ContextSelectionReceipt,
        presentation: project_surface::ProjectPresentation,
    ) -> Self {
        Self {
            presentation: Some(presentation),
            ..Self::general(target)
        }
    }
    pub(super) fn validate_outer(
        &self,
        owner: ContextWindowReceipt,
        item: EntityId,
        window: &Window,
        cx: &App,
    ) -> Result<(), ContextSelectionError> {
        if let Some(project) = &self.presentation {
            project.surface().validate_outer(owner, item, window, cx)?;
        }
        Ok(())
    }
    pub(super) fn duplicate_project(&self, tabs: &tabs::Tabs) -> bool {
        self.presentation
            .as_ref()
            .is_some_and(|project| tabs.project_item(project.surface().project()).is_some())
    }
    pub(super) fn outer(&self) -> bool {
        self.presentation.is_some()
    }
    pub(super) fn take_presentation(&mut self) -> Option<project_surface::ProjectPresentation> {
        self.presentation.take()
    }
    pub(super) fn resolve(
        &self,
        selection: &SurfaceSelection,
        owner: &window::SurfaceWindow,
        tabs: &tabs::Tabs,
        window: &Window,
        cx: &App,
    ) -> Result<(DockPlacement, Option<NodeId>), ContextSelectionError> {
        let validation = if self.project {
            selection.validate_source(self.target.surface(), owner, window)
        } else {
            selection.validate_general(self.target.surface(), owner, window)
        };
        validation.and_then(|_| self.destination(tabs, cx))
    }
    pub(in crate::workspace) fn target(&self) -> ContextSelectionReceipt {
        self.target
    }
    pub(in crate::workspace) fn destination(
        &self,
        tabs: &tabs::Tabs,
        cx: &App,
    ) -> Result<(DockPlacement, Option<NodeId>), ContextSelectionError> {
        match self.location {
            Location::Central => Ok((
                DockPlacement::Center,
                tabs.active_location(cx)
                    .filter(|(placement, _)| *placement == DockPlacement::Center)
                    .map(|(_, node)| node),
            )),
            Location::Local(LocalTerminalTarget::Bottom) => Ok(tabs
                .local_destination(cx)
                .map_or((DockPlacement::Bottom, None), |(placement, node)| {
                    (placement, Some(node))
                })),
            Location::Local(LocalTerminalTarget::Beside(anchor)) => tabs
                .item_location(anchor, cx)
                .map(|(placement, node)| (placement, Some(node)))
                .ok_or(ContextSelectionError::StaleSurface),
        }
    }
}
