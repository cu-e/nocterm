//! The arrangement of the window: a built-in layout resolved against the plugin catalog.
//!
//! Until layouts are stored, the choice between the built-ins is the shared legacy flag
//! `workspace.swapped`: the default arrangement, or the same one with every group on the other
//! side of the content. Both are resolved once, when the window is composed.

use gpui_kit::App;
use nocterm_plugin_model::{PluginCatalog, Side};
use nocterm_ui::LayoutHandle;
use nocterm_window_layout::{BuiltIn, ResolvedGroup, ResolvedLayout};

use super::{layout::report_layout, panel_groups::GroupKey};

/// Whether the sidebar and the side panel trade places.
const SWAPPED: &str = "workspace.swapped";

pub(super) struct ShellLayout {
    flags: LayoutHandle,
    default: ResolvedLayout,
    swapped: ResolvedLayout,
}

impl ShellLayout {
    pub(super) fn new(flags: LayoutHandle, catalog: &PluginCatalog) -> Self {
        Self {
            flags,
            default: BuiltIn::Default.resolve(catalog),
            swapped: BuiltIn::Swapped.resolve(catalog),
        }
    }

    /// Whether the groups are on the other side of the content.
    pub(super) fn swapped(&self, cx: &App) -> bool {
        self.flags.flag(SWAPPED, cx)
    }

    /// Selects the other built-in. Returns whether the groups are now swapped.
    pub(super) fn swap(&self, cx: &mut App) -> bool {
        let swapped = !self.swapped(cx);
        report_layout(self.flags.set_flag(SWAPPED, swapped, cx));
        swapped
    }

    /// The arrangement in force.
    pub(super) fn current(&self, cx: &App) -> &ResolvedLayout {
        self.layout(self.swapped(cx))
    }

    fn layout(&self, swapped: bool) -> &ResolvedLayout {
        if swapped {
            &self.swapped
        } else {
            &self.default
        }
    }

    /// The side of the content a group opens on. A group the arrangement does not place opens
    /// where its panels hint, as the arrangement would have put it.
    pub(super) fn side(&self, group: GroupKey, swapped: bool) -> Side {
        self.layout(swapped)
            .groups()
            .find(|placed| placed.id().as_str() == group.id())
            .map(ResolvedGroup::side)
            .unwrap_or_else(|| group.side().swapped_if(swapped))
    }

    /// The shown groups with their sides, in bar order: groups the arrangement places come
    /// first, in its order.
    pub(super) fn columns(
        &self,
        shown: impl IntoIterator<Item = GroupKey>,
        cx: &App,
    ) -> Vec<(GroupKey, Side)> {
        let swapped = self.swapped(cx);
        let layout = self.layout(swapped);
        let mut shown: Vec<_> = shown.into_iter().collect();
        shown.sort_by_key(|group| {
            layout
                .groups()
                .position(|placed| placed.id().as_str() == group.id())
                .unwrap_or(usize::MAX)
        });
        shown
            .into_iter()
            .map(|group| (group, self.side(group, swapped)))
            .collect()
    }
}
