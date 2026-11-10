//! The folder rows of the connections list: group header and its members.
use super::*;

impl ConnectionsPanel {
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub(super) fn render_group_header(
        &self,
        group: &str,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let name = group.to_owned();
        let drop_group = name.clone();
        let renaming = self.renaming.as_ref().filter(|r| r.group == group);
        let label = match renaming {
            Some(rename) => {
                let group = name.clone();
                div()
                    .id(SharedString::from(format!("group-rename-{group}")))
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&rename.input).small())
                    .on_click(|_: &ClickEvent, _, cx| cx.stop_propagation())
                    .on_key_down(
                        cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                            if event.keystroke.key == "escape" {
                                cx.stop_propagation();
                                this.finish_rename(false, true, window, cx);
                            }
                        }),
                    )
                    .into_any_element()
            }
            None => {
                let group = name.clone();
                div()
                    .id(SharedString::from(format!("group-name-{name}")))
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(name.clone())
                    // Keep the header from also treating the second click as a toggle.
                    .on_mouse_down(MouseButton::Left, |event, _, cx| {
                        if event.click_count == 2 {
                            cx.stop_propagation();
                        }
                    })
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        if event.click_count() == 2 {
                            // Undo the first click's toggle, then edit.
                            cx.stop_propagation();
                            this.toggle_group(&group, cx);
                            this.start_rename(&group, window, cx);
                        }
                    }))
                    .into_any_element()
            }
        };
        let open_group = name.clone();
        let delete_group = name.clone();
        h_flex()
            .id(SharedString::from(format!("group-{group}")))
            .test_support()
            .group(GROUP_HEADER)
            .gap_1()
            .mx_1()
            .px_1()
            .py_1()
            .rounded(theme.radius)
            .cursor_pointer()
            .text_xs()
            .font_semibold()
            .text_color(theme.muted_foreground)
            .hover(|row| row.bg(theme.sidebar_accent))
            .drag_over::<DraggedProfile>(|style, _, _, cx| style.bg(cx.theme().accent))
            .on_drop(
                cx.listener(move |this, dragged: &DraggedProfile, window, cx| {
                    this.move_profile(dragged, Some(drop_group.clone()), None, window, cx)
                }),
            )
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .xsmall(),
            )
            .child(
                Icon::new(if collapsed {
                    IconName::FolderClosed
                } else {
                    IconName::FolderOpen
                })
                .small(),
            )
            .child(label)
            .when(renaming.is_none(), |header| {
                header.child(
                    h_flex()
                        .invisible()
                        .group_hover(GROUP_HEADER, |buttons| buttons.visible())
                        .child(
                            Button::new(SharedString::from(format!("group-open-{group}")))
                                .ghost()
                                .xsmall()
                                .icon(IconName::ArrowUpRight)
                                .tooltip("Open every connection of the group")
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.open_group(&open_group, window, cx);
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("group-delete-{group}")))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Trash)
                                .tooltip("Delete group")
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.confirm_delete_group(delete_group.clone(), window, cx);
                                })),
                        ),
                )
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_group(&name, cx)))
    }
}
