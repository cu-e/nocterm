//! One selectable line of a zone list: the item, a button for its menu and its right-click
//! menu. The line is a drag source for its item and a drop target for the place before it.

use std::rc::Rc;

use gpui_kit::{
    Anchor, AnyElement, App, IntoElement, MouseButton, RenderOnce, SharedString,
    TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        menu::{ContextMenuExt as _, DropdownMenu as _},
    },
    div,
    prelude::*,
};
use nocterm_ui::IconName;
use nocterm_window_layout::Subject;

use crate::{
    commands::Commands,
    dnd::{self, Dragged, Landing, ROW},
    menu::ItemMenu,
};

#[derive(IntoElement)]
pub(crate) struct ItemRow {
    id: SharedString,
    subject: Subject,
    selected: bool,
    indented: bool,
    label: AnyElement,
    menu: Rc<ItemMenu>,
    /// What the line carries when dragged, and where a drop on it lands.
    drag: Option<(Dragged, Landing)>,
    commands: Commands,
}

impl ItemRow {
    pub(crate) fn new(
        id: impl Into<SharedString>,
        subject: Subject,
        label: impl IntoElement,
        menu: Rc<ItemMenu>,
        commands: Commands,
    ) -> Self {
        Self {
            id: id.into(),
            subject,
            selected: false,
            indented: false,
            label: label.into_any_element(),
            menu,
            drag: None,
            commands,
        }
    }

    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Makes the line draggable, and a place that takes a drop at `landing`.
    pub(crate) fn draggable(mut self, dragged: Dragged, landing: Landing) -> Self {
        self.drag = Some((dragged, landing));
        self
    }

    /// A member of a group, set in from its header.
    pub(crate) fn indented(mut self) -> Self {
        self.indented = true;
        self
    }
}

impl RenderOnce for ItemRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let (commands, subject) = (self.commands.clone(), self.subject);
        let (popup, context) = (self.menu.clone(), self.menu);
        let name = match &subject {
            Subject::Contribution(id) => id.to_string(),
            Subject::Group(id) => format!("group:{id}"),
        };
        let more = Button::new(SharedString::from(format!("layout-more:{name}")))
            .ghost()
            .xsmall()
            .icon(IconName::Ellipsis)
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
                popup.popup(menu, window, cx)
            });
        let marker_id = SharedString::from(format!("layout-drop-marker:{name}"));
        let row = h_flex()
            .id(self.id)
            .test_support()
            .group(ROW)
            .relative()
            .w_full()
            .min_w_0()
            .gap_1()
            .items_center()
            .when_else(self.indented, |row| row.pl_6(), |row| row.pl_2())
            .pr_1()
            .py_0p5()
            .rounded(theme.radius)
            .cursor_pointer()
            .when(self.selected, |row| row.bg(theme.foreground.opacity(0.1)))
            .hover(|row| row.bg(theme.foreground.opacity(0.05)))
            .on_click(move |_, window, cx| {
                let subject = subject.clone();
                commands.run(window, cx, move |section, window, cx| {
                    section.pick(Some(subject), window, cx);
                });
            })
            .child(div().flex_1().min_w_0().child(self.label))
            .child(
                h_flex()
                    .flex_shrink_0()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(more),
            );
        let row = match self.drag {
            Some((dragged, landing)) => {
                let row = row.child(dnd::marker(marker_id, ROW, false, landing.clone(), cx));
                let row = dnd::target(row, landing, true, self.commands);
                dnd::source(row, dragged)
            }
            None => row,
        };
        row.context_menu(move |menu, window, cx| context.popup(menu, window, cx))
    }
}
