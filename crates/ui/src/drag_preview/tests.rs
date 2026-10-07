use super::*;
use gpui_kit::{
    InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, TestAppContext,
    base::ElementExt as _,
    component::{ActiveTheme as _, Theme, ThemeMode, h_flex},
    point, px,
    test::TestWindowExt as _,
};

const NAME: &str = "資料 — a-very-long-file-name-that-retains-its-source-width.txt";

fn row() -> gpui_kit::Div {
    h_flex().w_full().h_full().px_2().gap_2().child(
        div()
            .id("source-label")
            .test_support()
            .role(gpui_kit::Role::Label)
            .aria_label(NAME)
            .flex_1()
            .min_w_0()
            .truncate()
            .child(NAME),
    )
}

struct Fixture {
    dropped: Rc<RefCell<Vec<Vec<String>>>>,
}
impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let source = DragSource::default();
        let dropped = self.dropped.clone();
        div()
            .size_full()
            .font_family(cx.theme().font_family.clone())
            .text_size(px(21.))
            .child(
                div()
                    .id("drag-source")
                    .test_support()
                    .w(px(420.))
                    .h(px(42.))
                    .on_prepaint({
                        let source = source.clone();
                        move |bounds, window, _| source.capture(bounds, window)
                    })
                    .on_drag(
                        vec![NAME.to_owned(), "second selected file".to_owned()],
                        move |_, _, _, cx| {
                            cx.new(|_| source.preview(move |_, _| row().into_any_element()))
                        },
                    )
                    .child(row()),
            )
            .child(
                div()
                    .w(px(420.))
                    .h(px(80.))
                    .on_drop(move |files: &Vec<String>, _, _| {
                        dropped.borrow_mut().push(files.clone())
                    }),
            )
    }
}

#[gpui_kit::test]
fn native_drag_retains_source_dimensions_label_payload_and_pointer_offset(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::init(
            crate::DesignTokens::builtin(),
            crate::SettingsStore::in_memory(Default::default()),
            cx,
        );
    });
    let dropped = Rc::new(RefCell::new(Vec::new()));
    let (_, cx) = cx.add_window_view(|_, _| Fixture {
        dropped: dropped.clone(),
    });
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update(|window, cx| {
            Theme::change(mode, None, cx);
            window.render_frame(cx);
            let source = window.find("drag-source").bounds();
            let origin = source.origin + point(px(16.), px(18.));
            window.dispatch_event(
                MouseMoveEvent {
                    position: origin,
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
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
            let threshold = origin + point(px(18.), px(0.));
            window.dispatch_event(
                MouseMoveEvent {
                    position: threshold,
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            let preview = window.find("drag-preview").bounds();
            assert_eq!(preview.size, source.size);
            assert_eq!(preview.origin, source.origin);
            assert_eq!(
                window.within("drag-preview").find("source-label").label(),
                Some(NAME)
            );
            assert!(cx.has_active_drag());
            let destination = threshold + point(px(50.), px(28.));
            window.dispatch_event(
                MouseMoveEvent {
                    position: destination,
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            assert_eq!(
                window.find("drag-preview").bounds().origin,
                source.origin + point(px(50.), px(28.))
            );
            window.dispatch_event(
                MouseUpEvent {
                    position: destination,
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
        });
        assert_eq!(
            dropped.borrow().last().unwrap(),
            &[NAME.to_owned(), "second selected file".to_owned()]
        );
    }
}
