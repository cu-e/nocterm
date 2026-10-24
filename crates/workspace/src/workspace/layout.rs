//! The body's resizable columns: one per shown panel group and the tabs,
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
};
use nocterm_plugin_model::{Side, WidthClass};
use nocterm_ui::{LayoutHandle, width_range};

use super::{
    Workspace,
    panel_groups::{GroupKey, PanelGroups},
    shell_layout::ShellLayout,
};

const SIDEBAR: &str = "workspace.sidebar";
const RIGHT_PANEL: &str = "workspace.right_panel";

/// One column of the body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Column {
    /// A shown panel group: its width is remembered per class, and its side tells which edge
    /// of the column the border is on.
    Group {
        group: GroupKey,
        class: WidthClass,
        side: Side,
    },
    Content,
}

/// Which columns show, in order from the left. An expanded group replaces the content.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Arrangement {
    columns: Vec<Column>,
    expanded: Option<GroupKey>,
}

impl Arrangement {
    /// `shown` lists the shown groups in bar order with their sides. Within a side the bar
    /// runs from the window edge inward, so the right side is laid out in reverse.
    pub(super) fn new(
        shown: impl IntoIterator<Item = (GroupKey, Side)>,
        expanded: Option<GroupKey>,
    ) -> Self {
        let (mut left, mut right) = (Vec::new(), Vec::new());
        for (group, side) in shown {
            let column = Column::Group {
                group,
                class: group.width_class(),
                side,
            };
            match side {
                Side::Left => left.push(column),
                Side::Right => right.push(column),
            }
        }
        right.reverse();
        let content = expanded.is_none().then_some(Column::Content);
        Self {
            columns: left.into_iter().chain(content).chain(right).collect(),
            expanded,
        }
    }
    pub(super) fn columns(&self) -> &[Column] {
        &self.columns
    }
    pub(super) fn expanded(&self) -> Option<GroupKey> {
        self.expanded
    }
    fn position_of(&self, group: GroupKey) -> Option<usize> {
        self.columns.iter().position(
            |column| matches!(column, Column::Group { group: shown, .. } if *shown == group),
        )
    }
}

/// Resize states, one per arrangement, created as arrangements come up.
pub(super) struct Body {
    states: HashMap<Arrangement, (Entity<ResizableState>, Subscription)>,
    layout: LayoutHandle,
}

impl Body {
    /// The columns of the groups that are shown now, on the sides the arrangement gives them.
    pub(super) fn arrangement(&self, groups: &PanelGroups, shell: &ShellLayout) -> Arrangement {
        Arrangement::new(shell.columns(groups.shown()), groups.expanded())
    }

    pub(super) fn new(layout: LayoutHandle) -> Self {
        Self {
            states: HashMap::new(),
            layout,
        }
    }

    pub(super) fn widths(&self, window: &Window, cx: &gpui_kit::App) -> BodyWidths {
        BodyWidths::new(&self.layout, window, cx)
    }

    pub(super) fn resized(
        &self,
        arrangement: &Arrangement,
        sizes: Vec<Pixels>,
        cx: &mut gpui_kit::App,
    ) {
        // An expanded arrangement can deliver its last drag after restoration.
        if arrangement.expanded().is_some() {
            return;
        }
        for (column, size) in arrangement.columns().iter().zip(sizes) {
            let Column::Group { class, .. } = column else {
                continue;
            };
            report_layout(self.layout.set(storage_key(*class), size, cx));
        }
    }

    pub(super) fn widen_group(
        &mut self,
        arrangement: &Arrangement,
        group: GroupKey,
        delta: Pixels,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        let Some(index) = arrangement.position_of(group) else {
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
        arrangement: &Arrangement,
        cx: &mut Context<Workspace>,
    ) -> Entity<ResizableState> {
        self.states
            .entry(arrangement.clone())
            .or_insert_with(|| {
                let arrangement = arrangement.clone();
                let state = cx.new(|_| ResizableState::default());
                let subscription =
                    cx.subscribe(&state, move |this, state, _: &ResizablePanelEvent, cx| {
                        let sizes = state.read(cx).sizes().clone();
                        this.body_resized(&arrangement, sizes, cx);
                    });
                (state, subscription)
            })
            .0
            .clone()
    }
}

/// A class of column: its remembered or default width and the range it may be resized in.
pub(super) struct ClassWidth {
    pub size: Pixels,
    pub range: std::ops::Range<Pixels>,
}

/// The columns' widths and ranges: remembered, or the design's defaults.
pub(super) struct BodyWidths {
    navigation: ClassWidth,
    assistant: ClassWidth,
}

impl BodyWidths {
    pub(super) fn new(memory: &LayoutHandle, window: &Window, cx: &gpui_kit::App) -> Self {
        let rem = window.rem_size();
        let class = |key, class| {
            let (default, range) = width_range(class, rem, cx);
            let size = memory
                .get(key, cx)
                .unwrap_or(default)
                .clamp(range.start, range.end);
            ClassWidth { size, range }
        };
        Self {
            navigation: class(SIDEBAR, WidthClass::Navigation),
            assistant: class(RIGHT_PANEL, WidthClass::Assistant),
        }
    }

    pub(super) fn of(&self, class: WidthClass) -> &ClassWidth {
        match class {
            WidthClass::Navigation => &self.navigation,
            WidthClass::Assistant => &self.assistant,
        }
    }
}

/// The key a class of column remembers its width under.
fn storage_key(class: WidthClass) -> &'static str {
    match class {
        WidthClass::Navigation => SIDEBAR,
        WidthClass::Assistant => RIGHT_PANEL,
    }
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;

pub(super) fn report_layout(result: Result<(), nocterm_store::StoreError>) {
    if let Err(error) = result {
        tracing::warn!(%error, "workspace layout change was not admitted");
    }
}
