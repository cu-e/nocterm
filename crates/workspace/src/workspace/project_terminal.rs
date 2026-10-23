//! Original shell placement, private behind the lower-layer captured destination.
use super::*;
use nocterm_workspace_ports::{ProjectTerminalDestination, ProjectTerminalTarget};

pub(super) struct ProjectTerminalPlacement {
    surface: ProjectSurface,
    selection: ContextSelectionReceipt,
    destination: Option<crate::LocalTerminalTarget>,
    initial: bool,
}
impl ProjectTerminalPlacement {
    pub(super) fn capture(
        surface: ProjectSurface,
        selection: ContextSelectionReceipt,
        destination: Option<crate::LocalTerminalTarget>,
        initial: bool,
        cx: &mut App,
    ) -> ProjectTerminalTarget {
        ProjectTerminalTarget::new(cx.new(|_| Self {
            surface,
            selection,
            destination,
            initial,
        }))
    }
}
impl ProjectTerminalDestination for ProjectTerminalPlacement {
    fn validate(&self, window: &Window, cx: &App) -> Result<(), crate::ContextSelectionError> {
        self.surface.owner().check_window(window)?;
        if self.initial {
            self.surface.owner().resolve(self.selection.surface(), cx)?;
            if !self
                .surface
                .owner()
                .registered(self.selection.surface(), cx)
            {
                return Err(crate::ContextSelectionError::StaleSurface);
            }
        } else if !self.surface.accepts_selection(self.selection, cx) {
            return Err(crate::ContextSelectionError::StaleSelection);
        }
        if let Some(crate::LocalTerminalTarget::Beside(anchor)) = self.destination
            && self
                .surface
                .engine()
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
fn placement(
    target: &ProjectTerminalTarget,
) -> Result<Entity<ProjectTerminalPlacement>, crate::ContextSelectionError> {
    target
        .destination_entity()
        .downcast::<ProjectTerminalPlacement>()
        .map_err(|_| crate::ContextSelectionError::StaleSurface)
}

/// Checks this shell's captured destination before a provider creates an Item.
pub fn validate_project_terminal_target(
    target: &ProjectTerminalTarget,
    window: &Window,
    cx: &App,
) -> Result<(), crate::ContextSelectionError> {
    placement(target)?.read(cx).validate(window, cx)
}

/// Admits through the original captured shell placement.
/// A foreign destination type is refused without closing the caller's Item: it
/// grants no membership information, so the caller retains cleanup responsibility.
/// A genuine stale destination closes only a new, unowned Item once.
pub fn admit_project_terminal<T: crate::LocalTerminal>(
    target: &ProjectTerminalTarget,
    item: Entity<T>,
    window: &mut Window,
    cx: &mut App,
) -> Result<bool, crate::ContextSelectionError> {
    let placement = placement(target)?;
    let placement = placement.read(cx);
    let (surface, selection, destination) = (
        placement.surface.clone(),
        placement.selection,
        placement.destination,
    );
    if let Err(error) = validate_project_terminal_target(target, window, cx) {
        let source = surface.source(cx);
        if !surface.engine().read(cx).contains_item(item.entity_id())
            && !surface.owner().foreign_item(source, item.entity_id(), cx)
        {
            ItemHandle::close(&item, window, cx);
        }
        return Err(error);
    }
    match destination {
        Some(destination) => surface.add_local_terminal(selection, item, destination, window, cx),
        None => surface.add_item(selection, item, window, cx),
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn project_terminal_selection(
    target: &ProjectTerminalTarget,
    cx: &App,
) -> Result<ContextSelectionReceipt, crate::ContextSelectionError> {
    Ok(placement(target)?.read(cx).selection)
}
