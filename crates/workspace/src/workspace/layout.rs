//! The body's resizable columns: the sidebar, the tabs and the side panel,
//! in the order and at the widths the user chose.
//!
//! Only visible columns take part in the resizable group. A hidden column
//! kept in the group would keep its last width in the group's arithmetic and
//! stop its neighbours from growing, so each arrangement of visible columns
//! has a resize state of its own, and widths are remembered per column
//! rather than per position.
use std::collections::HashMap;

use gpui_kit::{
    AppContext as _, Context, Entity, Pixels, Subscription, Window,
    component::{ResizablePanelEvent, ResizableState},
    rems,
};
use nocterm_ui::{ActiveDesign as _, LayoutMemory};

use super::Workspace;

const SIDEBAR: &str = "workspace.sidebar";
const RIGHT_PANEL: &str = "workspace.right_panel";
/// Whether the sidebar and the side panel trade places.
const SWAPPED: &str = "workspace.swapped";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Column {
    Sidebar,
    Content,
    SidePanel,
}

/// Which columns show, and on which side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Arrangement {
    pub sidebar: bool,
    pub side_panel: bool,
    pub swapped: bool,
}

impl Arrangement {
    pub(super) fn columns(self) -> Vec<Column> {
        let (left, right) = if self.swapped {
            (Column::SidePanel, Column::Sidebar)
        } else {
            (Column::Sidebar, Column::SidePanel)
        };
        let shown = |column| match column {
            Column::Sidebar => self.sidebar,
            Column::SidePanel => self.side_panel,
            Column::Content => true,
        };
        [left, Column::Content, right]
            .into_iter()
            .filter(|column| shown(*column))
            .collect()
    }
}

/// Resize states, one per arrangement, created as arrangements come up.
#[derive(Default)]
pub(super) struct Body {
    states: HashMap<Arrangement, (Entity<ResizableState>, Subscription)>,
}

impl Body {
    pub(super) fn state(
        &mut self,
        arrangement: Arrangement,
        cx: &mut Context<Workspace>,
    ) -> Entity<ResizableState> {
        self.states
            .entry(arrangement)
            .or_insert_with(|| {
                let state = cx.new(|_| ResizableState::default());
                let subscription =
                    cx.subscribe(&state, move |this, state, _: &ResizablePanelEvent, cx| {
                        if this.right_panel_maximized {
                            return;
                        }
                        let sizes = state.read(cx).sizes().clone();
                        for (column, size) in arrangement.columns().into_iter().zip(sizes) {
                            match column {
                                Column::Sidebar => LayoutMemory::set(SIDEBAR, size, cx),
                                Column::SidePanel => LayoutMemory::set(RIGHT_PANEL, size, cx),
                                Column::Content => {}
                            }
                        }
                    });
                (state, subscription)
            })
            .0
            .clone()
    }
}

/// The columns' widths and ranges: remembered, or the design's defaults.
pub(super) struct BodyWidths {
    pub sidebar: Pixels,
    pub sidebar_range: std::ops::Range<Pixels>,
    pub right_panel: Pixels,
    pub right_panel_range: std::ops::Range<Pixels>,
}

impl BodyWidths {
    pub(super) fn new(window: &Window, cx: &gpui_kit::App) -> Self {
        let layout = &cx.design().layout;
        let rem = window.rem_size();
        let sidebar_range = rems(layout.sidebar_min_width).to_pixels(rem)
            ..rems(layout.sidebar_max_width).to_pixels(rem);
        let right_panel_range = rems(layout.agent_panel_min_width).to_pixels(rem)
            ..rems(layout.agent_panel_max_width).to_pixels(rem);
        let sidebar = LayoutMemory::get(SIDEBAR, cx)
            .unwrap_or_else(|| rems(layout.sidebar_width).to_pixels(rem))
            .clamp(sidebar_range.start, sidebar_range.end);
        let right_panel = LayoutMemory::get(RIGHT_PANEL, cx)
            .unwrap_or_else(|| rems(layout.agent_panel_width).to_pixels(rem))
            .clamp(right_panel_range.start, right_panel_range.end);
        Self {
            sidebar,
            sidebar_range,
            right_panel,
            right_panel_range,
        }
    }
}

impl Workspace {
    pub(super) fn arrangement(&self, cx: &gpui_kit::App) -> Arrangement {
        Arrangement {
            sidebar: self.sidebar_open,
            side_panel: self.right_panel_open && self.right_panel_available,
            swapped: Self::sides_swapped(cx),
        }
    }

    /// Whether the sidebar is on the right and the side panel on the left.
    pub fn sides_swapped(cx: &gpui_kit::App) -> bool {
        LayoutMemory::flag(SWAPPED, cx)
    }

    /// Puts the sidebar and the side panel on each other's side.
    pub fn swap_sides(&mut self, cx: &mut Context<Self>) {
        let swapped = !Self::sides_swapped(cx);
        LayoutMemory::set_flag(SWAPPED, swapped, cx);
        if let Some(panel) = &self.right_panel {
            panel.set_docked_left(swapped, cx);
        }
        cx.notify();
    }

    /// Widens (or, with a negative delta, narrows) the side panel, for a
    /// panel that grows a column of its own.
    pub(super) fn widen_right_panel(
        &mut self,
        delta: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.right_panel_maximized || !self.right_panel_open {
            return;
        }
        let arrangement = self.arrangement(cx);
        let Some(ix) = arrangement
            .columns()
            .iter()
            .position(|column| *column == Column::SidePanel)
        else {
            return;
        };
        let body = self.body.state(arrangement, cx);
        body.update(cx, |body, cx| {
            if let Some(size) = body.sizes().get(ix).copied() {
                body.resize_panel(ix, size + delta, window, cx);
            }
        });
        cx.notify();
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
