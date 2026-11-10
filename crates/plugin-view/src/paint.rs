//! Draws the nodes of a validated view.

use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, div,
    prelude::FluentBuilder as _, px, relative, transparent_black,
};
use nocterm_plugin_api::{Event, Glyph, NodeKind, Tone};

use crate::{
    FrameKind, ViewStyle,
    components::{
        self, BUTTON_HEIGHT, ICON_BUTTON, RADIUS, ROW_ICON, SMALL_ICON, icon_box, item_body,
        section_heading,
    },
    tree::Tree,
    view::{Folds, Hooks, Interaction, Origin},
};

const METER_WIDTH: f32 = 56.;
const METER_HEIGHT: f32 = 4.;
const CODE_MAX_HEIGHT: f32 = 360.;
const SWITCH_WIDTH: f32 = 24.;
const SWITCH_HEIGHT: f32 = 12.;

/// Which way the parent lays out its children.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    Row,
    Column,
}

/// Draws one view. The controls take their focus handles from the application in the order they
/// are drawn.
pub(crate) struct Painter<'a> {
    tree: &'a Tree,
    style: &'a ViewStyle,
    hooks: &'a Hooks,
    folds: &'a Folds,
    kind: FrameKind,
    cx: &'a mut App,
    controls: usize,
}

impl<'a> Painter<'a> {
    pub(crate) fn new(
        tree: &'a Tree,
        style: &'a ViewStyle,
        hooks: &'a Hooks,
        folds: &'a Folds,
        kind: FrameKind,
        cx: &'a mut App,
    ) -> Self {
        Self {
            tree,
            style,
            hooks,
            folds,
            kind,
            cx,
            controls: 0,
        }
    }

    /// The roots of the view side by side in a bar, one under the other in a panel or a side
    /// bar. A terminal's bar fills its length, so a spacer moves what follows to the end.
    pub(crate) fn roots(mut self) -> AnyElement {
        let axis = if self.kind.is_column() {
            Axis::Column
        } else {
            Axis::Row
        };
        let tree = self.tree;
        let mut group = match axis {
            Axis::Row => self.group(Axis::Row),
            Axis::Column => self.group(Axis::Column).gap_px(),
        };
        if matches!(self.kind, FrameKind::Strip | FrameKind::Rail) {
            group = group.flex_1();
        }
        for &top in tree.roots() {
            group = group.child(self.node(top, axis));
        }
        group.into_any_element()
    }

    fn group(&self, axis: Axis) -> Div {
        let group = div().flex().min_w_0();
        match axis {
            Axis::Row => group.flex_row().items_center().gap_1(),
            Axis::Column => group.flex_col().gap_1(),
        }
    }

    /// A node in its place. In a panel, a node that is not a row of its own (a text, a caption,
    /// a field, a run of buttons) takes the inset of the rows around it.
    fn node(&mut self, index: usize, parent: Axis) -> AnyElement {
        let element = self.element(index, parent);
        let spans = matches!(
            self.tree.node(index).kind,
            NodeKind::Section { .. } | NodeKind::Item { .. } | NodeKind::List | NodeKind::Column
        );
        if self.kind == FrameKind::Panel && parent == Axis::Column && !spans {
            div()
                .px_2()
                .py_0p5()
                .min_w_0()
                .child(element)
                .into_any_element()
        } else {
            element
        }
    }

    fn element(&mut self, index: usize, parent: Axis) -> AnyElement {
        let tree = self.tree;
        let node = tree.node(index);
        let color = self.style.tone(node.tone);
        let key = node.key.clone();
        match &node.kind {
            NodeKind::Row => self.container(self.group(Axis::Row), index, Axis::Row),
            NodeKind::Column => self.container(self.group(Axis::Column), index, Axis::Column),
            NodeKind::List => match self.kind {
                FrameKind::Panel => {
                    self.container(self.group(Axis::Column).gap_px(), index, Axis::Column)
                }
                FrameKind::Rail => self.container(self.group(Axis::Column), index, Axis::Column),
                FrameKind::Bar | FrameKind::Strip => {
                    self.container(self.group(Axis::Row), index, Axis::Row)
                }
            },
            NodeKind::Text(text) => self.text(text, color),
            NodeKind::Icon(glyph) => {
                icon_box(ROW_ICON, color, (self.hooks.icon)(*glyph)).into_any_element()
            }
            NodeKind::Meter(percent) => self.meter(*percent, color),
            NodeKind::Badge(text) => components::badge(text, color),
            NodeKind::Button(label) => self.button(key, label, color),
            NodeKind::Toggle(on) => {
                let event = Event::Toggled {
                    key: key.unwrap_or_default(),
                    on: !*on,
                };
                let toggle = self.control(event);
                self.switch(toggle, *on, color)
            }
            NodeKind::Divider => self.divider(parent),
            NodeKind::Spacer => div().flex_1().into_any_element(),
            NodeKind::Code(text) => self.code(text, index, color),
            NodeKind::Section {
                title,
                count,
                folded,
            } => self.section(index, key, title, count.as_deref(), *folded),
            NodeKind::Item {
                title,
                detail,
                icon,
                selected,
            } => self.item(index, key, title, detail.as_deref(), *icon, *selected),
            NodeKind::IconButton { icon, label } => self.icon_button(key, *icon, label, node.tone),
            NodeKind::Caption(text) => components::caption(self.style, text),
            NodeKind::Field { label, value } => components::field(self.style, label, value, color),
            NodeKind::Chart { series, max, span } => {
                let first = if node.tone == nocterm_plugin_api::Tone::Default {
                    self.style.chart[0]
                } else {
                    color
                };
                let lines = series
                    .iter()
                    .zip([first, self.style.chart[1]])
                    .map(|(values, color)| crate::chart::Line {
                        values: values.clone(),
                        color,
                    })
                    .collect();
                crate::chart::chart(self.kind, lines, *max, *span, self.style.border)
            }
        }
    }

