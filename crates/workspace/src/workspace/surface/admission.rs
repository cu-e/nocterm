//! Checked registration owns source resolution before any native dock mutation.
use super::*;
use crate::LocalTerminalTarget;
use gpui_kit::component::dock::NodeId;

pub(super) struct SurfaceItemAdmission {
    target: ContextSelectionReceipt,
    project: bool,
    location: Location,
}
enum Location {
    Central,
    Local(LocalTerminalTarget),
}

impl SurfaceItemAdmission {
    pub(super) fn general(target: ContextSelectionReceipt) -> Self {
        Self {
            target,
            project: false,
            location: Location::Central,
        }
    }
    pub(super) fn general_local(
        target: ContextSelectionReceipt,
        destination: LocalTerminalTarget,
    ) -> Self {
        Self {
            target,
            project: false,
            location: Location::Local(destination),
        }
    }
    pub(super) fn central(target: ContextSelectionReceipt) -> Self {
        Self {
            target,
            project: true,
            location: Location::Central,
        }
    }
    pub(super) fn local(target: ContextSelectionReceipt, destination: LocalTerminalTarget) -> Self {
        Self {
            target,
            project: true,
            location: Location::Local(destination),
        }
    }
    pub(super) fn target(&self) -> ContextSelectionReceipt {
        self.target
    }
    pub(super) fn project(&self) -> bool {
        self.project
    }
    pub(super) fn destination(
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
