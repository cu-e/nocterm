//! The row of the status bar: [`StatusBarRow`] draws the resolved entries and reports clicks.

use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, Context, ElementId, IntoElement, RenderOnce, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        floating::FloatingCards,
        h_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_ui::notice::NoticeBar;

use super::{BarEntry, BarInputs, inputs::PanelToggle};
use crate::{
    ContextSelectionReceipt, Workspace, plugin::LOCAL_TERMINAL, workspace::panel_groups::GroupKey,
};

/// What a click on the bar asks of the window.
#[derive(Clone, Debug)]
pub(in crate::workspace) enum BarClick {
    /// The toggle of the panel at this position of its group.
    Panel { group: GroupKey, ix: usize },
    /// The local terminal toggle, for the selection the bar was drawn under.
    LocalTerminal(ContextSelectionReceipt),
}

type OnClick = Rc<dyn Fn(BarClick, &mut Window, &mut App)>;

/// The bar's entries over the window's frame.
#[derive(IntoElement)]
pub(super) struct StatusBarRow {
    inputs: BarInputs,
    on_click: OnClick,
}

impl StatusBarRow {
    pub(super) fn new(inputs: BarInputs, on_click: OnClick) -> Self {
        Self { inputs, on_click }
    }
}

/// The footer: the bar under the body, handling the window's forwarded commands.
pub(in crate::workspace) fn footer(
    inputs: BarInputs,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let on_click: OnClick = {
        let handle = cx.listener(|this, click: &BarClick, window, cx| match click {
            BarClick::Panel { group, ix } => this.toggle_group_panel(*group, *ix, window, cx),
            BarClick::LocalTerminal(selection) => {
                if this.accepts_context_selection(*selection, cx) {
                    this.toggle_local_terminal(window, cx);
                }
            }
        });
        Rc::new(move |click, window, cx| handle(&click, window, cx))
    };
    Workspace::forward_commands(div(), cx)
        .id("workspace-footer")
        .test_support()
        .w_full()
        .flex_shrink_0()
        .child(StatusBarRow::new(inputs, on_click))
}

impl RenderOnce for StatusBarRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let border = cx.theme().sidebar_border;
        let floating = FloatingCards::get(cx).is_some();
        let inputs = &self.inputs;
        let draw = |entries: &[BarEntry], cx: &App| -> Vec<AnyElement> {
            entries
                .iter()
                .map(|entry| self.entry(entry, inputs, cx))
                .collect()
        };
        h_flex()
            .gap_1()
            .px_2()
            .py_1()
            // On the canvas the gap above already separates the footer.
            .when(!floating, |row| row.border_t_1().border_color(border))
            .children(draw(inputs.left(), cx))
            .children(draw(inputs.center(), cx))
            .child(div().flex_1())
            .children(draw(inputs.right(), cx))
    }
}

impl StatusBarRow {
    fn entry(&self, entry: &BarEntry, inputs: &BarInputs, cx: &App) -> AnyElement {
        match entry {
            BarEntry::Panel(toggle) => self.panel_toggle(toggle),
            BarEntry::Notices => NoticeBar::new().into_any_element(),
            BarEntry::LocalTerminal(look) => {
                let on_click = self.on_click.clone();
                let selection = inputs.selection();
                Button::new(status_item(LOCAL_TERMINAL.id.as_str()))
                    .ghost()
                    .small()
                    .icon(look.icon)
                    .tooltip(look.tooltip.clone())
                    .selected(inputs.local_terminal_open())
                    .on_click(move |_, window, cx| {
                        on_click(BarClick::LocalTerminal(selection), window, cx)
                    })
                    .into_any_element()
            }
            BarEntry::Widget(view) => view.clone().into_any_element(),
            BarEntry::Separator => div()
                .w(px(1.))
                .h_4()
                .flex_none()
                .bg(cx.theme().border)
                .into_any_element(),
        }
    }

    fn panel_toggle(&self, toggle: &PanelToggle) -> AnyElement {
        let (group, ix) = (toggle.group(), toggle.ix());
        let on_click = self.on_click.clone();
        Button::new(toggle_id(group, ix))
            .ghost()
            .small()
            .icon(toggle.look().icon)
            .tooltip(toggle.look().tooltip.clone())
            .when_some(toggle.badge().cloned(), Button::label)
            .selected(toggle.is_selected())
            .on_click(move |_, window, cx| on_click(BarClick::Panel { group, ix }, window, cx))
            .into_any_element()
    }
}

/// The id of a status bar item the shell draws itself.
fn status_item(id: &str) -> ElementId {
    (ElementId::from("status-item"), id.to_owned()).into()
}

/// The sidebar's toggles keep their position as id; any other group names its own.
fn toggle_id(group: GroupKey, ix: usize) -> ElementId {
    if group.primary() {
        ("sidebar-panel", ix).into()
    } else {
        (
            ElementId::from("panel-group-toggle"),
            format!("{}:{ix}", group.id()),
        )
            .into()
    }
}
