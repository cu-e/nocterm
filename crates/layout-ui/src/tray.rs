//! What the layout leaves out of the bar: contributions that can be put back, and ids this
//! application does not know. The tray takes what is dropped on it out of the bar.

use std::{rc::Rc, sync::Arc};

use gpui_kit::{
    Anchor, AnyElement, App, IntoElement, RenderOnce, SharedString, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Icon, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        menu::DropdownMenu as _,
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_plugin_model::{ContributionId, PluginCatalog, SystemContribution};
use nocterm_ui::{IconName, contribution_icon, form};
use nocterm_window_layout::{EditOp, Subject};

use crate::{
    commands::Commands,
    dnd::{self, Dragged, Landing},
    menu::ItemMenu,
    parts::{action, tag},
    snapshot::Snapshot,
};

#[derive(IntoElement)]
pub(crate) struct Tray {
    snapshot: Rc<Snapshot>,
    catalog: Arc<PluginCatalog>,
    selected: Option<Subject>,
    commands: Commands,
}

impl Tray {
    pub(crate) fn new(
        snapshot: Rc<Snapshot>,
        catalog: Arc<PluginCatalog>,
        selected: Option<Subject>,
        commands: Commands,
    ) -> Self {
        Self {
            snapshot,
            catalog,
            selected,
            commands,
        }
    }

    fn entry(&self, contribution: &'static SystemContribution, cx: &App) -> AnyElement {
        let id = contribution.id.clone();
        let subject = Subject::Contribution(id.clone());
        let selected = self.selected.as_ref() == Some(&subject);
        let disabled = self.snapshot.is_disabled(&id, &self.catalog);
        let (pick, add) = (self.commands.clone(), self.commands.clone());
        let restore = id.clone();
        let menu = ItemMenu::new(
            &subject,
            &self.snapshot,
            &self.catalog,
            self.commands.clone(),
        );
        let more = Button::new(SharedString::from(format!("layout-more:{id}")))
            .ghost()
            .xsmall()
            .icon(IconName::Ellipsis)
            .dropdown_menu_with_anchor(Anchor::TopRight, move |popup, window, cx| {
                menu.popup(popup, window, cx)
            });
        let theme = cx.theme();
        let chip = h_flex()
            .id(SharedString::from(format!("layout-tray:{id}")))
            .aria_label(contribution.title)
            .test_support()
            .gap_1()
            .px_2()
            .py_1()
            .items_center()
            .rounded(theme.radius)
            .border_1()
            .border_color(if selected {
                theme.primary
            } else {
                theme.border
            })
            .cursor_pointer()
            .when(disabled, |chip| chip.opacity(0.5))
            .on_click(move |_, window, cx| {
                let subject = subject.clone();
                pick.run(window, cx, move |section, window, cx| {
                    section.pick(Some(subject), window, cx);
                });
            })
            .child(Icon::new(contribution_icon(contribution.icon)).small())
            .child(div().text_sm().child(contribution.title))
            .when(disabled, |chip| {
                chip.child(tag(format!("layout-tray-disabled:{id}"), "Disabled", cx))
            });
        let chip = if self.snapshot.editable() {
            dnd::source(chip, Dragged::contribution(contribution, &self.snapshot))
        } else {
            chip
        };
        h_flex()
            .gap_1()
            .items_center()
            .child(chip)
            .child(action(
                SharedString::from(format!("layout-tray-add:{id}")),
                "Add",
                self.snapshot.editable(),
                move |window, cx| {
                    let id = restore.clone();
                    add.run(window, cx, move |section, _, cx| {
                        section.edit(EditOp::Restore { id }, cx);
                    });
                },
            ))
            .child(more)
            .into_any_element()
    }

    fn unknown(&self, id: &ContributionId, cx: &App) -> AnyElement {
        let (commands, forget) = (self.commands.clone(), id.clone());
        let text = format!("Not installed: {id}");
        h_flex()
            .gap_1()
            .items_center()
            .child(
                div()
                    .id(SharedString::from(format!("layout-unknown:{id}")))
                    .aria_label(text.clone())
                    .test_support()
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_dashed()
                    .border_color(cx.theme().border)
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(text),
            )
            .child(action(
                SharedString::from(format!("layout-forget:{id}")),
                "Forget",
                self.snapshot.editable(),
                move |window, cx| {
                    let id = forget.clone();
                    commands.run(window, cx, move |section, _, cx| {
                        section.edit(EditOp::Forget { id }, cx);
                    });
                },
            ))
            .into_any_element()
    }
}

impl RenderOnce for Tray {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let empty = self.snapshot.tray().is_empty() && self.snapshot.unknown().is_empty();
        let entries: Vec<AnyElement> = self
            .snapshot
            .tray()
            .iter()
            .map(|contribution| self.entry(contribution, cx))
            .chain(
                self.snapshot
                    .unknown()
                    .iter()
                    .map(|id| self.unknown(id, cx)),
            )
            .collect();
        let theme = cx.theme();
        let tray = v_flex()
            .id("layout-tray")
            .test_support()
            .w_full()
            .min_h_20()
            .gap_2()
            .p_2()
            .rounded(theme.radius)
            .border_1()
            .border_dashed()
            .border_color(theme.border)
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child("NOT IN THE BAR"),
            )
            .when(empty, |tray| {
                tray.child(form::note("Everything is in the bar.", cx))
            })
            .child(h_flex().flex_wrap().gap_x_4().gap_y_2().children(entries));
        if self.snapshot.editable() {
            dnd::target(tray, Landing::tray(), true, self.commands).into_any_element()
        } else {
            tray.into_any_element()
        }
    }
}
