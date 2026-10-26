//! Draws the nodes of a validated view.

use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, Hsla, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, div, px, relative,
};
use nocterm_plugin_api::{Event, NodeKind, PluginIcon};

use crate::{
    FrameKind, ViewStyle,
    tree::Tree,
    view::{Hooks, Interaction, Origin},
};

const ICON_SIZE: f32 = 14.;
const METER_WIDTH: f32 = 64.;
const METER_HEIGHT: f32 = 6.;
const LIST_MAX_HEIGHT: f32 = 240.;
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
    kind: FrameKind,
    cx: &'a mut App,
    controls: usize,
}

impl<'a> Painter<'a> {
    pub(crate) fn new(
        tree: &'a Tree,
        style: &'a ViewStyle,
        hooks: &'a Hooks,
        kind: FrameKind,
        cx: &'a mut App,
    ) -> Self {
        Self {
            tree,
            style,
            hooks,
            kind,
            cx,
            controls: 0,
        }
    }

    /// The roots of the view side by side in the bar, one under the other in a panel.
    pub(crate) fn roots(mut self) -> AnyElement {
        let axis = match self.kind {
            FrameKind::Bar => Axis::Row,
            FrameKind::Panel => Axis::Column,
        };
        let tree = self.tree;
        let mut group = self.group(axis);
        for &top in tree.roots() {
            group = group.child(self.node(top, axis));
        }
        group.into_any_element()
    }

    fn group(&self, axis: Axis) -> Div {
        let group = div().flex().gap_1().min_w_0();
        match axis {
            Axis::Row => group.flex_row().items_center(),
            Axis::Column => group.flex_col(),
        }
    }

    fn node(&mut self, index: usize, parent: Axis) -> AnyElement {
        let tree = self.tree;
        let node = tree.node(index);
        let color = self.style.tone(node.tone);
        let key = node.key.clone().unwrap_or_default();
        match &node.kind {
            NodeKind::Row => self.container(self.group(Axis::Row), index, Axis::Row),
            NodeKind::Column => self.container(self.group(Axis::Column), index, Axis::Column),
            NodeKind::List => {
                let list = self
                    .group(Axis::Column)
                    .id(self.id("plugin-list", index))
                    .max_h(px(LIST_MAX_HEIGHT))
                    .overflow_y_scroll();
                self.container(list, index, Axis::Column)
            }
            NodeKind::Text(text) => self.text(text, color),
            NodeKind::Icon(icon) => self.icon(*icon, color),
            NodeKind::Meter(percent) => self.meter(*percent, color),
            NodeKind::Badge(text) => div()
                .flex_none()
                .px_1()
                .rounded_sm()
                .border_1()
                .border_color(color)
                .text_color(color)
                .text_xs()
                .child(SharedString::from(text.clone()))
                .into_any_element(),
            NodeKind::Button(label) => {
                let button = self.control(Event::Activated { key }, color);
                button
                    .px_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(self.style.border)
                    .child(SharedString::from(label.clone()))
                    .into_any_element()
            }
            NodeKind::Toggle(on) => {
                let toggle = self.control(Event::Toggled { key, on: !*on }, color);
                self.switch(toggle, *on, color)
            }
            NodeKind::Divider => self.divider(parent),
            NodeKind::Spacer => div().flex_1().into_any_element(),
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

    fn id(&self, name: &'static str, index: usize) -> ElementId {
        ElementId::from(SharedString::from(format!("{name}-{index}")))
    }

    fn text(&self, text: &str, color: Hsla) -> AnyElement {
        let line = div().min_w_0().text_color(color);
        let line = match self.kind {
            FrameKind::Bar => line.truncate(),
            FrameKind::Panel => line,
        };
        line.child(SharedString::from(text.to_string()))
            .into_any_element()
    }

    fn icon(&self, icon: PluginIcon, color: Hsla) -> AnyElement {
        div()
            .flex_none()
            .size(px(ICON_SIZE))
            .text_color(color)
            .child((self.hooks.icon)(icon))
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
        let line = div().flex_none().bg(self.style.border);
        match parent {
            Axis::Row => line.w(px(1.)).h(px(ICON_SIZE)),
            Axis::Column => line.h(px(1.)).w_full(),
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
            .border_1()
            .border_color(self.style.border);
        match on {
            true => track.justify_end(),
            false => track.justify_start(),
        }
        .child(knob)
        .into_any_element()
    }

    /// A focusable, clickable shell that reports `event` when it is operated.
    fn control(&mut self, event: Event, color: Hsla) -> gpui::Stateful<Div> {
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
            .text_color(color)
            .hover(|hovered| hovered.bg(self.style.hover))
            .focus_visible(|ringed| ringed.border_color(self.style.marker))
            .on_click(move |click: &ClickEvent, window, cx| {
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