    fn container(
        &mut self,
        mut group: impl ParentElement + IntoElement,
        index: usize,
        axis: Axis,
    ) -> AnyElement {
        let tree = self.tree;
        for &child in tree.children(index) {
            group.extend(Some(self.node(child, axis)));
        }
        group.into_any_element()
    }

    /// A titled group. In a bar it has no heading and its children run on in the bar.
    fn section(
        &mut self,
        index: usize,
        key: Option<String>,
        title: &str,
        count: Option<&str>,
        start_folded: bool,
    ) -> AnyElement {
        if self.kind == FrameKind::Rail {
            return self.container(self.group(Axis::Column), index, Axis::Column);
        }
        if self.kind.is_bar() {
            return self.container(self.group(Axis::Row), index, Axis::Row);
        }
        let folded = key.as_ref().is_some_and(|key| {
            self.folds
                .borrow()
                .get(key)
                .copied()
                .unwrap_or(start_folded)
        });
        let chevron = key.as_ref().map(|_| {
            (self.hooks.icon)(if folded {
                Glyph::ChevronRight
            } else {
                Glyph::ChevronDown
            })
        });
        let heading = section_heading(self.style, title, count, chevron);
        let heading = match key {
            Some(key) => {
                let (folds, redraw) = (self.folds.clone(), self.hooks.redraw.clone());
                heading
                    .id(self.id("plugin-section", index))
                    .debug_selector(|| format!("plugin-section-{index}"))
                    .cursor_pointer()
                    .hover(|hovered| hovered.bg(self.style.hover))
                    .on_click(move |_: &ClickEvent, _, cx| {
                        {
                            let mut folds = folds.borrow_mut();
                            let folded = folds.entry(key.clone()).or_insert(start_folded);
                            *folded = !*folded;
                        }
                        redraw(cx);
                    })
                    .into_any_element()
            }
            None => heading.into_any_element(),
        };
        let mut column = div().flex().flex_col().gap_px().min_w_0().child(heading);
        if !folded {
            for &child in self.tree.children(index) {
                column = column.child(self.node(child, Axis::Column));
            }
        }
        column.into_any_element()
    }

    /// A row of a list. With a key the whole row is a control; its children follow at its end.
    fn item(
        &mut self,
        index: usize,
        key: Option<String>,
        title: &str,
        detail: Option<&str>,
        glyph: Option<Glyph>,
        selected: bool,
    ) -> AnyElement {
        let tone = self.tree.node(index).tone;
        let icon_color = match tone {
            Tone::Default => self.style.muted,
            tone => self.style.tone(tone),
        };
        // The row takes its control slot before its children, in the order they are drawn.
        let shell = match (self.kind, key) {
            (FrameKind::Panel, Some(key)) => Some(self.control(Event::Activated { key })),
            _ => None,
        };
        let trailing: Vec<AnyElement> = self
            .tree
            .children(index)
            .iter()
            .map(|&child| self.node(child, Axis::Row))
            .collect();
        let icon = glyph.map(|glyph| ((self.hooks.icon)(glyph), icon_color));
        if self.kind.is_bar() {
            return div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .min_w_0()
                .children(icon.map(|(icon, color)| icon_box(SMALL_ICON, color, icon)))
                .child(div().truncate().child(SharedString::from(title.to_owned())))
                .children(detail.map(|detail| {
                    div()
                        .truncate()
                        .text_color(self.style.muted)
                        .child(SharedString::from(detail.to_owned()))
                }))
                .children(trailing)
                .into_any_element();
        }
        let body = item_body(self.style, title, detail, icon, trailing);
        let selected_bg = self.style.selected;
        match shell {
            Some(shell) => shell
                .w_full()
                .px_2()
                .py_1()
                .when(selected, |row| row.bg(selected_bg))
                .child(body)
                .into_any_element(),
            None => div()
                .px_2()
                .py_1()
                .rounded(px(RADIUS))
                .when(selected, |row| row.bg(selected_bg))
                .child(body)
                .into_any_element(),
        }
    }

