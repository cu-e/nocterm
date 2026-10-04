use super::*;
use gpui_kit::{
    AnyWindowHandle, Context, TestAppContext, WindowOptions, div, prelude::*,
    test::TestWindowExt as _,
};
use std::cell::Cell;

/// A window with a footer, like the workspace's.
struct Probe;
impl gpui_kit::Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui_kit::IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .child(div().flex_1())
                    .child(NoticeBar::new()),
            )
    }
}

fn window(cx: &mut TestAppContext) -> AnyWindowHandle {
    let handle = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::init(
            crate::DesignTokens::builtin(),
            crate::SettingsStore::in_memory(nocterm_settings::Settings::default()),
            cx,
        );
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Probe))
            .unwrap()
            .0
    });
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
    })
    .unwrap();
    handle
}

#[gpui_kit::test]
fn stable_key_replaces_notice_and_footer_action_runs(cx: &mut TestAppContext) {
    let handle = window(cx);
    let recovered = Rc::new(Cell::new(false));
    cx.update_window(handle, |_, window, cx| {
        assert!(
            window.try_find("notice-dismiss").is_none()
                && window.try_find("notice-toggle").is_none(),
            "an empty footer shows no notices"
        );
        success(window, cx, "same-operation", "Done", "First result");
        warning(window, cx, "same-operation", "Changed", "Second result");
        assert_eq!(count(window, cx), 1);
        let recovered = recovered.clone();
        error_action(
            window,
            cx,
            "same-operation",
            "Needs attention",
            "Third result",
            "Retry",
            move |_, _| recovered.set(true),
        );
        assert_eq!(count(window, cx), 1);
        window.render_frame(cx);
        let action = window.find("notice-action");
        assert!(
            action.visible() && action.bounds().size.width > gpui_kit::px(0.),
            "{action:?}"
        );
        let line = window.find("notice-bar");
        assert!(
            line.bounds().origin.y > window.viewport_size().height / 2.,
            "notices sit in the footer, not over the window: {line:?}"
        );
        window.click("notice-action", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(recovered.get());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(count(window, cx), 0);
        assert!(window.try_find("notice-action").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn transient_notice_leaves_the_line_but_stays_in_the_list(cx: &mut TestAppContext) {
    let handle = window(cx);
    cx.update_window(handle, |_, window, cx| {
        info(window, cx, "copied", "Copied", "Path copied");
        warning_action(
            window,
            cx,
            "partial",
            "Folder size is partial",
            "Limit reached",
            "Recalculate",
            |_, _| {},
        );
        info(window, cx, "saved", "Saved", "Profile saved");
        window.render_frame(cx);
        assert!(window.find("notice-dismiss").visible());
        assert!(
            window.try_find("notice-action").is_none(),
            "the newest notice is on the line"
        );
    })
    .unwrap();
    cx.executor()
        .advance_clock(TRANSIENT + Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.find("notice-action").visible(),
            "the actionable notice stays on the line after transient ones leave"
        );
        assert_eq!(count(window, cx), 3, "transient notices stay in the list");
        window.click("notice-toggle", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.find("notice-list").visible(),
            "the bell opens the list"
        );
        let ids: Vec<u64> = notices(window, cx).notices.iter().map(|n| n.id).collect();
        for id in &ids {
            assert!(window.try_find(("notice-item", *id)).is_some());
        }
        window.click(("notice-item-dismiss", ids[0]), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(count(window, cx), 2);
        window.click("notice-clear", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(count(window, cx), 0);
        assert!(window.try_find("notice-list").is_none());
        assert!(window.try_find("notice-toggle").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn long_text_scrolls_and_short_text_does_not(cx: &mut TestAppContext) {
    let handle = window(cx);
    cx.update_window(handle, |_, window, cx| {
        info(window, cx, "short", "Saved", "Done");
        window.render_frame(cx);
        assert!(window.try_find("notice-marquee").is_none());
        error(
            window,
            cx,
            "long",
            "Transfer failed",
            "The connection to the server was lost while uploading a large archive; nothing was written to the destination folder.",
        );
        window.render_frame(cx);
        assert!(window.find("notice-marquee").visible());
        let text = window.find("notice-text");
        assert!(text.bounds().size.width <= gpui_kit::px(bar::LINE_WIDTH + 1.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn remove_and_run_action_target_one_key_and_one_window(cx: &mut TestAppContext) {
    let first = window(cx);
    let second = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Probe))
                .map(|(handle, _)| handle)
        })
        .unwrap();
    let ran = Rc::new(Cell::new(0));
    cx.update_window(first, |_, window, cx| {
        let ran = ran.clone();
        error_action(window, cx, "retry", "Failed", "x", "Retry", move |_, _| {
            ran.set(ran.get() + 1)
        });
        info(window, cx, "other", "Other", "Keep");
    })
    .unwrap();
    cx.update_window(second, |_, window, cx| {
        assert_eq!(count(window, cx), 0, "notices belong to their window");
        assert!(!run_action(window, cx, "retry"));
    })
    .unwrap();
    cx.update_window(first, |_, window, cx| {
        assert!(
            !run_action(window, cx, "other"),
            "a notice without an action"
        );
        assert!(run_action(window, cx, "retry"));
        assert_eq!(ran.get(), 1);
        assert_eq!(count(window, cx), 1);
        remove(window, cx, "other");
        assert_eq!(count(window, cx), 0);
    })
    .unwrap();
}
