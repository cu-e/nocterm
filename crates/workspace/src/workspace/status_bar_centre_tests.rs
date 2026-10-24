//! A status bar with a centre draws three slots: the centre stays at the window's midpoint, and
//! a narrow window cuts text before it cuts a toggle.
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, IntoElement, Render, TestAppContext,
    TestSupportExt as _, Window, div, prelude::*, px, size, test::TestWindowExt as _,
};
use nocterm_settings::SettingsDocument;
use nocterm_store::SettingsConfig;
use nocterm_window_layout::LayoutSettings;

use super::{PanelRegistration, WidgetRegistration, Workspace};
use crate::{
    test_support::{
        EXPANDABLE, PANEL_A, PANEL_B, TEST_PLUGIN, TestPanel, Toggle, WIDGET, deps_over, toggle_id,
    },
    workspace::tests::fixture_with_deps,
};

/// A centre widget of a width that tests can rely on.
struct Centre;
impl Render for Centre {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("centre-marker")
            .test_support()
            .w(px(160.))
            .h(px(20.))
    }
}

/// The layout the tests use: toggles on the left, a widget in the centre, and the notices and
/// the side panel on the right.
fn stored() -> LayoutSettings {
    layout(
        serde_json::json!({
            "left": [{
                "group": "navigation",
                "primary": true,
                "members": ["test.panel-a", "test.panel-b"],
            }],
            "center": [{ "item": "test.widget" }],
            "right": [
                { "item": "workspace.notifications" },
                { "group": "assistant", "members": ["test.expandable"] },
            ],
        }),
        &[],
    )
}

/// A layout with the given `bar`, leaving the `tray` panels unplaced.
fn layout(bar: serde_json::Value, tray: &[&str]) -> LayoutSettings {
    serde_json::from_value(serde_json::json!({
        "active": "p1",
        "presets": [{ "id": "p1", "name": "Centred", "bar": bar, "tray": tray }],
    }))
    .expect("the stored layout is valid")
}

struct Bar {
    handle: AnyWindowHandle,
    workspace: Entity<Workspace>,
    side_panel: Entity<TestPanel>,
}

/// A window of `width` pixels whose bar has a centre.
fn centred_bar(cx: &mut TestAppContext, width: f32) -> Bar {
    bar_of(cx, width, stored())
}

/// A window of `width` pixels whose bar is arranged by `layouts`.
fn bar_of(cx: &mut TestAppContext, width: f32, layouts: LayoutSettings) -> Bar {
    let (handle, workspace, _) = fixture_with_deps(cx, |layout, cx| {
        let mut document = SettingsDocument::default();
        document.register::<LayoutSettings>();
        document.set(layouts);
        let settings = SettingsConfig::in_memory(document).install(cx);
        deps_over(layout, &[], &[&TEST_PLUGIN]).with_settings(settings)
    });
    let side_panel = cx
        .update_window(handle, |_, window, cx| {
            window.resize(size(px(width), px(700.)));
            workspace.update(cx, |workspace, cx| {
                for contribution in [PANEL_A, PANEL_B] {
                    let panel = TestPanel::builder(contribution.title).build(cx);
                    workspace
                        .register_panel(
                            PanelRegistration::new(contribution, move |_, _, _| panel.clone()),
                            window,
                            cx,
                        )
                        .expect("the panel is in the test catalog");
                }
                workspace
                    .register_widget(
                        WidgetRegistration::new(WIDGET, |_, _, cx| cx.new(|_| Centre)),
                        window,
                        cx,
                    )
                    .expect("the widget is in the test catalog");
                let side_panel = TestPanel::builder("side").title("Side").build(cx);
                let registered = side_panel.clone();
                workspace
                    .register_panel(
                        PanelRegistration::new(EXPANDABLE, move |_, _, _| registered.clone())
                            .expandable(),
                        window,
                        cx,
                    )
                    .expect("the panel is in the test catalog");
                side_panel
            })
        })
        .unwrap();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    Bar {
        handle,
        workspace,
        side_panel,
    }
}