    /// A small borderless button that shows a glyph and names its action in a tooltip.
    fn icon_button(
        &mut self,
        key: Option<String>,
        glyph: Glyph,
        label: &str,
        tone: Tone,
    ) -> AnyElement {
        let color = match tone {
            Tone::Default => self.style.muted,
            tone => self.style.tone(tone),
        };
        let icon = (self.hooks.icon)(glyph);
        let tooltip = self.hooks.tooltip.clone();
        let label = SharedString::from(label.to_owned());
        self.control(Event::Activated {
            key: key.unwrap_or_default(),
        })
        .flex()
        .items_center()
        .justify_center()
        .size(px(ICON_BUTTON))
        .child(icon_box(SMALL_ICON, color, icon))
        .tooltip(move |window, cx| tooltip(label.clone(), window, cx))
        .into_any_element()
    }

    /// A compact outlined button.
    fn button(&mut self, key: Option<String>, label: &str, color: Hsla) -> AnyElement {
        let border = self.style.border;
        self.control(Event::Activated {
            key: key.unwrap_or_default(),
        })
        .h(px(BUTTON_HEIGHT))
        .px_2()
        .flex()
        .items_center()
        .border_color(border)
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .whitespace_nowrap()
        .child(SharedString::from(label.to_owned()))
        .into_any_element()
    }

    /// A scrolled monospace block, one line of the text per row.
    fn code(&self, text: &str, index: usize, color: Hsla) -> AnyElement {
        div()
            .id(self.id("plugin-code", index))
            .flex()
            .flex_col()
            .w_full()
            .min_w_0()
            .max_h(px(CODE_MAX_HEIGHT))
            .overflow_scroll()
            .p_1p5()
            .rounded(px(RADIUS))
            .border_1()
            .border_color(self.style.border.opacity(0.6))
            .font_family(self.hooks.mono.clone())
            .text_xs()
            .text_color(color)
            .children(text.split('\n').map(|line| {
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .child(SharedString::from(line.to_owned()))
            }))
            .into_any_element()
    }

    fn id(&self, name: &'static str, index: usize) -> ElementId {
        ElementId::from(SharedString::from(format!("{name}-{index}")))
    }

    fn text(&self, text: &str, color: Hsla) -> AnyElement {
        let line = div().min_w_0().text_color(color);
        let line = if self.kind.is_bar() {
            line.truncate()
        } else {
            line.text_sm()
        };
        line.child(SharedString::from(text.to_string()))
            .into_any_element()
    }

    fn meter(&self, percent: u8, color: Hsla) -> AnyElement {
        let fraction = f32::from(percent.min(100)) / 100.;
        div()
            .flex_none()
            .w(px(METER_WIDTH))
            .h(px(METER_HEIGHT))
            .rounded_full()
            .bg(self.style.border)
            .child(
                div()
                    .h_full()
                    .w(relative(fraction))
                    .rounded_full()
                    .bg(color),
            )
            .into_any_element()
    }

    fn divider(&self, parent: Axis) -> AnyElement {
        let line = div().flex_none().bg(self.style.border.opacity(0.6));
        match parent {
            Axis::Row => line.w(px(1.)).h(px(ROW_ICON)).mx_0p5(),
            Axis::Column => line.h(px(1.)).w_full().my_1(),
        }
        .into_any_element()
    }

    fn switch(&self, control: gpui::Stateful<Div>, on: bool, color: Hsla) -> AnyElement {
        let knob = div().size(px(SWITCH_HEIGHT - 4.)).rounded_full().bg(if on {
            color
        } else {
            self.style.muted
        });
        let track = control
            .w(px(SWITCH_WIDTH))
            .h(px(SWITCH_HEIGHT))
            .px_0p5()
            .flex()
            .items_center()
            .rounded_full()
            .border_color(self.style.border);
        match on {
            true => track.justify_end(),
            false => track.justify_start(),
        }
        .child(knob)
        .into_any_element()
    }

    /// A focusable, clickable shell that reports `event` when it is operated. A click on a
    /// control inside a row does not reach the row.
    fn control(&mut self, event: Event) -> gpui::Stateful<Div> {
        let slot = self.controls;
        self.controls += 1;
        let handle = (self.hooks.handles)(slot, self.cx);
        let report = self.hooks.event.clone();
        div()
            .id(self.id("plugin-control", slot))
            .debug_selector(|| format!("plugin-control-{slot}"))
            .flex_none()
            .track_focus(&handle)
            .cursor_pointer()
            .rounded(px(RADIUS))
            .border_1()
            .border_color(transparent_black())
            .hover(|hovered| hovered.bg(self.style.hover))
            .focus_visible(|ringed| ringed.border_color(self.style.marker))
            .on_click(move |click: &ClickEvent, window, cx| {
                cx.stop_propagation();
                if !click.standard_click() {
                    return;
                }
                let origin = if click.is_keyboard() {
                    Origin::Key
                } else {
                    Origin::Pointer
                };
                report(
                    Interaction {
                        event: event.clone(),
                        origin,
                    },
                    window,
                    cx,
                );
            })
    }
}
