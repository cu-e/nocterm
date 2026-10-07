//! The same passive server row supplies sidebar content and its drag appearance.
use super::*;
use gpui_kit::{Div, ObjectFit, base::ElementExt as _};
use nocterm_ui::{DragSource, IconName};

#[derive(Clone)]
struct RowVisual {
    profile: Profile,
    os: Option<&'static Os>,
    flag: Option<(String, Arc<Image>)>,
}

impl RowVisual {
    fn new(profile: &Profile, cx: &App) -> Self {
        let facts = ServerFacts::global(cx).read(cx);
        Self {
            profile: profile.clone(),
            os: profile
                .icon
                .as_deref()
                .and_then(os::find)
                .or_else(|| facts.os(profile.id)),
            flag: facts
                .shown_country(profile, cx)
                .and_then(|code| Some((code.to_uppercase(), facts.flag(code)?))),
        }
    }

    fn buttons(&self) -> (Button, Button) {
        let id = self.profile.id;
        (
            Button::new(SharedString::from(format!("edit-{id}")))
                .ghost()
                .xsmall()
                .icon(IconName::Pencil),
            Button::new(SharedString::from(format!("delete-{id}")))
                .ghost()
                .xsmall()
                .icon(IconName::Trash),
        )
    }

    fn render(&self, hovered: bool, actions: AnyElement, cx: &App) -> Div {
        let theme = cx.theme();
        let profile = &self.profile;
        h_flex()
            .w_full()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .when(hovered, |row| row.bg(theme.sidebar_accent))
            .hover(|row| row.bg(theme.sidebar_accent))
            .child(
                div()
                    .id(SharedString::from(format!(
                        "connection-icon-{}",
                        profile.id
                    )))
                    .test_support()
                    .flex_shrink_0()
                    .child(server_icon(
                        self.os,
                        profile
                            .icon_color
                            .as_deref()
                            .filter(|color| os::is_valid_color(color)),
                        theme.muted_foreground,
                    )),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap_1p5()
                            .min_w_0()
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "connection-name-{}",
                                        profile.id
                                    )))
                                    .test_support()
                                    .text_sm()
                                    .truncate()
                                    .child(profile.name.clone()),
                            )
                            .when_some(self.flag.clone(), |line, (country, flag)| {
                                line.child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "connection-flag-{}",
                                            profile.id
                                        )))
                                        .test_support()
                                        .flex_shrink_0()
                                        .rounded_xs()
                                        .overflow_hidden()
                                        .border_1()
                                        .border_color(theme.border)
                                        .child(
                                            img(flag)
                                                .w(gpui_kit::px(16.))
                                                .h(gpui_kit::px(11.))
                                                .object_fit(ObjectFit::Cover),
                                        )
                                        .when(!hovered, |flag| {
                                            flag.tooltip(move |_, cx| {
                                                cx.new(|_| {
                                                    gpui_kit::component::tooltip::Tooltip::new(
                                                        country.clone(),
                                                    )
                                                })
                                                .into()
                                            })
                                        }),
                                )
                            }),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "connection-target-{}",
                                profile.id
                            )))
                            .test_support()
                            .text_xs()
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .child(profile.target.to_string()),
                    )
                    .when(!profile.description.is_empty(), |column| {
                        column.child(
                            div()
                                .id(SharedString::from(format!(
                                    "connection-description-{}",
                                    profile.id
                                )))
                                .test_support()
                                .text_xs()
                                .truncate()
                                .text_color(theme.muted_foreground)
                                .child(description_preview(&profile.description)),
                        )
                    }),
            )
            .child(actions)
    }

    fn preview(&self, cx: &App) -> AnyElement {
        let (edit, delete) = self.buttons();
        self.render(
            true,
            h_flex().child(edit).child(delete).into_any_element(),
            cx,
        )
        .into_any_element()
    }
}

fn insertion_marker(id: SharedString, cx: &App) -> impl IntoElement {
    let color = cx.theme().border;
    div()
        .id(id)
        .test_support()
        .cursor_pointer()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(gpui_kit::px(1.))
        .invisible()
        .group_drag_over::<DraggedProfile>(ROW_GROUP, move |line| line.visible().bg(color))
}

impl ConnectionsPanel {
    pub(super) fn render_append_zone(
        &self,
        group: Option<&str>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = match group {
            Some(group) => format!("connections-append-group-{group}"),
            None => "connections-append-root".to_owned(),
        };
        let marker_id = SharedString::from(format!("{id}-marker"));
        let group = group.map(str::to_owned);
        div()
            .id(SharedString::from(id))
            .test_support()
            .group(ROW_GROUP)
            .relative()
            .mx_1()
            .h_2()
            .cursor_pointer()
            .child(insertion_marker(marker_id, cx))
            .on_drop(
                cx.listener(move |this, dragged: &DraggedProfile, window, cx| {
                    this.move_profile(dragged, group.clone(), None, window, cx)
                }),
            )
    }

    pub(super) fn render_row(&self, profile: &Profile, cx: &mut Context<Self>) -> impl IntoElement {
        let id = profile.id;
        let visual = RowVisual::new(profile, cx);
        let source = DragSource::default();
        let source_visual = visual.clone();
        let group = profile.group.clone();
        let (edit, delete) = visual.buttons();
        let actions = h_flex()
            .invisible()
            .group_hover(ROW_GROUP, |buttons| buttons.visible())
            .child(edit.tooltip("Edit").on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.edit(id, window, cx);
                },
            )))
            .child(delete.tooltip("Delete").on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.confirm_delete(id, window, cx);
                },
            )))
            .into_any_element();
        div()
            .id(SharedString::from(format!("connection-{id}")))
            .test_support()
            .group(ROW_GROUP)
            .relative()
            .mx_1()
            .cursor_pointer()
            .on_prepaint({
                let source = source.clone();
                move |bounds, window, _| source.capture(bounds, window)
            })
            .on_drag(DraggedProfile(id), move |_, _, _, cx| {
                cx.stop_propagation();
                let visual = visual.clone();
                cx.new(|_| source.preview(move |_, cx| visual.preview(cx)))
            })
            .on_drop(
                cx.listener(move |this, dragged: &DraggedProfile, window, cx| {
                    this.move_profile(dragged, group.clone(), Some(id), window, cx)
                }),
            )
            .when(!profile.description.is_empty(), |row| {
                let description = description_tooltip(&profile.description);
                row.tooltip(move |_, cx| {
                    cx.new(|_| gpui_kit::component::tooltip::Tooltip::new(description.clone()))
                        .into()
                })
            })
            .child(source_visual.render(false, actions, cx))
            // Overlay outside the rounded body: no corner clipping or layout shift.
            .child(insertion_marker(
                SharedString::from(format!("connection-drop-marker-{id}")),
                cx,
            ))
            .on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| this.open(id, window, cx)),
            )
    }
}
