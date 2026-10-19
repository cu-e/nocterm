use super::tool_output::{call, exec_result, expanded_fixture, settle};
use super::*;
use crate::panel::entries::tool_input::source;
use nocterm_ai::model::{
    ids::ToolId,
    tools::{ToolChanges, ToolPatch, ToolStatus},
};
use nocterm_ai::tool_display;
use serde_json::json;

#[test]
fn only_explicit_supported_shell_command_flags_promote_the_script() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let script = "printf '%s\\n' 'literal'\r\n\tprintf '```'\n";
    for program in ["bash", "sh", "/bin/bash", "/usr/bin/sh"] {
        for flag in ["-c", "-lc"] {
            let call = call(
                &authority,
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
            &authority,
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
    let f = expanded_fixture(
        |authority| {
            call(
                authority,
                "exec_command",
                json!({"terminal_id":"t1", "program":"bash", "args":args,"stdin":"input\r\n"}),
                output,
                true,
            )
        },
        cx,
    );
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
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let call = call(
        &authority,
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
/// A chat whose first row is an expanded thought that is still streaming.
fn expanded_thought(f: &Fixture, cx: &mut TestAppContext) -> Entity<crate::thread::AgentThread> {
    new_chat(f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    deliver(f, &stream_session(&thread, cx), thought_chunk("short"), cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
    })
    .unwrap();
    thread
}
fn stream_session(
    thread: &Entity<crate::thread::AgentThread>,
    cx: &mut TestAppContext,
) -> SessionId {
    cx.update(|cx| thread.read(cx).session().clone().unwrap())
}
fn thought_chunk(text: &str) -> SessionUpdate {
    SessionUpdate::ThoughtChunk(Content::Text(Text {
        text: text.into(),
        annotations: None,
        extensions: None,
    }))
}
/// Adds a pending tool and thirty answered questions after the thought, then expands the tool.
fn add_tool_and_questions(
    f: &Fixture,
    thread: &Entity<crate::thread::AgentThread>,
    cx: &mut TestAppContext,
) {
    let session = stream_session(thread, cx);
    deliver(
        f,
        &session,
        SessionUpdate::ToolStarted(super::provider_fixtures::provider_tool(
            "call",
            "Tool",
            ToolStatus::Pending,
        )),
        cx,
    );
    for i in 0..30 {
        thread.update(cx, |thread, cx| thread.send(format!("question {i}"), cx));
        cx.run_until_parked();
        complete_active(f, cx);
    }
    thread.update(cx, |thread, cx| {
        for index in 0..thread.history_rows().len() {
            thread.mark_dirty(index);
        }
        cx.notify();
    });
    super::navigation::scroll_to(f, 0, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click(("tool-call", 1usize), cx);
        window.render_frame(cx);
    })
    .unwrap();
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
    let thread = expanded_thought(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click(("thought", 0usize), cx)
    })
    .unwrap();
    let before = cx
        .update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(("agent-entry", 0usize)).bounds().size.height
        })
        .unwrap();
    let session = stream_session(&thread, cx);
    deliver(
        &f,
        &session,
        thought_chunk(&"thought line\n\n".repeat(10)),
        cx,
    );
    add_tool_and_questions(&f, &thread, cx);
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
        report_control(
            &f,
            thread,
            SessionUpdate::ToolChanged(ToolPatch {
                id: ToolId::new("call"),
                changes: ToolChanges {
                    output: Some(vec![super::tool_output::text_output(
                        "tool line\n".repeat(12),
                    )]),
                    ..Default::default()
                },
                extensions: None,
                claims: Default::default(),
                literal: Default::default(),
            }),
            cx,
        );
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
