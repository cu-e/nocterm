//! A plugin's notices keep to their own class and never displace the application's.
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext, test::TestWindowExt as _};
use nocterm_plugin_api::capability::notifications::{
    NoticeTone, Own, OwnNotice, OwnNotices, Post, PostNotice,
};
use nocterm_plugin_model::PluginId;
use nocterm_plugin_ports::{Apply, Observe};

use super::PluginNotices;
use crate::notice::{count, error, error_action, run_action, tests::window};

fn plugin(name: &str) -> PluginId {
    PluginId::user("acme", name).expect("a valid plugin id")
}

fn post(
    handle: AnyWindowHandle,
    who: &PluginId,
    text: &str,
    key: Option<&str>,
    cx: &mut TestAppContext,
) {
    let request = Post {
        tone: NoticeTone::Info,
        text: text.to_owned(),
        notice_key: key.map(str::to_owned),
    };
    cx.update(|cx| {
        Apply::<PostNotice>::apply(&PluginNotices::new(handle), who, request, cx)
            .expect("the window is open");
    });
}

fn own(handle: AnyWindowHandle, who: &PluginId, cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        let OwnNotices(list) =
            Observe::<Own>::sample(&PluginNotices::new(handle), who, cx).expect("always answers");
        list.into_iter()
            .map(|OwnNotice { text, .. }| text)
            .collect()
    })
}

fn total(handle: AnyWindowHandle, cx: &mut TestAppContext) -> usize {
    cx.update_window(handle, |_, window, cx| count(window, cx))
        .expect("the window is open")
}

#[gpui_kit::test]
fn fifty_plugin_posts_never_evict_an_approval_request(cx: &mut TestAppContext) {
    let handle = window(cx);
    cx.update_window(handle, |_, window, cx| {
        error_action(
            window,
            cx,
            "agent-approval",
            "Approval needed",
            "Run a command?",
            "Review",
            |_, _| {},
        );
    })
    .unwrap();
    for number in 0..50 {
        let who = plugin(&format!("flood{}", number % 10));
        post(handle, &who, &format!("spam {number}"), None, cx);
    }
    assert_eq!(
        total(handle, cx),
        1 + 16,
        "the approval and the plugin quota"
    );
    cx.update_window(handle, |_, window, cx| {
        assert!(
            run_action(window, cx, "agent-approval"),
            "the approval is still waiting for the user"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_application_limit_never_evicts_a_plugin_notice(cx: &mut TestAppContext) {
    let handle = window(cx);
    let who = plugin("quiet");
    post(handle, &who, "still here", None, cx);
    cx.update_window(handle, |_, window, cx| {
        for number in 0..60 {
            let key: &'static str = Box::leak(format!("host-{number}").into_boxed_str());
            error(window, cx, key, "Failed", "details");
        }
    })
    .unwrap();
    assert_eq!(own(handle, &who, cx), ["still here"]);
    assert_eq!(
        total(handle, cx),
        51,
        "fifty of the application and one plugin notice"
    );
}

#[gpui_kit::test]
fn a_plugin_keeps_its_three_newest_notices(cx: &mut TestAppContext) {
    let handle = window(cx);
    let who = plugin("chatty");
    for number in 1..=5 {
        post(handle, &who, &format!("note {number}"), None, cx);
    }
    assert_eq!(own(handle, &who, cx), ["note 3", "note 4", "note 5"]);
}

#[gpui_kit::test]
fn all_plugins_together_keep_sixteen(cx: &mut TestAppContext) {
    let handle = window(cx);
    for name in ["aa", "bb", "cc", "dd", "ee", "ff"] {
        let who = plugin(name);
        for number in 1..=3 {
            post(handle, &who, &format!("{name} {number}"), None, cx);
        }
    }
    assert_eq!(total(handle, cx), 16);
    assert!(
        own(handle, &plugin("aa"), cx).len() < 3,
        "the oldest plugin notices went first"
    );
    assert_eq!(own(handle, &plugin("ff"), cx).len(), 3);
}

#[gpui_kit::test]
fn the_same_key_replaces_a_notice_and_a_plugin_sees_only_its_own(cx: &mut TestAppContext) {
    let handle = window(cx);
    let first = plugin("first");
    let second = plugin("second");
    post(handle, &first, "queued 1", Some("queue"), cx);
    post(handle, &second, "queued by another", Some("queue"), cx);
    post(handle, &first, "queued 2", Some("queue"), cx);
    assert_eq!(own(handle, &first, cx), ["queued 2"]);
    assert_eq!(own(handle, &second, cx), ["queued by another"]);
}

#[gpui_kit::test]
fn a_plugin_notice_does_not_take_the_line_from_an_action_of_the_application(
    cx: &mut TestAppContext,
) {
    let handle = window(cx);
    cx.update_window(handle, |_, window, cx| {
        error_action(
            window,
            cx,
            "agent-approval",
            "Approval needed",
            "Run a command?",
            "Review",
            |_, _| {},
        );
    })
    .unwrap();
    post(handle, &plugin("late"), "hello", None, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("notice-action").is_some(),
            "the action of the application stays on the line"
        );
        assert_eq!(count(window, cx), 2, "the plugin notice waits in the list");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_plugin_notice_takes_the_line_when_nothing_of_the_application_needs_it(
    cx: &mut TestAppContext,
) {
    let handle = window(cx);
    post(handle, &plugin("alone"), "hello", None, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("notice-dismiss").is_some());
    })
    .unwrap();
}
