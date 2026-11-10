//! How an item of the bar looks: one chip, with its icon and, for a widget while there is
//! room, its text; a group of chips on a background of its own; a separator; and the marks of
//! where a drop would land. The preview, the tray and the ghost that follows a drag draw the
//! same boxes, so that a lifted item looks like itself and fills the box it was measured in.

use gpui_kit::{
    AnyElement, App, Div, IntoElement, ParentElement, SharedString, Styled, TestSupportExt as _,
    component::{ActiveTheme as _, Icon, Sizable as _, Size, h_flex},
    div,
    prelude::*,
    px,
};
use nocterm_ui::IconName;

/// How tall a chip is.
pub(crate) const CHIP_HEIGHT: f32 = 28.;
/// The border of a chip, a group and a zone, transparent unless it shows something.
pub(crate) const BORDER: f32 = 1.;
/// The padding of a group around its panels, border included.
pub(crate) const PILL_PADDING: f32 = 2.;
/// How wide the gap between the entries of a zone, or the members of a group, is.
pub(crate) const GAP_WIDTH: f32 = 6.;
/// How thick the line that marks where a drop lands is.
const MARK: f32 = 2.;
/// The id of the one mark of where a drop would land.
const DROP_MARK: &str = "layout-drop-marker";

/// What one chip shows.
#[derive(Clone, Debug)]
pub(crate) struct Face {
    icon: IconName,
    text: Option<SharedString>,
    /// A panel that opens with the layout, which the accent marks.
    open: bool,
}

impl Face {
    pub(crate) fn new(icon: IconName, text: Option<SharedString>, open: bool) -> Self {
        Self { icon, text, open }
    }

    /// `chip` drawn as this face: the box every chip has, and its content.
    pub(crate) fn fill<E>(self, chip: E, cx: &App) -> E
    where
        E: ParentElement + Styled + IntoElement,
    {
        let theme = cx.theme();
        let icon = Icon::new(self.icon)
            .with_size(Size::Medium)
            .when(self.open, |icon| icon.text_color(theme.primary));
        chip.relative()
            .flex_none()
            .h(px(CHIP_HEIGHT))
            .gap_1()
            .px_1p5()
            .items_center()
            .rounded(theme.radius)
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .when(self.open, |chip| {
                chip.bg(theme.primary.opacity(0.14)).child(
                    div()
                        .absolute()
                        .bottom(px(1.))
                        .left(px(6.))
                        .right(px(6.))
                        .h(px(2.))
                        .rounded_full()
                        .bg(theme.primary),
                )
            })
            .child(icon)
            .children(self.text.map(|text| div().text_xs().child(text)))
    }
}

/// The box of a group around its members. An `empty` group shows its title instead.
pub(crate) fn pill(empty: bool, cx: &App) -> Div {
    h_flex()
        .relative()
        .flex_none()
        .h(px(CHIP_HEIGHT + 2. * PILL_PADDING))
        .py(px(PILL_PADDING - BORDER))
        .when(empty, |pill| pill.px(px(PILL_PADDING - BORDER)))
        .items_center()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(gpui_kit::transparent_black())
}

/// The title an empty group shows.
pub(crate) fn title(title: SharedString, cx: &App) -> Div {
    div()
        .px_1p5()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(title)
}

/// A separator between two entries: a short line with a little room on both sides.
pub(crate) fn separator(cx: &App) -> Div {
    div()
        .flex_none()
        .h(px(CHIP_HEIGHT))
        .px(px(2.))
        .flex()
        .items_center()
        .child(div().w(px(1.)).h(px(14.)).bg(cx.theme().border))
}

/// The room between two entries.
pub(crate) fn gap() -> Div {
    div()
        .relative()
        .flex_none()
        .w(px(GAP_WIDTH))
        .h(px(CHIP_HEIGHT))
}

/// The accent line, in a gap, where the item held over the bar would land.
pub(crate) fn line(cx: &App) -> AnyElement {
    div()
        .id(DROP_MARK)
        .test_support()
        .absolute()
        .top(px(2.))
        .bottom(px(2.))
        .left(px((GAP_WIDTH - MARK) / 2.))
        .w(px(MARK))
        .rounded_full()
        .bg(cx.theme().primary)
        .into_any_element()
}

/// The tint on what the item held over the bar would go into: a panel on its own that it would
/// make a group with, or a group without members.
pub(crate) fn tint(cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .id(DROP_MARK)
        .test_support()
        .absolute()
        .top_0()
        .bottom_0()
        .left_0()
        .right_0()
        .rounded(theme.radius)
        .bg(theme.primary.opacity(0.12))
        .border_1()
        .border_color(theme.primary)
        .into_any_element()
}

/// What a dragged item looks like.
#[derive(Clone, Debug)]
pub(crate) enum Look {
    Chip(Face),
    /// A group: its members, `None` for a separator, or the title of an empty one.
    Group {
        members: Vec<Option<Face>>,
        title: SharedString,
    },
}

/// The ghost that follows the pointer: the item as the bar draws it, lifted, filling the box
/// the item was measured in.
pub(crate) fn ghost(look: &Look, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let body = match look {
        Look::Chip(face) => face.clone().fill(h_flex(), cx),
        Look::Group { members, title } if members.is_empty() => {
            pill(true, cx).child(self::title(title.clone(), cx))
        }
        Look::Group { members, .. } => {
            let mut entries = Vec::with_capacity(2 * members.len() + 1);
            for member in members {
                entries.push(gap().into_any_element());
                entries.push(match member {
                    Some(face) => face.clone().fill(h_flex(), cx).into_any_element(),
                    None => separator(cx).into_any_element(),
                });
            }
            entries.push(gap().into_any_element());
            pill(false, cx).children(entries)
        }
    };
    body.id("layout-drag-ghost")
        .test_support()
        .size_full()
        .border_color(theme.primary)
        .bg(theme.popover)
        .shadow_md()
        .into_any_element()
}
