use super::tool_output::{call, exec_result, expanded_fixture, settle};
use super::*;
use crate::panel::entries::tool_input::source;
use nocterm_ai::thread::Entry;
use nocterm_ai::tool_display;
use serde_json::json;

#[test]
fn only_explicit_supported_shell_command_flags_promote_the_script() {
    let script = "printf '%s\\n' 'literal'\r\n\tprintf '```'\n";
    for program in ["bash", "sh", "/bin/bash", "/usr/bin/sh"] {
        for flag in ["-c", "-lc"] {
            let call = call(
                "exec_command",
                json!({"terminal_id":"t1", "program":program, "args":[flag,script,"", "a b"], "stdin":"\tinput\r\n"}),
                exec_result("running"),
                true,
            );
            let selected = source(&call).unwrap();
            assert_eq!(selected.label, "Command");
            assert_eq!(selected.text, script);
            assert_eq!(selected.language, Some("bash"));
            assert_eq!(selected.extra[0].text, program);
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&selected.extra[1].text).unwrap(),
                json!([flag, script, "", "a b"])
            );
            assert_eq!(selected.extra[2].text, "\tinput\r\n");
        }
    }
    for (program, args) in [
        ("bash", json!(["script.sh"])),
        ("bash", json!(["-c"])),
        ("bash", json!(["-x", "-c", script])),
        ("python3", json!(["-c", script])),
        ("env", json!(["bash", "-lc", script])),
    ] {
        let call = call(
            "exec_command",
            json!({"terminal_id":"t1", "program":program, "args":args}),
            exec_result("running"),
            true,
        );
        let selected = source(&call).unwrap();
        assert_eq!(selected.label, "Program");
        assert_eq!(selected.text, program);
        assert_eq!(selected.extra[0].label, "Arguments");
    }
}

#[gpui_kit::test]
fn screenshot_doubleunderscore_shell_command_copies_literal_script_and_full_argv(
    cx: &mut TestAppContext,
) {
    let script = "printf '%s\\n' '*literal*' \"$HOME\"\r\n\tprintf '```'\n";
    let args = json!(["-lc", script, "", "a b"]);
    let payload = exec_result("running");
    let output = json!({"content":[{"type":"text","text":payload.to_string()}]});
    let call = call(
        "exec_command",
        json!({"terminal_id":"t1", "program":"bash", "args":args,"stdin":"input\r\n"}),
        output,
        true,
    );
    // A restored read-only history keeps the frozen bridge destination.
    let call = serde_json::from_value(serde_json::to_value(call).unwrap()).unwrap();
    let f = expanded_fixture(call, cx);
    for mode in [
        gpui_kit::component::ThemeMode::Dark,
        gpui_kit::component::ThemeMode::Light,
    ] {
        cx.update(|cx| gpui_kit::component::Theme::change(mode, None, cx));
        settle(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            assert_eq!(
                window.find(("tool-call", 0usize)).label(),
                Some("Production · root@actual.example:22 · Execute program · Completed")
            );
            assert_eq!(
                window.find(("copy-tool-input", 0usize)).label(),
                Some("Copy command")
            );
            window.click(("copy-tool-input", 0usize), cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), script);
            window.click(("copy-tool-input-extra-0", 0usize), cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "bash");
            window.click(("copy-tool-input-extra-1", 0usize), cx);
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(
                    &cx.read_from_clipboard().unwrap().text().unwrap()
                )
                .unwrap(),
                args
            );
            window.click(("copy-tool-output", 0usize), cx);
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                payload["stdout"].as_str().unwrap()
            );
        })
        .unwrap();
    }
}

#[test]
fn unmatched_historical_command_is_readable_without_current_attachment_host() {
    let call = call(
        "exec_command",
        json!({"terminal_id":"t1", "program":"sh", "args":["-c","echo history"]}),
        exec_result("running"),
        false,
    );
    let selected = source(&call).unwrap();
    assert_eq!(selected.label, "Requested command");
    assert_eq!(selected.text, "echo history");
    assert_eq!(
        tool_display::header(&call),
        "Nocterm · Execute program · Completed"
    );
}

fn expanded_thought_and_tool(
    f: &Fixture,
    cx: &mut TestAppContext,
) -> Entity<crate::thread::AgentThread> {
    new_chat(f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        let mut entries = vec![
            Entry::Thought("short".into()),
            Entry::Tool(acp::ToolCall::new("call", "Tool")),
        ];
        entries.extend((0..30).map(|i| {
            Entry::User(vec![acp::ContentBlock::Text(acp::TextContent::new(
                format!("question {i}"),
            ))])
        }));
        restore_entries(f, thread, entries, cx);
        for index in 0..thread.transcript().entries.len() {
            thread.mark_dirty(index);
        }
        cx.notify();
    });
    super::navigation::scroll_to(f, 0, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click(("thought", 0usize), cx);
        window.render_frame(cx);
        window.click(("tool-call", 1usize), cx);
        window.render_frame(cx);
    })
    .unwrap();
    thread
}

fn assert_static_row(f: &Fixture, index: usize, cx: &mut TestAppContext) {
    let before = cx
        .update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(("agent-entry", index)).bounds()
        })
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(("agent-entry", index)).bounds(), before);
    })
    .unwrap();
}

#[gpui_kit::test]
fn dirty_expanded_thought_and_offscreen_tool_remeasure_without_reveal_ticks(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let thread = expanded_thought_and_tool(&f, cx);
    let before = cx
        .update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(("agent-entry", 0usize)).bounds().size.height
        })
        .unwrap();
    thread.update(cx, |thread, cx| {
        let mut entries = thread.history_entries().to_vec();
        entries[0] = Entry::Thought("thought line\n\n".repeat(10));
        restore_entries(&f, thread, entries, cx);
        thread.mark_dirty(0);
        cx.notify();
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thought = window.find(("agent-entry", 0usize)).bounds();
        assert!(thought.size.height > before);
        assert!(window.find(("agent-entry", 1usize)).bounds().origin.y >= thought.bottom());
    })
    .unwrap();
    assert_static_row(&f, 0, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click(
            (
                gpui_kit::ElementId::from("agent-transcript"),
                "jump-to-latest",
            ),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, _| {
        assert!(window.try_find(("tool-output", 1usize)).is_none());
    })
    .unwrap();
    thread.update(cx, |thread, cx| {
        report_control(thread, acp::SessionUpdate::ToolCallUpdate(serde_json::from_value(serde_json::json!({
            "toolCallId":"call", "content":[{"type":"content", "content":{"type":"text", "text":"tool line\n".repeat(12)}}]
        })).unwrap()), cx);
        thread.mark_dirty(1);
        cx.notify();
    });
    super::navigation::scroll_to(&f, 1, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let tool = window.find(("agent-entry", 1usize)).bounds();
        assert!(tool.size.height > gpui_kit::px(160.));
        assert!(window.find(("agent-entry", 2usize)).bounds().origin.y >= tool.bottom());
    })
    .unwrap();
    assert_static_row(&f, 1, cx);
}
