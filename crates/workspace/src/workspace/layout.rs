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
use nocterm_ui::{ActiveDesign as _, LayoutHandle};

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

/// A rendering projection of the independent panel's authoritative state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum PanelPresentation {
    Hidden,
    Docked,
    Expanded,
}

impl PanelPresentation {
    pub(super) fn new(open: bool, maximized: bool) -> Self {
        match (open, maximized) {
            (false, _) => Self::Hidden,
            (true, false) => Self::Docked,
            (true, true) => Self::Expanded,
        }
    }
}

/// Which columns show, and on which side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Arrangement {
    pub sidebar: bool,
    pub side_panel: PanelPresentation,
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
            Column::SidePanel => self.side_panel != PanelPresentation::Hidden,
            Column::Content => self.side_panel != PanelPresentation::Expanded,
        };
        [left, Column::Content, right]
            .into_iter()
            .filter(|column| shown(*column))
            .collect()
    }
}

/// Resize states, one per arrangement, created as arrangements come up.
pub(super) struct Body {
    states: HashMap<Arrangement, (Entity<ResizableState>, Subscription)>,
    layout: LayoutHandle,
}

impl Body {
    pub(super) fn arrangement(
        &self,
        sidebar: bool,
        side_panel: PanelPresentation,
        cx: &gpui_kit::App,
    ) -> Arrangement {
        Arrangement {
            sidebar,
            side_panel,
            swapped: self.swapped(cx),
        }
    }

    pub(super) fn new(layout: LayoutHandle) -> Self {
        Self {
            states: HashMap::new(),
            layout,
        }
    }

    pub(super) fn swapped(&self, cx: &gpui_kit::App) -> bool {
        self.layout.flag(SWAPPED, cx)
    }

    pub(super) fn swap(&self, cx: &mut gpui_kit::App) -> bool {
        let swapped = !self.swapped(cx);
        report_layout(self.layout.set_flag(SWAPPED, swapped, cx));
        swapped
    }

    pub(super) fn widths(&self, window: &Window, cx: &gpui_kit::App) -> BodyWidths {
        BodyWidths::new(&self.layout, window, cx)
    }

    pub(super) fn resized(
        &self,
        arrangement: Arrangement,
        sizes: Vec<Pixels>,
        cx: &mut gpui_kit::App,
    ) {
        // An expanded arrangement can deliver its last drag after restoration.
        if arrangement.side_panel == PanelPresentation::Expanded {
            return;
        }
        for (column, size) in arrangement.columns().into_iter().zip(sizes) {
            let result = match column {
                Column::Sidebar => self.layout.set(SIDEBAR, size, cx),
                Column::SidePanel => self.layout.set(RIGHT_PANEL, size, cx),
                Column::Content => continue,
            };
            report_layout(result);
        }
    }

    pub(super) fn widen_side_panel(
        &mut self,
        arrangement: Arrangement,
        delta: Pixels,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        let Some(index) = arrangement
            .columns()
            .iter()
            .position(|column| *column == Column::SidePanel)
        else {
            return;
        };
        self.state(arrangement, cx).update(cx, |state, cx| {
            if let Some(size) = state.sizes().get(index).copied() {
                state.resize_panel(index, size + delta, window, cx);
            }
        });
    }

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
                        let sizes = state.read(cx).sizes().clone();
                        this.body_resized(arrangement, sizes, cx);
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
    pub(super) fn new(memory: &LayoutHandle, window: &Window, cx: &gpui_kit::App) -> Self {
        let layout = &cx.design().layout;
        let rem = window.rem_size();
        let sidebar_range = rems(layout.sidebar.min_width).to_pixels(rem)
            ..rems(layout.sidebar.max_width).to_pixels(rem);
        let right_panel_range = rems(layout.agent_panel.min_width).to_pixels(rem)
            ..rems(layout.agent_panel.max_width).to_pixels(rem);
        let sidebar = memory
            .get(SIDEBAR, cx)
            .unwrap_or_else(|| rems(layout.sidebar.width).to_pixels(rem))
            .clamp(sidebar_range.start, sidebar_range.end);
        let right_panel = memory
            .get(RIGHT_PANEL, cx)
            .unwrap_or_else(|| rems(layout.agent_panel.width).to_pixels(rem))
            .clamp(right_panel_range.start, right_panel_range.end);
        Self {
            sidebar,
            sidebar_range,
            right_panel,
            right_panel_range,
        }
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;

fn report_layout(result: Result<(), nocterm_store::StoreError>) {
    if let Err(error) = result {
        tracing::warn!(%error, "workspace layout change was not admitted");
    }
}
