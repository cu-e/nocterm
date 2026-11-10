//! What the layout leaves out of the bar: contributions that can be put back, and ids this
//! application does not know. The tray takes what is dropped on it out of the bar.

use std::{rc::Rc, sync::Arc};

use gpui_kit::{
    AnyElement, App, IntoElement, RenderOnce, SharedString, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        menu::ContextMenuExt as _,
    },
    div,
    prelude::*,
    px,
};
use nocterm_plugin_model::{ContributionId, ContributionRef, PluginSet};
use nocterm_ui::{DragSource, IconName, contribution_icon};
use nocterm_window_layout::{EditOp, Subject};

use crate::{
    commands::Commands,
    dnd::{self, Dragged},
    face::{BORDER, CHIP_HEIGHT, Face, Look},
    menu::{MenuOf, Menus},
    snapshot::Snapshot,
};

#[derive(IntoElement)]
pub(crate) struct Tray {
    snapshot: Rc<Snapshot>,
    catalog: Arc<PluginSet>,
    commands: Commands,
}

impl Tray {
    pub(crate) fn new(snapshot: Rc<Snapshot>, catalog: Arc<PluginSet>, commands: Commands) -> Self {
        Self {
            snapshot,
            catalog,
            commands,
        }
    }

    /// A contribution that waits out of the bar: a click puts it back where its hints say, a
    /// drag puts it anywhere.
    fn entry(&self, contribution: ContributionRef<'_>, menus: &Menus, cx: &App) -> AnyElement {
        let id = contribution.id().clone();
        let subject = Subject::Contribution(id.clone());
        let disabled = self.snapshot.is_disabled(&id, &self.catalog);
        let editable = self.snapshot.editable();
        let add = self.commands.clone();
        let theme = cx.theme();
        let name: SharedString = if disabled {
            format!("{} (disabled)", contribution.title()).into()
        } else {
            contribution.title().to_owned().into()
        };
        let title = SharedString::from(contribution.title().to_owned());
        let face = Face::new(contribution_icon(contribution.icon()), Some(title), false);
        let chip = face
            .clone()
            .fill(h_flex(), cx)
            .id(SharedString::from(format!("layout-tray:{id}")))
            .aria_label(name)
            .test_support()
            .bg(theme.secondary)
            .when(editable, |chip| {
                chip.cursor_pointer()
                    .hover(|style| style.bg(theme.secondary_active))
                    .on_click(move |_, window, cx| {
                        // A release that ends a drag is the drop, not a click.
                        if cx.has_active_drag() {
                            return;
                        }
                        let id = id.clone();
                        add.run(window, cx, move |section, _, cx| {
                            section.edit(EditOp::Restore { id }, cx);
                        });
                    })
            })
            .when(disabled, |chip| chip.opacity(0.5));
        let chip = if editable {
            let source = DragSource::default();
            let measure = source.clone();
            let chip = dnd::measured(chip, BORDER, move |bounds, window, _| {
                measure.capture(bounds, window);
            });
            let dragged = Dragged::contribution(contribution, None);
            dnd::source(chip, dragged, source, move || Look::Chip(face.clone()))
        } else {
            chip
        };
        let (menus, of) = (menus.clone(), MenuOf::Item(subject));
        chip.context_menu(move |popup, window, cx| menus.popup(&of, popup, window, cx))
            .into_any_element()
    }

    fn unknown(&self, id: &ContributionId, cx: &App) -> AnyElement {
        let (commands, forget) = (self.commands.clone(), id.clone());
        let text = format!("Not installed: {id}");
        let theme = cx.theme();
        h_flex()
            .id(SharedString::from(format!("layout-unknown:{id}")))
            .aria_label(text.clone())
            .test_support()
            .flex_none()
            .h(px(CHIP_HEIGHT))
            .gap_1()
            .pl_1p5()
            .items_center()
            .rounded(theme.radius)
            .border_1()
            .border_dashed()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(text)
            .child(
                Button::new(SharedString::from(format!("layout-forget:{id}")))
                    .xsmall()
                    .ghost()
                    .icon(IconName::Close)
                    .tooltip("Forget")
                    .disabled(!self.snapshot.editable())
                    .on_click(move |_, window, cx| {
                        let id = forget.clone();
                        commands.run(window, cx, move |section, _, cx| {
                            section.edit(EditOp::Forget { id }, cx);
                        });
                    }),
            )
            .into_any_element()
    }
}

impl RenderOnce for Tray {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let menus = Menus::new(
            self.snapshot.clone(),
            self.catalog.clone(),
            self.commands.clone(),
        );
        let entries: Vec<AnyElement> = self
            .snapshot
            .tray()
            .iter()
            .filter_map(|id| self.catalog.contribution(id))
            .map(|contribution| self.entry(contribution, &menus, cx))
            .chain(
                self.snapshot
                    .unknown()
                    .iter()
                    .map(|id| self.unknown(id, cx)),
            )
            .collect();
        let theme = cx.theme();
        let tray = h_flex()
            .id("layout-tray")
            .test_support()
            .w_full()
            .flex_wrap()
            .gap_1p5()
            .items_center()
            .min_h(px(CHIP_HEIGHT + 12.))
            .px_2()
            .py_1()
            .rounded(theme.radius_lg)
            .border_1()
            .border_dashed()
            .border_color(theme.border)
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Not in the bar"),
            )
            .children(entries);
        if self.snapshot.editable() {
            dnd::tray(tray, self.commands).into_any_element()
        } else {
            tray.into_any_element()
        }
    }
}
