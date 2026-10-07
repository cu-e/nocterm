use super::*;
use gpui_kit::px;
use nocterm_ai::thread::Entry;
use serde_json::{Value, json};

use crate::panel::{entries::tool_input::source, widgets::short_hover_label};

#[test]
fn tool_source_preserves_literals_and_uses_explicit_fields() {
    let literal = "python3 - <<'PY'\r\n\tprint('*literal*')\r\nPY\r\n";
    for input in [
        json!({"command":literal,"script":"other","code":"other"}),
        json!({"command":3,"script":literal,"code":"other"}),
        json!({"code":literal}),
    ] {
        let mut call = acp::ToolCall::new("call", "provider summary");
        call.raw_input = Some(input.clone());
        let selected = source(&call).unwrap();
        assert_eq!(selected.label, "Script");
        assert_eq!(selected.text, literal);
        assert_eq!(call.raw_input.as_ref(), Some(&input));
    }
    let mut call = acp::ToolCall::new("call", "summary");
    call.raw_input = Some(json!(literal));
    let selected = source(&call).unwrap();
    assert_eq!(selected.label, "Tool input");
    assert_eq!(selected.text, literal);
    for input in [
        json!({"arguments":{"command":literal}}),
        json!([1, "two"]),
        json!(false),
    ] {
        let mut call = acp::ToolCall::new("call", "summary\nwith lines");
        call.raw_input = Some(input.clone());
        let selected = source(&call).unwrap();
        assert_eq!(selected.label, "Tool input");
        assert_eq!(selected.text, serde_json::to_string_pretty(&input).unwrap());
    }
    for input in [None, Some(Value::Null)] {
        let mut call = acp::ToolCall::new("call", literal);
        call.raw_input = input.clone();
        let selected = source(&call).unwrap();
        assert_eq!(selected.label, "Tool details");
        assert_eq!(selected.text, literal);
        call.title = "ordinary title".into();
        assert!(source(&call).is_none());
    }
}

#[test]
fn hover_summary_is_short_and_unicode_safe() {
    assert_eq!(short_hover_label("short\n title"), "short title");
    let long = "界🚀".repeat(100);
    let shortened = short_hover_label(&long);
    assert_eq!(shortened.chars().count(), 161);
    assert_eq!(shortened, format!("{}…", "界🚀".repeat(80)));
    assert_eq!(short_hover_label(&"界".repeat(160)), "界".repeat(160));
}

fn settle(f: &Fixture, cx: &mut TestAppContext) {
    for _ in 0..3 {
        cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
}

fn tool_fixture(cx: &mut TestAppContext, title: &str, input: Option<Value>) -> Fixture {
    let f = fixture_with_width(cx, 18.);
    new_chat(&f, cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        let mut call = acp::ToolCall::new("script", title);
        call.status = acp::ToolCallStatus::Completed;
        call.raw_input = input;
        call.content = serde_json::from_value(json!([
            {"type":"content", "content":{"type":"text", "text":"original result"}}
        ]))
        .unwrap();
        thread.state.entries = vec![Entry::Tool(call)];
        thread.mark_dirty(0);
        cx.notify();
    });
    settle(&f, cx);
    f
}

#[gpui_kit::test]
fn click_reveals_literal_source_copy_and_collapse_without_changing_result(cx: &mut TestAppContext) {
    let script =
        "python3 - <<'PY'\r\n    print('literal *markdown* ![image](url)')\r\n\tprint(2)\r\nPY\r\n";
    let f = tool_fixture(cx, "Execute Python", Some(json!({"command":script})));
    cx.update_window(f.handle, |_, window, cx| {
        assert!(window.try_find(("tool-input", 0usize)).is_none());
        window.click(("tool-call", 0usize), cx);
    })
    .unwrap();
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        assert!(window.try_find(("tool-input", 0usize)).is_some());
        assert!(window.try_find(("tool-output", 0usize)).is_some());
        assert_eq!(
            window.find(("copy-tool-input", 0usize)).label(),
            Some("Copy tool input")
        );
        window.click(("copy-tool-input", 0usize), cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), script);
        let thread = f.panel.read(cx).current().unwrap();
        let Entry::Tool(call) = &thread.read(cx).state.entries[0] else {
            panic!("tool missing")
        };
        assert_eq!(call.raw_input, Some(json!({"command":script})));
        let result = serde_json::to_value(&call.content).unwrap();
        assert_eq!(result[0]["content"]["text"], "original result");
    })
    .unwrap();

    // ACP updates remeasure the expanded row and replace the Copy source.
    let updated = "#!/bin/sh\n\techo updated\n";
    let session = cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .read(cx)
            .session
            .clone()
            .unwrap()
    });
    f.events
        .try_send(AgentEvent::Session(acp::SessionNotification::new(
            session,
            acp::SessionUpdate::ToolCallUpdate(
                serde_json::from_value(json!({
                    "toolCallId":"script", "rawInput":{"script":updated}
                }))
                .unwrap(),
            ),
        )))
        .unwrap();
    cx.run_until_parked();
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click(("copy-tool-input", 0usize), cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), updated);
        assert!(window.try_find(("tool-output", 0usize)).is_some());
        window.click(("tool-call", 0usize), cx);
    })
    .unwrap();
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, _| {
        assert!(window.try_find(("tool-input", 0usize)).is_none());
        assert!(window.try_find(("tool-output", 0usize)).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn large_input_is_bounded_scrollable_and_copied_in_full(cx: &mut TestAppContext) {
    let script = format!(
        "{}\n{}\nfinal line\n",
        "long_line_".repeat(100),
        "\tprint('unchanged')\n".repeat(13000)
    );
    assert!(script.len() > 200 * 1024);
    let f = tool_fixture(cx, "Large input", Some(json!(script)));
    cx.update_window(f.handle, |_, window, cx| {
        window.click(("tool-call", 0usize), cx)
    })
    .unwrap();
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        let scroll = window.find(("tool-input-scroll", 0usize)).bounds();
        let code = window.find(("tool-input-code", 0usize)).bounds();
        let entry = window.find(("agent-entry", 0usize)).bounds();
        assert!(scroll.size.height <= px(240.));
        assert!(scroll.right() <= entry.right() + px(1.));
        assert!(code.size.height > scroll.size.height * 2.);
        assert!(
            code.size.width > scroll.size.width,
            "long code lines should scroll horizontally: {code:?}, {scroll:?}"
        );
        window.scroll(
            ("tool-input-scroll", 0usize),
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(-80.), px(0.))),
            cx,
        );
        window.scroll(
            ("tool-input-scroll", 0usize),
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-80.))),
            cx,
        );
        window.render_frame(cx);
        let moved = window.find(("tool-input-code", 0usize)).bounds();
        assert!(moved.origin.x < code.origin.x && moved.origin.y < code.origin.y);
        assert_eq!(window.find(("tool-input-scroll", 0usize)).bounds(), scroll);
        window.click(("copy-tool-input", 0usize), cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), script);
    })
    .unwrap();
}

#[gpui_kit::test]
fn actual_hover_tooltip_is_short_and_bounded(cx: &mut TestAppContext) {
    let f = tool_fixture(cx, &"very long tool title\n".repeat(400), None);
    cx.update_window(f.handle, |_, window, cx| {
        window.hover(("tool-call", 0usize), cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, _| {
        let tooltip = window.find("agent-activity-tooltip").bounds();
        assert!(tooltip.size.width <= px(280.));
        assert!(tooltip.size.height <= px(150.));
        assert!(tooltip.size.height > px(0.));
    })
    .unwrap();
}
