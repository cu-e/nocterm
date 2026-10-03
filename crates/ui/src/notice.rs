//! Window-side operational notices using gpui-kit's native notification list.

use std::rc::Rc;

use gpui_kit::{
    Anchor, App, SharedString, Window,
    component::{WindowExt as _, button::Button, notification::Notification},
};

struct NoticeKey;

fn bounded(message: impl Into<SharedString>) -> SharedString {
    let message: SharedString = message.into();
    let mut chars = message.chars();
    let text: String = chars.by_ref().take(512).collect();
    if chars.next().is_some() {
        format!("{text}…").into()
    } else {
        text.into()
    }
}

fn post(window: &mut Window, cx: &mut App, notification: Notification, key: &'static str) {
    window.push_notification(
        notification
            .id1::<NoticeKey>(key)
            .placement(Anchor::RightCenter),
        cx,
    );
}

pub fn error(
    window: &mut Window,
    cx: &mut App,
    key: &'static str,
    title: &'static str,
    message: impl Into<SharedString>,
) {
    post(
        window,
        cx,
        Notification::error(bounded(message)).title(title),
        key,
    );
}

pub fn warning(
    window: &mut Window,
    cx: &mut App,
    key: &'static str,
    title: &'static str,
    message: impl Into<SharedString>,
) {
    post(
        window,
        cx,
        Notification::warning(bounded(message)).title(title),
        key,
    );
}

pub fn success(
    window: &mut Window,
    cx: &mut App,
    key: &'static str,
    title: &'static str,
    message: impl Into<SharedString>,
) {
    post(
        window,
        cx,
        Notification::success(bounded(message)).title(title),
        key,
    );
}

pub fn info(
    window: &mut Window,
    cx: &mut App,
    key: &'static str,
    title: &'static str,
    message: impl Into<SharedString>,
) {
    post(
        window,
        cx,
        Notification::info(bounded(message)).title(title),
        key,
    );
}

/// An actionable error stays visible until the user chooses the recovery or
/// dismisses it. Repeating the same `key` replaces the earlier notice.
pub fn error_action(
    window: &mut Window,
    cx: &mut App,
    key: &'static str,
    title: &'static str,
    message: impl Into<SharedString>,
    action_label: &'static str,
    action: impl Fn(&mut Window, &mut App) + 'static,
) {
    post_action(
        window,
        cx,
        Notification::error(bounded(message)),
        key,
        title,
        action_label,
        action,
    );
}

pub fn warning_action(
    window: &mut Window,
    cx: &mut App,
    key: &'static str,
    title: &'static str,
    message: impl Into<SharedString>,
    action_label: &'static str,
    action: impl Fn(&mut Window, &mut App) + 'static,
) {
    post_action(
        window,
        cx,
        Notification::warning(bounded(message)),
        key,
        title,
        action_label,
        action,
    );
}

fn post_action(
    window: &mut Window,
    cx: &mut App,
    notification: Notification,
    key: &'static str,
    title: &'static str,
    action_label: &'static str,
    action: impl Fn(&mut Window, &mut App) + 'static,
) {
    let action = Rc::new(action);
    let notification = notification.title(title).action(move |_, _, cx| {
        let action = action.clone();
        Button::new("notice-action")
            .label(action_label)
            .on_click(cx.listener(move |notice, _, window, cx| {
                action(window, cx);
                notice.dismiss(window, cx);
            }))
    });
    post(window, cx, notification, key);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{
        Context, TestAppContext, WindowOptions, div, prelude::*, test::TestWindowExt as _,
    };
    use std::{cell::Cell, rc::Rc};

    struct Probe;
    impl gpui_kit::Render for Probe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui_kit::IntoElement {
            div().size_full()
        }
    }

    #[gpui_kit::test]
    fn stable_key_replaces_notice_and_recovery_action_runs(cx: &mut TestAppContext) {
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
        let recovered = Rc::new(Cell::new(false));
        cx.update_window(handle, |_, window, cx| {
            window.activate_window();
            success(window, cx, "same-operation", "Done", "First result");
            warning(window, cx, "same-operation", "Changed", "Second result");
            assert_eq!(window.notifications(cx).len(), 1);
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
            assert_eq!(window.notifications(cx).len(), 1);
            window.render_frame(cx);
        })
        .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let action = window.find("notice-action");
            assert!(
                action.visible() && action.bounds().size.width > gpui_kit::px(0.),
                "{action:?}"
            );
            assert!(
                action.bounds().origin.y >= gpui_kit::px(0.),
                "action is off-screen: {action:?}"
            );
            window.click("notice-action", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(recovered.get());
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.notifications(cx).is_empty());
        })
        .unwrap();
    }
}
