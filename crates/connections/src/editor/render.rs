//! The editor's look: a page list beside titled sections of rows, like the
//! settings tab.
use gpui_kit::{
    AnyElement, App, Context, Entity, SharedString, Window,
    base::TestSupportExt as _,
    component::{
        Sizable as _, h_flex,
        input::{Input, InputState},
        v_flex,
    },
    div,
    prelude::*,
    px, rems,
};
use nocterm_ui::{ActiveDesign as _, IconName, form};

use super::{ConnectionEditor, EditorSection};

/// A text field on the right of its label.
pub(super) fn field_row(
    label: &'static str,
    description: impl Into<SharedString>,
    input: &Entity<InputState>,
    cx: &App,
) -> AnyElement {
    form::row(
        label,
        description,
        div().w(rems(17.)).child(
            Input::new(input)
                .id(format!("editor-field-{label}"))
                .small(),
        ),
        cx,
    )
}

impl EditorSection {
    fn icon(self) -> IconName {
        match self {
            Self::Connection => IconName::Server,
            Self::Authentication => IconName::KeyRound,
            Self::JumpHosts => IconName::Cable,
            Self::Session => IconName::SquareTerminal,
            Self::Launch => IconName::Zap,
        }
    }
}

impl Render for ConnectionEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layout = &cx.design().layout;
        let height = rems(layout.connection_editor_height).to_pixels(window.rem_size());
        let available =
            (window.viewport_size().height - rems(10.).to_pixels(window.rem_size())).max(px(0.));
        let nav_width = rems(layout.connection_nav_width);
        let page = match self.section {
            EditorSection::Connection => self.connection.page(
                &self.launch,
                &self.appearance,
                self.edit.id(),
                self.edit.editing(),
                &self.deps,
                cx,
            ),
            EditorSection::Authentication => self.authentication.page(cx),
            EditorSection::JumpHosts => {
                let labels = self
                    .jump_hosts
                    .labels(self.deps.connections().read(cx).profiles(cx));
                self.jump_hosts.page(labels, cx)
            }
            EditorSection::Session => {
                vec![form::section(
                    "Session",
                    [div().py_3().child(self.options.clone()).into_any_element()],
                    cx,
                )]
            }
            EditorSection::Launch => self.launch.page(cx),
        };
        h_flex()
            .id("connection-editor")
            .track_focus(&self.focus)
            .items_start()
            .h(height.min(available))
            .min_h_0()
            .gap_4()
            .child(v_flex().w(nav_width).flex_shrink_0().gap(px(2.)).children(
                EditorSection::ALL.into_iter().map(|(section, id, label)| {
                    form::nav_item(id, section.icon(), label, self.section == section, cx)
                        .test_support()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_section(section, window, cx)
                        }))
                }),
            ))
            .child(
                v_flex()
                    .id("editor-section-content")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .min_h_0()
                    .pr_2()
                    .overflow_y_scroll()
                    .children(page),
            )
    }
}
