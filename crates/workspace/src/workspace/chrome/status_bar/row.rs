//! The row of the status bar: [`StatusBarRow`] draws the resolved entries and reports clicks.

use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, IntoElement, MouseButton, Pixels, RenderOnce,
    TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        floating::FloatingCards,
        h_flex,
        menu::ContextMenuExt as _,
    },
    div,
    prelude::*,
    px, rems,
};
use nocterm_plugin_model::ContributionId;
use nocterm_ui::{ActiveDesign as _, notice::NoticeBar};

use super::{
    BarEntry, BarInputs, BarMenu,
    inputs::{PanelToggle, Shrink},
};
use crate::{
    ContextSelectionReceipt, Workspace, plugin::LOCAL_TERMINAL, workspace::panel_groups::GroupKey,
};

/// The gap between the entries of a slot, which `gap_1` draws.
const SLOT_GAP: Pixels = px(4.);
/// The width of a toggle button. A badge beside the icon makes the toggle wider than this.
const TOGGLE_WIDTH: Pixels = px(24.);
/// The width of a separator.
const SEPARATOR_WIDTH: Pixels = px(1.);

/// What a fixed entry needs of its slot.
fn fixed_width(entry: &BarEntry) -> Pixels {
    match entry {
        BarEntry::Separator => SEPARATOR_WIDTH,
        _ => TOGGLE_WIDTH,
    }
}

/// What a click on the bar asks of the window.
#[derive(Clone, Debug)]
pub(in crate::workspace) enum BarClick {
    /// The toggle of this panel.
    Panel(ContributionId),
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
    menu: BarMenu,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let pointed = inputs.pointed().clone();
    let on_click: OnClick = {
        let handle = cx.listener(|this, click: &BarClick, window, cx| match click {
            BarClick::Panel(id) => this.toggle_panel(id, window, cx),
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
        // The entry under a right press records itself after this clears what was there.
        .capture_any_mouse_down(move |_, _, _| pointed.set(None))
        .child(StatusBarRow::new(inputs, on_click))
        .context_menu(move |popup, window, cx| menu.build(popup, window, cx))
}

impl RenderOnce for StatusBarRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let border = cx.theme().sidebar_border;
        let floating = FloatingCards::get(cx).is_some();
        let inputs = &self.inputs;
        let row = h_flex()
            .gap_1()
            .px_2()
            .py_1()
            // On the canvas the gap above already separates the footer.
            .when(!floating, |row| row.border_t_1().border_color(border));
        if inputs.center().is_empty() {
            return row
                .children(self.draw(inputs.left(), cx))
                .child(div().flex_1())
                .children(self.draw(inputs.right(), cx));
        }
        // The outer slots share what the centre leaves, equally, so the centre sits at the
        // window's midpoint whenever nothing is cut.
        let centre_max =
            rems(cx.design().layout.status_bar_center_max).to_pixels(window.rem_size());
        row.child(self.slot(inputs.left(), false, cx))
            .child(
                self.slot(inputs.center(), false, cx)
                    .flex_initial()
                    .max_w(centre_max),
            )
            .child(self.slot(inputs.right(), true, cx))
    }
}

impl StatusBarRow {
    /// The entries in order, as the flat bar draws them.
    fn draw(&self, entries: &[BarEntry], cx: &App) -> Vec<AnyElement> {
        entries
            .iter()
            .map(|entry| {
                // The wrapper of an entry that truncates may shrink below its content, as the
                // entry does, or it would hold the entry at full width.
                h_flex()
                    .when(entry.shrink() == Shrink::Elastic, |wrapper| {
                        wrapper.min_w_0()
                    })
                    .on_mouse_down(MouseButton::Right, self.pointing(entry))
                    .child(self.entry(entry, &self.inputs, cx))
                    .into_any_element()
            })
            .collect()
    }

    /// One of the three slots: its entries by shrink class.
    ///
    /// A slot shares the room the centre leaves equally with the other outer slot but never
    /// gives up what its fixed entries need, so the centre and the entries that truncate give way
    /// before a toggle is clipped.
    ///
    /// With `from_end`, free room goes before the entries so they end at the window's edge;
    /// without room they start at the slot's start and the slot clips them at its outer end.
    fn slot(&self, entries: &[BarEntry], from_end: bool, cx: &App) -> Div {
        let fixed = entries
            .iter()
            .filter(|entry| entry.shrink() == Shrink::Fixed)
            .map(fixed_width)
            .fold(px(0.), |sum, width| sum + width);
        let children = entries.len() + usize::from(from_end);
        let gaps = SLOT_GAP * children.saturating_sub(1) as f32;
        h_flex()
            .flex_1()
            .min_w(fixed + gaps)
            .gap_1()
            .overflow_hidden()
            .when(from_end, |slot| slot.child(div().flex_1().min_w_0()))
            .children(entries.iter().map(|entry| {
                let drawn = self.entry(entry, &self.inputs, cx);
                match entry.shrink() {
                    Shrink::Fixed => h_flex().flex_none().child(drawn),
                    Shrink::Elastic => h_flex().min_w_0().overflow_hidden().child(drawn),
                }
                .on_mouse_down(MouseButton::Right, self.pointing(entry))
            }))
    }
}

impl StatusBarRow {
    /// Records the entry a right press lands on, for the context menu.
    fn pointing(
        &self,
        entry: &BarEntry,
    ) -> impl Fn(&gpui_kit::MouseDownEvent, &mut Window, &mut App) + 'static {
        let (pointed, id) = (self.inputs.pointed().clone(), entry.id());
        move |_, _, _| pointed.set(id.clone())
    }

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
            BarEntry::Widget(_, view) => view.clone().into_any_element(),
            BarEntry::Separator => div()
                .w(px(1.))
                .h_4()
                .flex_none()
                .bg(cx.theme().border)
                .into_any_element(),
        }
    }

    fn panel_toggle(&self, toggle: &PanelToggle) -> AnyElement {
        let id = toggle.id().clone();
        let on_click = self.on_click.clone();
        Button::new(toggle_id(toggle.group(), toggle.ix()))
            .ghost()
            .small()
            .icon(toggle.look().icon)
            .tooltip(toggle.look().tooltip.clone())
            .when_some(toggle.badge().cloned(), Button::label)
            .selected(toggle.is_selected())
            .on_click(move |_, window, cx| on_click(BarClick::Panel(id.clone()), window, cx))
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
