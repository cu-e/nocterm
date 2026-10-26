//! The `»` button and the menu of the entries the bar had no room for.

use gpui_kit::{
    Anchor, AnyElement, App, ElementId, FocusHandle, IntoElement, RenderOnce, TestSupportExt as _,
    Window,
    component::{
        Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        popover::Popover,
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_plugin_model::ContributionId;

use super::super::{
    BarEntry, BarInputs,
    row::{BarClick, OnClick},
};
use crate::ContextSelectionReceipt;

/// The least width of the menu.
const MIN_WIDTH: f32 = 160.;

/// The entries the bar hides, behind a button at the end of the bar.
///
/// - A panel's or the terminal's row is a button that does what its toggle does.
/// - A widget's row is the widget's own view, so it behaves as it does in the bar, and the menu
///   stays open while a popup the widget opened has the focus: a press inside that popup is
///   outside the menu's box and must not close the menu under it.
/// - Choosing a row closes the menu.
#[derive(IntoElement)]
pub(in crate::workspace) struct OverflowButton {
    hidden: Vec<BarEntry>,
    open: bool,
    local_terminal_open: bool,
    selection: ContextSelectionReceipt,
    on_click: OnClick,
}

impl OverflowButton {
    /// The button for the entries `inputs` hides; `None` while it hides none.
    pub(in crate::workspace) fn of(inputs: &BarInputs, on_click: OnClick) -> Option<Self> {
        if inputs.hidden().is_empty() {
            return None;
        }
        Some(Self {
            hidden: inputs.hidden().to_vec(),
            open: inputs.overflow().is_open(),
            local_terminal_open: inputs.local_terminal_open(),
            selection: inputs.selection(),
            on_click,
        })
    }
}

impl RenderOnce for OverflowButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            hidden,
            open,
            local_terminal_open,
            selection,
            on_click,
        } = self;
        let toggled = on_click.clone();
        // Nothing in the list holds the focus unless a widget's popup does.
        let inside = window
            .use_keyed_state("status-overflow-focus", cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        Popover::new("status-overflow-menu")
            .anchor(Anchor::BottomRight)
            // A press outside the list closes the menu below, unless a popup of a widget is up.
            .overlay_closable(false)
            .trigger(
                Button::new("status-overflow")
                    .ghost()
                    .small()
                    .label("\u{bb}")
                    .tooltip("More")
                    .selected(open),
            )
            .open(open)
            .on_open_change(move |open, window, cx| toggled(BarClick::Overflow(*open), window, cx))
            .content(move |_, _, _| {
                v_flex()
                    .id("status-overflow-list")
                    .test_support()
                    .track_focus(&inside)
                    .on_mouse_down_out(close_unless_inside(&inside, &on_click))
                    .min_w(px(MIN_WIDTH))
                    .gap_1()
                    .children(
                        hidden.iter().filter_map(|entry| {
                            row(entry, local_terminal_open, selection, &on_click)
                        }),
                    )
            })
    }
}

/// Closes the menu on a press outside it, unless the focus is inside the list, where only a
/// popup a widget opened can have put it.
fn close_unless_inside(
    inside: &FocusHandle,
    on_click: &OnClick,
) -> impl Fn(&gpui_kit::MouseDownEvent, &mut Window, &mut App) + 'static {
    let (inside, on_click) = (inside.clone(), on_click.clone());
    move |_, window, cx| {
        if !inside.contains_focused(window, cx) {
            on_click(BarClick::Overflow(false), window, cx);
        }
    }
}

/// One hidden entry as a row of the menu.
fn row(
    entry: &BarEntry,
    local_terminal_open: bool,
    selection: ContextSelectionReceipt,
    on_click: &OnClick,
) -> Option<AnyElement> {
    match entry {
        BarEntry::Panel(toggle) => {
            let id = toggle.id().clone();
            Some(
                action_row(
                    &id,
                    toggle.look().icon,
                    toggle.look().tooltip.clone(),
                    toggle.is_selected(),
                    on_click,
                    BarClick::Panel(id.clone()),
                )
                .into_any_element(),
            )
        }
        BarEntry::LocalTerminal(look) => {
            let id = entry.id()?;
            Some(
                action_row(
                    &id,
                    look.icon,
                    look.tooltip.clone(),
                    local_terminal_open,
                    on_click,
                    BarClick::LocalTerminal(selection),
                )
                .into_any_element(),
            )
        }
        BarEntry::Widget(id, view) | BarEntry::UserWidget(id, view) => Some(
            div()
                .id((ElementId::from("overflow-widget"), id.as_str().to_owned()))
                .child(view.clone())
                .into_any_element(),
        ),
        BarEntry::Notices(_) | BarEntry::Separator => None,
    }
}

fn action_row(
    id: &ContributionId,
    icon: nocterm_ui::IconName,
    label: gpui_kit::SharedString,
    selected: bool,
    on_click: &OnClick,
    click: BarClick,
) -> Button {
    let on_click = on_click.clone();
    Button::new((ElementId::from("overflow-item"), id.as_str().to_owned()))
        .ghost()
        .small()
        .w_full()
        .justify_start()
        .icon(icon)
        .label(label)
        .selected(selected)
        .on_click(move |_, window, cx| {
            on_click(click.clone(), window, cx);
            on_click(BarClick::Overflow(false), window, cx);
        })
}
