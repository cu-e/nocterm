//! What a panel-group transition asks the window to do once the owner has committed.

use gpui_kit::FocusHandle;

use super::GroupKey;

/// Where focus goes after a group changed.
pub(in crate::workspace) enum PanelFocus {
    /// Focus stays where it is.
    Preserve,
    /// A panel takes focus.
    Panel(FocusHandle),
    /// The active tab, or the workspace itself while there is none.
    Central,
    /// The active tab; with no tab, focus stays where it is.
    ActiveItem,
}

/// The effect of one group transition: which group became visible or hidden, and
/// where focus goes. The workspace applies it after the owner has committed.
pub(in crate::workspace) struct PanelTransition {
    visibility: Option<GroupKey>,
    focus: PanelFocus,
}

impl PanelTransition {
    pub(super) fn new(visibility: Option<GroupKey>, focus: PanelFocus) -> Self {
        Self { visibility, focus }
    }
    /// The group that was shown or hidden, if one was.
    pub(in crate::workspace) fn visibility_changed(&self) -> Option<GroupKey> {
        self.visibility
    }
    pub(in crate::workspace) fn into_focus(self) -> PanelFocus {
        self.focus
    }
}
