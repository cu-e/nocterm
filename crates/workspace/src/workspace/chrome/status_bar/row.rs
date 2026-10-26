//! The row of the status bar: [`StatusBarRow`] draws the resolved entries and reports clicks.

use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, IntoElement, MouseButton, Pixels, RenderOnce,
    TestSupportExt as _, WeakEntity, Window, canvas,
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
use nocterm_ui::{ActiveDesign as _, notice::NoticeBar, zone_anchor};

use super::{
    BarEntry, BarInputs, BarMenu,
    inputs::{PanelToggle, Shrink, USER_WIDGET_MAX_WIDTH},
    overflow::{Frame, OverflowButton, SEPARATOR_WIDTH, SLOT_GAP, TOGGLE_WIDTH},
};
use crate::{ContextSelectionReceipt, Workspace, plugin::LOCAL_TERMINAL};

/// What a fixed entry needs of its slot: its measure once it has been drawn.
fn fixed_width(entry: &BarEntry, inputs: &BarInputs) -> Pixels {
    let measured = entry.id().and_then(|id| inputs.overflow().width_of(&id));
    match entry {
        BarEntry::Separator => SEPARATOR_WIDTH,
        BarEntry::Widget(..) | BarEntry::UserWidget(..) => measured.unwrap_or(px(0.)),
        _ => measured.unwrap_or(TOGGLE_WIDTH).max(TOGGLE_WIDTH),
    }
}

/// What a click on the bar asks of the window.
#[derive(Clone, Debug)]
pub(in crate::workspace) enum BarClick {
    /// The toggle of this panel.
    Panel(ContributionId),
    /// The local terminal toggle, for the selection the bar was drawn under.
    LocalTerminal(ContextSelectionReceipt),
    /// The overflow menu opens or closes.
    Overflow(bool),
}

pub(super) type OnClick = Rc<dyn Fn(BarClick, &mut Window, &mut App)>;

/// The bar's entries over the window's frame.
#[derive(IntoElement)]
pub(super) struct StatusBarRow {
    inputs: BarInputs,
    on_click: OnClick,
    /// The window to draw again when a measure changed what fits.
    workspace: WeakEntity<Workspace>,
}

impl StatusBarRow {
    pub(super) fn new(
        inputs: BarInputs,
        on_click: OnClick,
        workspace: WeakEntity<Workspace>,
    ) -> Self {
        Self {
            inputs,
            on_click,
            workspace,
        }
    }
}

