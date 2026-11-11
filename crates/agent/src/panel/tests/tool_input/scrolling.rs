//! Native wheel behavior of literal tool sections nested in a chat.
use super::*;
use gpui_kit::InputEvent as _;

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

fn nested_tool(cx: &mut TestAppContext, lines: usize) -> Fixture {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        let mut call = ToolRecord::new("script", "Scrollable command");
        call.status = ToolStatus::Succeeded;
        call.raw_input = Some(json!(format!("{}\n", "echo line ".repeat(20)).repeat(lines)));
        call.content = serde_json::from_value(json!([
            {"type":"content", "content":{"type":"text", "text":format!("{}\n", "result line ".repeat(20)).repeat(lines)}}
        ])).unwrap();
        restore_entries(&f, thread, vec![
            Entry::User(vec![Content::text("Earlier question.\n\n".repeat(60))]),
            Entry::Tool(call),
            Entry::Agent("Later answer.\n\n".repeat(60)),
        ], cx);
        for row in 0..3 { thread.mark_dirty(row); }
        cx.notify();
    });
    settle(&f, cx);
    super::super::navigation::scroll_to(&f, 1, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click(("tool-call", 1usize), cx)
    })
    .unwrap();
    settle(&f, cx);
    super::super::navigation::scroll_to(&f, 1, cx);
    f
}

fn section_id(namespace: &str, suffix: &str) -> (gpui_kit::SharedString, usize) {
    (format!("{namespace}-{suffix}").into(), 1)
}

#[gpui_kit::test]
fn wheel_over_tool_text_moves_only_the_text_in_both_directions(cx: &mut TestAppContext) {
    let f = nested_tool(cx, 100);
    for namespace in ["tool-input", "tool-output"] {
        assert_nested_wheel(&f, namespace, cx);
    }
}

fn assert_nested_wheel(f: &Fixture, namespace: &str, cx: &mut TestAppContext) {
    use gpui_kit::{ScrollDelta, ScrollWheelEvent, point};
    cx.update_window(f.handle, |_, window, cx| {
        let viewport_id = section_id(namespace, "scroll");
        let code_id = section_id(namespace, "code");
        window.hover(viewport_id.clone(), cx);
        let before = window.find(code_id.clone()).bounds();
        let viewport = window.find(viewport_id.clone()).bounds();
        let entry = window.find(("agent-entry", 1usize)).bounds();
        assert!(before.size.height > viewport.size.height * 2.);
        // Dispatch a burst without rendering between events.
        for _ in 0..3 {
            window.dispatch_event(
                ScrollWheelEvent {
                    position: viewport.center(),
                    delta: ScrollDelta::Pixels(point(px(0.), px(-20.))),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        }
        window.render_frame(cx);
        assert_eq!(
            window.find(("agent-entry", 1usize)).bounds(),
            entry,
            "wheel over {namespace} must not move the transcript"
        );
        assert_eq!(
            window.find(code_id.clone()).bounds().top(),
            before.top() - px(60.)
        );
        window.scroll(
            viewport_id.clone(),
            ScrollDelta::Pixels(point(px(0.), px(40.))),
            cx,
        );
        assert_eq!(window.find(("agent-entry", 1usize)).bounds(), entry);
        assert_eq!(
            window.find(code_id.clone()).bounds().top(),
            before.top() - px(20.)
        );
        assert!(before.size.width > viewport.size.width);
        window.scroll(
            viewport_id,
            ScrollDelta::Pixels(point(px(-40.), px(0.))),
            cx,
        );
        assert_eq!(window.find(("agent-entry", 1usize)).bounds(), entry);
        assert_eq!(
            window.find(code_id).bounds().left(),
            before.left() - px(40.)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn tool_text_chains_to_the_chat_only_after_reaching_either_edge(cx: &mut TestAppContext) {
    use gpui_kit::{ScrollDelta, point};
    for namespace in ["tool-input", "tool-output"] {
        for direction in [-1., 1.] {
            let f = nested_tool(cx, 100);
            cx.update_window(f.handle, |_, window, cx| {
                let viewport_id = section_id(namespace, "scroll");
                let entry = window.find(("agent-entry", 1usize)).bounds();
                window.scroll(
                    viewport_id.clone(),
                    ScrollDelta::Pixels(point(px(0.), px(-60.))),
                    cx,
                );
                window.scroll(
                    viewport_id.clone(),
                    ScrollDelta::Pixels(point(px(0.), px(direction * 1_000_000.))),
                    cx,
                );
                assert_eq!(
                    window.find(("agent-entry", 1usize)).bounds(),
                    entry,
                    "reaching an inner edge must not also move the chat"
                );
                window.scroll(
                    viewport_id,
                    ScrollDelta::Pixels(point(px(0.), px(direction * 24.))),
                    cx,
                );
                let after = window.find(("agent-entry", 1usize)).bounds();
                assert_eq!(after.top(), entry.top() + px(direction * 24.));
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn fitting_tool_text_passes_vertical_wheel_to_the_chat(cx: &mut TestAppContext) {
    use gpui_kit::{ScrollDelta, point};
    for namespace in ["tool-input", "tool-output"] {
        let f = nested_tool(cx, 1);
        cx.update_window(f.handle, |_, window, cx| {
            let entry = window.find(("agent-entry", 1usize)).bounds();
            window.scroll(
                section_id(namespace, "scroll"),
                ScrollDelta::Pixels(point(px(0.), px(-24.))),
                cx,
            );
            assert_eq!(
                window.find(("agent-entry", 1usize)).bounds().top(),
                entry.top() - px(24.)
            );
        })
        .unwrap();
    }
}
