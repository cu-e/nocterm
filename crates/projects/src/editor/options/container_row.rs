//! Native list-row presentation; container identity stays separate from display text.
use gpui_kit::{
    App, SharedString,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Selectable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_containers::{Container, Image};

pub(super) struct ContainerRow<'a> {
    container: &'a Container,
    images: &'a [Image],
    engine: &'a str,
    selected: bool,
}
impl<'a> ContainerRow<'a> {
    pub(super) fn new(
        container: &'a Container,
        images: &'a [Image],
        engine: &'a str,
        selected: bool,
    ) -> Self {
        Self {
            container,
            images,
            engine,
            selected,
        }
    }
    pub(super) fn render(self, cx: &App) -> Button {
        let key = format!("project-container-{}-{}", self.engine, self.container.id);
        let details = format!(
            "{} · {} · {} · {}",
            self.container.name, self.engine, self.container.image, self.container.status
        );
        Button::new(SharedString::from(key.clone()))
            .ghost()
            .w_full()
            .min_w_0()
            .h_auto()
            .py_2()
            .selected(self.selected)
            .role(gpui_kit::Role::RadioButton)
            .toggled(self.selected)
            .accessibility_label(details.clone())
            .tooltip(details)
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_3()
                    .items_start()
                    .child(self.description(&key, cx))
                    .child(
                        v_flex()
                            .flex_shrink_0()
                            .items_end()
                            .gap_2()
                            .child(
                                div()
                                    .px_1()
                                    .rounded_sm()
                                    .bg(cx.theme().muted)
                                    .text_xs()
                                    .child(self.engine.to_owned()),
                            )
                            .when(self.selected, |view| {
                                view.child(
                                    gpui_kit::component::Icon::new(nocterm_ui::IconName::Check)
                                        .size_4()
                                        .text_color(cx.theme().primary),
                                )
                            }),
                    ),
            )
    }
    fn description(&self, key: &str, cx: &App) -> impl IntoElement + use<> {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(Self::line(
                key,
                "name",
                self.container.name.clone(),
                true,
                cx,
            ))
            .child(Self::line(key, "image", self.image_label(), false, cx))
            .child(Self::line(
                key,
                "status",
                self.container.status.clone(),
                false,
                cx,
            ))
    }
    fn line(
        key: &str,
        field: &str,
        text: String,
        primary: bool,
        cx: &App,
    ) -> impl IntoElement + use<> {
        div()
            .id(SharedString::from(format!("{key}-{field}")))
            .test_support()
            .w_full()
            .min_w_0()
            .truncate()
            .text_left()
            .when(primary, |line| line.text_sm().font_medium())
            .when(!primary, |line| {
                line.text_xs().text_color(cx.theme().muted_foreground)
            })
            .child(text)
    }
    fn image_label(&self) -> String {
        let reference = &self.container.image;
        let digest = reference.strip_prefix("sha256:").unwrap_or(reference);
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return reference.clone();
        }
        if let Some(image) = self.images.iter().find(|image| {
            image.id.strip_prefix("sha256:").unwrap_or(&image.id) == digest
                && image.repository.is_some()
        }) {
            return image.name();
        }
        format!("Image sha256:{}", &digest[..12])
    }
}