#[gpui_kit::test]
fn the_centre_sits_at_the_window_midpoint_when_nothing_is_cut(cx: &mut TestAppContext) {
    let bar = centred_bar(cx, 1280.);
    cx.update_window(bar.handle, |_, window, cx| {
        window.render_frame(cx);
        let footer = window.find("workspace-footer").bounds();
        assert_eq!(footer.size.width, px(1280.));
        let centre = window.find("centre-marker").bounds();
        let midpoint = footer.left() + footer.size.width / 2.;
        let drift = (centre.center().x - midpoint).abs();
        assert!(drift <= px(1.), "centre {centre:?} in footer {footer:?}");
        let first = window.find(toggle_id(Toggle::Navigation(0))).bounds();
        assert!(
            first.right() < centre.left(),
            "the toggles stay on the left"
        );
        let side = window.find(toggle_id(Toggle::SidePanel)).bounds();
        assert!(
            centre.right() < side.left(),
            "the side panel stays on the right"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_narrow_window_cuts_the_notice_text_before_the_bell_and_the_attention_toggle(
    cx: &mut TestAppContext,
) {
    let bar = centred_bar(cx, 520.);
    cx.update_window(bar.handle, |_, window, cx| {
        nocterm_ui::notice::info(
            window,
            cx,
            "centre-test",
            "A notice",
            "with a message much longer than the room the narrow window leaves it",
        );
        bar.side_panel.update(cx, |panel, cx| {
            panel.set_attention(Some("permission".into()), cx)
        });
        bar.workspace.update(cx, |_, cx| cx.notify());
        window.render_frame(cx);
        let footer = window.find("workspace-footer").bounds();
        assert_eq!(footer.size.width, px(520.));
        let centre = window.find("centre-marker").bounds();
        let bell = window.find("notice-toggle").bounds();
        let side = window.find(toggle_id(Toggle::SidePanel)).bounds();
        let text = window.find("notice-text").bounds();
        let edge = footer.right() - px(8.);
        assert!(
            bell.left() >= centre.right(),
            "bell {bell:?} clear of {centre:?}"
        );
        assert!(bell.right() <= side.left(), "bell {bell:?} before {side:?}");
        assert!(
            side.right() <= edge,
            "attention toggle {side:?} within {footer:?}"
        );
        assert!(
            text.right() <= bell.left(),
            "text {text:?} gave way to {bell:?}"
        );
        assert!(
            text.size.width < px(340.),
            "the text was cut: {text:?} in {footer:?}"
        );
    })
    .unwrap();
}

fn say_much(window: &mut Window, cx: &mut gpui_kit::App) {
    nocterm_ui::notice::info(
        window,
        cx,
        "centre-test",
        "A notice",
        "with a message much longer than the room the narrow window leaves it",
    );
}

#[gpui_kit::test]
fn notices_in_the_centre_give_way_before_the_toggles_beside_them(cx: &mut TestAppContext) {
    let layouts = layout(
        serde_json::json!({
            "left": [{
                "group": "navigation",
                "primary": true,
                "members": ["test.panel-a", "test.panel-b"],
            }],
            "center": [{ "item": "workspace.notifications" }],
            "right": [{ "group": "assistant", "members": ["test.expandable"] }],
        }),
        &["test.widget"],
    );
    let wide = bar_of(cx, 1280., layouts.clone());
    let toggle_width = |bar: &Bar, cx: &mut TestAppContext| {
        cx.update_window(bar.handle, |_, window, cx| {
            say_much(window, cx);
            bar.workspace.update(cx, |_, cx| cx.notify());
            window.render_frame(cx);
            (
                window.find(toggle_id(Toggle::Navigation(1))).bounds(),
                window.find(toggle_id(Toggle::SidePanel)).bounds(),
            )
        })
        .unwrap()
    };
    let (open_left, open_right) = toggle_width(&wide, cx);
    let narrow = bar_of(cx, 520., layouts);
    let (left, right) = toggle_width(&narrow, cx);
    assert_eq!(left.size, open_left.size, "the left toggle keeps its size");
    assert_eq!(
        right.size, open_right.size,
        "the right toggle keeps its size"
    );
    cx.update_window(narrow.handle, |_, window, _| {
        let footer = window.find("workspace-footer").bounds();
        let text = window.find("notice-text").bounds();
        let bell = window.find("notice-toggle").bounds();
        let edge = footer.right() - px(8.);
        assert!(right.right() <= edge, "{right:?} inside {footer:?}");
        assert!(
            left.right() <= bell.left(),
            "{left:?} before the notice {bell:?}"
        );
        assert!(
            text.right() <= bell.left() && text.size.width < px(340.),
            "text {text:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_panel_asking_for_the_user_stays_inside_a_narrow_bar_with_the_notices(cx: &mut TestAppContext) {
    let layouts = layout(
        serde_json::json!({
            "left": [{
                "group": "navigation",
                "primary": true,
                "members": ["test.panel-a", "test.panel-b"],
            }],
            "center": [{ "item": "workspace.notifications" }],
            "right": [{ "item": "workspace.local-terminal" }],
        }),
        &["test.expandable", "test.widget"],
    );
    let bar = bar_of(cx, 520., layouts);
    cx.update_window(bar.handle, |_, window, cx| say_much(window, cx))
        .unwrap();
    cx.update(|cx| {
        bar.side_panel.update(cx, |panel, cx| {
            panel.set_attention(Some("permission".into()), cx)
        })
    });
    cx.update_window(bar.handle, |_, window, cx| {
        window.render_frame(cx);
        let footer = window.find("workspace-footer").bounds();
        let toggle = window.find(toggle_id(Toggle::SidePanel)).bounds();
        let terminal = window.find(toggle_id(Toggle::LocalTerminal)).bounds();
        let bell = window.find("notice-toggle").bounds();
        let edge = footer.right() - px(8.);
        assert!(
            bell.right() <= toggle.left() && toggle.right() <= terminal.left(),
            "the attention toggle {toggle:?} stays between {bell:?} and {terminal:?}"
        );
        assert!(
            terminal.right() <= edge,
            "the terminal toggle {terminal:?} is whole inside {footer:?}"
        );
    })
    .unwrap();
}
