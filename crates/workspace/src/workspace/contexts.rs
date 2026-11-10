//! Immutable UI identities distinguish live membership from deferred selection.
use super::*;
use gpui_kit::WindowId;
use nocterm_project_model::ContextId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextWindowReceipt {
    workspace: EntityId,
    window: WindowId,
}
impl ContextWindowReceipt {
    pub(super) fn new(workspace: EntityId, window: &Window) -> Self {
        Self {
            workspace,
            window: window.window_handle().window_id(),
        }
    }
    /// The original root Workspace entity, distinct from every inner surface.
    pub fn workspace_id(self) -> EntityId {
        self.workspace
    }
    pub(super) fn matches(self, window: &Window) -> bool {
        self.window == window.window_handle().window_id()
    }
}

/// A live original surface. Hiding does not expire it; recreating its entity does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectSurfaceReceipt {
    owner: ContextWindowReceipt,
    context: ContextId,
    surface: EntityId,
    outer: Option<EntityId>,
}
impl ProjectSurfaceReceipt {
    pub(super) fn new(
        owner: ContextWindowReceipt,
        context: ContextId,
        surface: EntityId,
        outer: Option<EntityId>,
    ) -> Self {
        Self {
            owner,
            context,
            surface,
            outer,
        }
    }
    pub fn context(self) -> ContextId {
        self.context
    }
    pub fn window(self) -> ContextWindowReceipt {
        self.owner
    }
    pub fn outer_item(self) -> Option<EntityId> {
        self.outer
    }
    pub(super) fn surface_id(self) -> EntityId {
        self.surface
    }
    pub(super) fn dock_id(self) -> SharedString {
        format!(
            "workspace-{}-{}",
            self.owner.workspace.as_u64(),
            self.surface.as_u64()
        )
        .into()
    }
}

/// Deferred UI permission expires after a different selection, including ABA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextSelectionReceipt {
    source: ProjectSurfaceReceipt,
    generation: u64,
    outer_generation: u64,
}
impl ContextSelectionReceipt {
    pub(super) fn new(
        source: ProjectSurfaceReceipt,
        generation: u64,
        outer_generation: u64,
    ) -> Self {
        Self {
            source,
            generation,
            outer_generation,
        }
    }
    pub fn surface(self) -> ProjectSurfaceReceipt {
        self.source
    }
}
