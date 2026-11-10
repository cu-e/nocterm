//! The keys of the window's groups and what the layout says about each group.

use nocterm_plugin_model::{GroupId, GroupSpec, Side, WidthClass};
use nocterm_window_layout::ResolvedGroup;

/// A permanent index into the window's groups. The catalog's groups come first, in `GROUPS`
/// order; a group only the layout knows (one the user made, or a panel on its own) gets the
/// next key the first time the window adopts a layout that places it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::workspace) struct GroupKey(u32);

impl GroupKey {
    /// The sidebar: the primary group, shown even while it has no panel.
    pub(in crate::workspace) const NAVIGATION: Self = Self(0);
    /// The assistant's side panel, hidden until its panel is available and opened.
    pub(in crate::workspace) const ASSISTANT: Self = Self(1);

    pub(super) fn at(index: usize) -> Self {
        Self(index as u32)
    }
    pub(super) fn index(self) -> usize {
        self.0 as usize
    }
}

/// Where a group goes: its id, whether it is the sidebar, the side of the content it opens on,
/// the class of its width and its position among the groups of the bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct GroupPlace {
    id: GroupId,
    primary: bool,
    side: Side,
    width: WidthClass,
    /// `None` while the bar does not place the group.
    order: Option<usize>,
}

impl GroupPlace {
    /// A group of the catalog where no layout places it: the sidebar on the left, any other
    /// group on the right.
    pub(super) fn of_catalog(spec: &GroupSpec) -> Self {
        let (side, width) = if spec.primary {
            (Side::Left, WidthClass::Navigation)
        } else {
            (Side::Right, WidthClass::Assistant)
        };
        Self {
            id: spec.id.clone(),
            primary: spec.primary,
            side,
            width,
            order: None,
        }
    }
    /// The group the layout places as its `order`th group.
    pub(super) fn placed(group: &ResolvedGroup, order: usize) -> Self {
        Self {
            id: group.id().clone(),
            primary: group.is_primary(),
            side: group.side(),
            width: group.width(),
            order: Some(order),
        }
    }
    /// The same group once the bar no longer places it.
    pub(super) fn unplaced(&self) -> Self {
        Self {
            order: None,
            ..self.clone()
        }
    }
    pub(super) fn id(&self) -> &GroupId {
        &self.id
    }
    pub(super) fn primary(&self) -> bool {
        self.primary
    }
    pub(super) fn side(&self) -> Side {
        self.side
    }
    pub(super) fn width(&self) -> WidthClass {
        self.width
    }
    pub(super) fn order(&self) -> Option<usize> {
        self.order
    }
}
