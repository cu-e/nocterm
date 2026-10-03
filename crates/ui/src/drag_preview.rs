//! A drag label has its own GPUI root, so it must carry its typography and
//! foreground rather than inherit them from the application window.

use gpui_kit::{
    Context, Render, SharedString, Window,
    base::TestSupportExt as _,
    component::{ActiveTheme as _, Icon, Sizable as _, h_flex},
    div,
    prelude::*,
    rems,
};

use crate::{ActiveDesign as _, IconName};

pub struct DragPreview {
    name: SharedString,
    count: usize,
    icon: IconName,
}

impl DragPreview {
    pub fn new(name: impl Into<SharedString>, count: usize, icon: IconName) -> Self {
        Self {
            name: name.into(),
            count,
            icon,
        }
    }

    fn label(&self) -> SharedString {
        if self.count == 1 {
            self.name.clone()
        } else {
            format!("{} items", self.count).into()
        }
    }
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let font_size = cx.design().typography.explorer_size.unwrap_or(12.0);
        h_flex()
            .id("drag-preview")
            .test_support()
            .max_w(rems(20.))
            .min_w(rems(6.))
            .gap_2()
            .px_3()
            .py_2()
            .rounded(theme.radius)
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .shadow_sm()
            .font_family(theme.font_family.clone())
            .text_size(gpui_kit::px(font_size))
            .text_color(theme.popover_foreground)
            .child(
                Icon::new(self.icon)
                    .small()
                    .text_color(theme.popover_foreground),
            )
            .child(
                div()
                    .id("drag-preview-label")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(self.label()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{
        InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent, TestAppContext,
        component::{Theme, ThemeMode},
        point, px,
        test::TestWindowExt as _,
    };

    struct DragFixture;

    impl Render for DragFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                div()
                    .id("drag-source")
                    .test_support()
                    .size(px(60.))
                    .on_drag((), |_, _, _, cx| {
                        cx.new(|_| {
                            DragPreview::new("a-very-long-file-name-".repeat(40), 1, IconName::File)
                        })
                    }),
            )
        }
    }

    #[gpui_kit::test]
    fn long_drag_label_is_bounded_without_an_application_root(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::init(
                crate::DesignTokens::builtin(),
                crate::SettingsStore::in_memory(nocterm_settings::Settings::default()),
                cx,
            );
        });
        let (_, cx) = cx.add_window_view(|_, _| DragFixture);
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            cx.update(|window, cx| {
                Theme::change(mode, None, cx);
                window.render_frame(cx);
                let origin = window.find("drag-source").bounds().center();
                window.dispatch_event(
                    MouseMoveEvent {
                        position: origin,
                        pressed_button: None,
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
                window.render_frame(cx);
                window.dispatch_event(
                    MouseDownEvent {
                        position: origin,
                        button: MouseButton::Left,
                        modifiers: Default::default(),
                        click_count: 1,
                        first_mouse: false,
                    }
                    .to_platform_input(),
                    cx,
                );
                window.render_frame(cx);
                window.dispatch_event(
                    MouseMoveEvent {
                        position: origin + point(px(18.), px(0.)),
                        pressed_button: Some(MouseButton::Left),
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
                window.render_frame(cx);
                let bounds = window.find("drag-preview").bounds();
                assert!(
                    bounds.size.width <= px(320.),
                    "drag label overflowed: {bounds:?}"
                );
                assert!(
                    bounds.size.width >= px(96.),
                    "drag label collapsed: {bounds:?}"
                );
                let label = window.find("drag-preview-label");
                assert!(label.visible() && label.bounds().size.width > px(30.));
                assert!(cx.has_active_drag());
            });
        }
    }
}
