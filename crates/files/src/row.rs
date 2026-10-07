//! Shared passive appearance for local and remote Explorer entries.
use gpui_kit::{
    App, Div,
    base::TestSupportExt as _,
    component::{ActiveTheme as _, Icon, Sizable as _},
    div,
    prelude::*,
    px,
};
use nocterm_ui::IconName;

#[derive(Clone)]
pub(super) struct ExplorerRow {
    pub(super) name: String,
    pub(super) directory: bool,
    pub(super) symlink: bool,
    pub(super) selected: bool,
    pub(super) font_size: f32,
}

impl ExplorerRow {
    pub(super) fn render(&self, hovered: bool, cx: &App) -> Div {
        div()
            .w_full()
            .h_8()
            .font_family(cx.theme().font_family.clone())
            .text_color(cx.theme().foreground)
            .text_size(px(self.font_size))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded_sm()
            .when(self.selected || hovered, |row| row.bg(cx.theme().accent))
            .hover(|row| row.bg(cx.theme().accent))
            .child(
                div()
                    .id("explorer-row-icon")
                    .test_support()
                    .flex_shrink_0()
                    .child(
                        Icon::new(if self.directory {
                            IconName::Folder
                        } else {
                            IconName::File
                        })
                        .small(),
                    ),
            )
            .child(
                div()
                    .id("explorer-row-name")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(format!(
                        "{}{}",
                        self.name,
                        if self.symlink { " (link)" } else { "" }
                    )),
            )
    }
}