/// The footer: the bar under the body, handling the window's forwarded commands.
pub(in crate::workspace) fn footer(
    inputs: BarInputs,
    menu: BarMenu,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let pointed = inputs.pointed().clone();
    let overflow = inputs.overflow().clone();
    let on_click: OnClick = {
        let handle = cx.listener(move |this, click: &BarClick, window, cx| match click {
            BarClick::Panel(id) => this.toggle_panel(id, window, cx),
            BarClick::LocalTerminal(selection) => {
                if this.accepts_context_selection(*selection, cx) {
                    this.toggle_local_terminal(window, cx);
                }
            }
            BarClick::Overflow(open) => {
                overflow.set_open(*open);
                cx.notify();
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
        .child(StatusBarRow::new(inputs, on_click, cx.weak_entity()))
        .context_menu(move |popup, window, cx| menu.build(popup, window, cx))
}

impl RenderOnce for StatusBarRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let rem = window.rem_size();
        let frame = Frame {
            padding: rems(0.5).to_pixels(rem) * 2.,
            centre_max: rems(cx.design().layout.status_bar_center_max).to_pixels(rem),
        };
        // What does not fit, by what the last frame measured, waits in the overflow menu.
        let Self {
            inputs,
            on_click,
            workspace,
        } = self;
        let inputs = inputs.overflow().clone().collapse(inputs, frame);
        Self::new(inputs, on_click, workspace).paint(frame, cx)
    }
}

impl StatusBarRow {
    fn paint(&self, frame: Frame, cx: &App) -> Div {
        let border = cx.theme().sidebar_border;
        let floating = FloatingCards::get(cx).is_some();
        let inputs = &self.inputs;
        let row = h_flex()
            .relative()
            .gap_1()
            .px_2()
            .py_1()
            // On the canvas the gap above already separates the footer.
            .when(!floating, |row| row.border_t_1().border_color(border))
            .child(self.measure_row());
        if !inputs.is_centred() {
            return row
                .children(self.draw(inputs.left(), cx))
                .child(div().flex_1())
                .children(OverflowButton::of(inputs, self.on_click.clone()))
                .children(self.draw(inputs.right(), cx));
        }
        // The outer slots share what the centre leaves, equally, so the centre sits at the
        // window's midpoint whenever nothing is cut.
        row.child(self.slot(inputs.left(), false, None, cx))
            .child(
                self.slot(inputs.center(), false, None, cx)
                    .flex_initial()
                    .max_w(frame.centre_max),
            )
            .child(self.slot(
                inputs.right(),
                true,
                OverflowButton::of(inputs, self.on_click.clone()),
                cx,
            ))
    }
}

impl StatusBarRow {
    /// The entries in order, as the flat bar draws them.
    fn draw(&self, entries: &[BarEntry], cx: &App) -> Vec<AnyElement> {
        entries
            .iter()
            .map(|entry| {
                // The wrapper of the entry that truncates may shrink below its content, as the
                // entry does, or it would hold the entry at full width.
                self.wrapper(entry, cx)
                    .on_mouse_down(MouseButton::Right, self.pointing(entry))
                    .into_any_element()
            })
            .collect()
    }

    /// One of the three slots: its entries by shrink class, then the overflow button at the end
    /// of the entries that come first.
    ///
    /// A slot shares the room the centre leaves equally with the other outer slot but never
    /// gives up what its fixed entries need, so the centre and the entries that truncate give way
    /// before a toggle is clipped.
    ///
    /// With `from_end`, free room goes before the entries so they end at the window's edge;
    /// without room they start at the slot's start and the slot clips them at its outer end.
    fn slot(
        &self,
        entries: &[BarEntry],
        from_end: bool,
        button: Option<OverflowButton>,
        cx: &App,
    ) -> Div {
        let fixed = entries
            .iter()
            .filter(|entry| entry.shrink() == Shrink::Fixed)
            .map(|entry| fixed_width(entry, &self.inputs))
            .fold(px(0.), |sum, width| sum + width);
        let button_width = if button.is_some() {
            TOGGLE_WIDTH
        } else {
            px(0.)
        };
        let children = entries.len() + usize::from(from_end) + usize::from(button.is_some());
        let gaps = SLOT_GAP * children.saturating_sub(1) as f32;
        h_flex()
            .flex_1()
            .min_w(fixed + button_width + gaps)
            .gap_1()
            .overflow_hidden()
            .when(from_end, |slot| slot.child(div().flex_1().min_w_0()))
            .children(button)
            .children(entries.iter().map(|entry| {
                self.wrapper(entry, cx)
                    .on_mouse_down(MouseButton::Right, self.pointing(entry))
            }))
    }

    /// The entry in a wrapper that sizes it by its shrink class and reports its measure.
    fn wrapper(&self, entry: &BarEntry, cx: &App) -> Div {
        let wrapper = h_flex().relative();
        match entry.shrink() {
            Shrink::Fixed => wrapper.flex_none(),
            Shrink::Elastic => wrapper.min_w_0().overflow_hidden(),
            Shrink::UserPlugin => wrapper
                .min_w_0()
                .max_w(px(USER_WIDGET_MAX_WIDTH))
                .overflow_hidden(),
        }
        .child(self.entry(entry, &self.inputs, cx))
        .children(self.watch(entry))
    }
}

impl StatusBarRow {
    /// Reports the width the row was drawn at.
    fn measure_row(&self) -> impl IntoElement {
        let (overflow, workspace) = (self.inputs.overflow().clone(), self.workspace.clone());
        canvas(
            move |bounds, _, cx| {
                if overflow.observe_row(bounds.size.width) {
                    redraw(workspace, cx);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }

    /// Reports the width an entry was drawn at, if it is one the overflow plans with.
    fn watch(&self, entry: &BarEntry) -> Option<impl IntoElement> {
        let id = entry.id()?;
        let (overflow, workspace) = (self.inputs.overflow().clone(), self.workspace.clone());
        Some(
            canvas(
                move |bounds, _, cx| {
                    if overflow.observe_entry(&id, bounds.size.width) {
                        redraw(workspace, cx);
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
    }

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
            BarEntry::Notices(zone) => NoticeBar::new()
                .anchor(zone_anchor(*zone))
                .into_any_element(),
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
            BarEntry::Widget(_, view) | BarEntry::UserWidget(_, view) => {
                view.clone().into_any_element()
            }
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
        Button::new(toggle.element().clone())
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

/// Draws the window again after the current frame: a measure changed what fits.
fn redraw(workspace: WeakEntity<Workspace>, cx: &mut App) {
    cx.defer(move |cx| {
        let _ = workspace.update(cx, |_, cx| cx.notify());
    });
}
