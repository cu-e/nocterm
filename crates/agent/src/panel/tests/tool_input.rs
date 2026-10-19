use super::*;
use gpui_kit::px;
use nocterm_ai::model::{
    ids::ToolId,
    json::OwnedJson,
    literal::{LiteralIdentity, TextRange},
    retention::RetentionAuthority,
    tools::{ToolChanges, ToolPatch, ToolRecord, ToolStatus},
};
use nocterm_ai::thread::Entry;
use serde_json::{Value, json};
fn wrapped_identity(call: &mut ToolRecord, owner: std::ops::Range<usize>) {
    let server = call.arguments.as_ref().unwrap()["server"].as_str().unwrap();
    let tool = call.arguments.as_ref().unwrap()["tool"].as_str().unwrap();
    call.literal.input = LiteralIdentity::Registered {
        owner: TextRange::supplied(server, &server[owner]).unwrap(),
        tool: TextRange::supplied(tool, tool).unwrap(),
    };
}

use crate::panel::entries::tool_input::source;
use nocterm_ai::tool_display::{self, ToolDisplay};

fn verified_call(authority: &RetentionAuthority, tool: &str, input: Value) -> ToolRecord {
    let request = tool_display::parse(tool, input.clone()).unwrap();
    let mut call = super::provider_intersections::provider_tool(
        "verified",
        format!("mcp.nocterm-3.{tool}"),
        ToolStatus::Pending,
    );
    let end = call.title.len();
    super::provider_intersections::known_title(&mut call, 12..13, 14..end);
    call.arguments = Some(
        OwnedJson::from_untracked(
            authority,
            json!({"server":"nocterm-3","tool":tool,"arguments":input}),
        )
        .unwrap(),
    );
    wrapped_identity(&mut call, 8..9);
    ToolDisplay::new(
        authority,
        &request,
        Some("Production · root@actual.example:2222".into()),
    )
    .unwrap()
    .attach(&mut call);
    call
}

#[test]
#[expect(
    clippy::cognitive_complexity,
    clippy::too_many_lines,
    reason = "predates the limit"
)]
fn verified_inputs_show_operations_with_literal_arguments_and_separate_options() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let command = r#"printf '%s\n' '```literal```' "$HOME""#;
    let call = verified_call(
        &authority,
        "run_command",
        json!({"terminal_id":"t1","command":command,"timeout_ms":10000,"idle_ms":500}),
    );
    let selected = source(&call).unwrap();
    assert_eq!(selected.label, "Command");
    assert_eq!(selected.text, command);
    assert_eq!(selected.language, Some("bash"));
    assert_eq!(selected.parameters, ["Timeout 10000 ms", "Idle 500 ms"]);
    let input = "\t*literal*\r\n```\r\n![not an image](url)\r\n";
    for enter in [false, true] {
        let call = verified_call(
            &authority,
            "send_input",
            json!({"terminal_id":"t1","text":input,"press_enter":enter}),
        );
        let selected = source(&call).unwrap();
        assert_eq!(selected.label, "Input");
        assert_eq!(selected.text, input);
        assert_eq!(
            selected.parameters,
            [if enter {
                "Enter after input"
            } else {
                "No Enter"
            }]
        );
    }
    let args = json!(["-c", "echo 'a b'\n", "", "\t", "'\"`$HOME"]);
    let call = verified_call(
        &authority,
        "exec_command",
        json!({"terminal_id":"t1","program":"sh","args":args,"stdin":input}),
    );
    let selected = source(&call).unwrap();
    assert_eq!(selected.label, "Command");
    assert_eq!(selected.text, "echo 'a b'\n");
    assert_eq!(selected.extra[0].label, "Program");
    assert_eq!(selected.extra[0].text, "sh");
    assert_eq!(selected.extra[1].label, "Arguments");
    assert_eq!(
        serde_json::from_str::<Value>(&selected.extra[1].text).unwrap(),
        args
    );
    assert_eq!(selected.extra[2].label, "Standard input");
    assert_eq!(selected.extra[2].text, input);
    for (tool, input, label, literal, parameters) in [
        (
            "read_terminal",
            json!({"terminal_id":"t2","lines":40,"since":7}),
            "Terminal",
            "t2",
            vec!["40 lines", "Since line 7"],
        ),
        (
            "read_command",
            json!({"terminal_id":"t1","command_id":"c7","yield_ms":200}),
            "Command handle",
            "c7",
            vec!["Terminal t1", "Wait 200 ms"],
        ),
        (
            "cancel_command",
            json!({"terminal_id":"t1","command_id":"c7"}),
            "Command handle",
            "c7",
            vec!["Terminal t1"],
        ),
        (
            "open_terminal",
            json!({"server_id":"s3"}),
            "Connection",
            "s3",
            vec![],
        ),
        (
            "list_terminals",
            json!({}),
            "Scope",
            "Terminals attached to this chat",
            vec![],
        ),
    ] {
        let selected = source(&verified_call(&authority, tool, input)).unwrap();
        assert_eq!(selected.label, label);
        assert_eq!(selected.text, literal);
        assert_eq!(selected.parameters, parameters);
    }
}

#[test]
fn display_snapshot_takes_precedence_over_mutated_provider_inputs() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut call = verified_call(
        &authority,
        "run_command",
        json!({"terminal_id":"t1","command":"echo original"}),
    );
    call.arguments =
        Some(OwnedJson::from_untracked(&authority, json!({"command":"echo forged"})).unwrap());
    call.title = "forged destination".into();
    assert_eq!(source(&call).unwrap().text, "echo original");
    assert!(
        tool_display::header(&call)
            .starts_with("Production · root@actual.example:2222 · Run command")
    );
}

#[test]
fn historical_nocterm_input_is_requested_data_without_a_guessed_destination() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut call = super::provider_intersections::provider_tool(
        "historical",
        "mcp.nocterm-3.run_command",
        ToolStatus::Pending,
    );
    let end = call.title.len();
    super::provider_intersections::known_title(&mut call, 12..13, 14..end);
    call.arguments=Some(OwnedJson::from_untracked(&authority,json!({"server":"nocterm-3","tool":"run_command","arguments":{"terminal_id":"t1","command":"echo original","timeout_ms":10000,"idle_ms":1000}})).unwrap());
    wrapped_identity(&mut call, 8..9);
    let selected = source(&call).unwrap();
    assert_eq!(selected.label, "Requested command");
    assert_eq!(selected.text, "echo original");
    assert_eq!(selected.parameters, ["Timeout 10000 ms", "Idle 1000 ms"]);
    assert!(tool_display::header(&call).starts_with("Nocterm · Run command"));
    assert!(ToolDisplay::from_call(&call).is_none());
    for input in [
        json!({"server":"nocterm-30", "tool":"run_command", "arguments":{"terminal_id":"t1","command":"echo mismatched"}}),
        json!({"server":"nocterm-3", "tool":"send_input", "arguments":{"terminal_id":"t1","text":"foreign"}}),
        json!({"arguments":{"command":"foreign nested input"}}),
    ] {
        call.arguments = Some(OwnedJson::from_untracked(&authority, input.clone()).unwrap());
        call.literal.input = LiteralIdentity::Invalid;
        if let Some(server) = input.get("server").and_then(Value::as_str) {
            wrapped_identity(&mut call, 8..server.len());
        }
        assert_eq!(source(&call).unwrap().label, "Tool input");
        assert_eq!(
            source(&call).unwrap().text,
            serde_json::to_string_pretty(&input).unwrap()
        );
    }
    call.title = "mcp.foreign.run_command".into();
    call.literal.title = LiteralIdentity::Invalid;
    call.arguments=Some(OwnedJson::from_untracked(&authority,json!({"server":"nocterm-3","tool":"run_command","arguments":{"terminal_id":"t1","command":"echo foreign"}})).unwrap());
    assert_eq!(source(&call).unwrap().label, "Tool input");
}

#[test]
fn tool_source_preserves_literals_and_uses_explicit_fields() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let literal = "python3 - <<'PY'\r\n\tprint('*literal*')\r\nPY\r\n";
    for input in [
        json!({"command":literal,"script":"other","code":"other"}),
        json!({"command":3,"script":literal,"code":"other"}),
        json!({"code":literal}),
    ] {
        let mut call = super::provider_intersections::provider_tool(
            "call",
            "provider summary",
            ToolStatus::Pending,
        );
        call.arguments = Some(OwnedJson::from_untracked(&authority, input.clone()).unwrap());
        let selected = source(&call).unwrap();
        assert_eq!(selected.label, "Script");
        assert_eq!(selected.text, literal);
        assert_eq!(
            call.arguments.as_ref().map(OwnedJson::as_value),
            Some(&input)
        );
    }
    let mut call =
        super::provider_intersections::provider_tool("call", "summary", ToolStatus::Pending);
    call.arguments = Some(OwnedJson::from_untracked(&authority, json!(literal)).unwrap());
    let selected = source(&call).unwrap();
    assert_eq!(selected.label, "Tool input");
    assert_eq!(selected.text, literal);
    for input in [
        json!({"arguments":{"command":literal}}),
        json!([1, "two"]),
        json!(false),
    ] {
        let mut call = super::provider_intersections::provider_tool(
            "call",
            "summary\nwith lines",
            ToolStatus::Pending,
        );
        call.arguments = Some(OwnedJson::from_untracked(&authority, input.clone()).unwrap());
        let selected = source(&call).unwrap();
        assert_eq!(selected.label, "Tool input");
        assert_eq!(selected.text, serde_json::to_string_pretty(&input).unwrap());
    }
    for input in [None, Some(Value::Null)] {
        let mut call =
            super::provider_intersections::provider_tool("call", literal, ToolStatus::Pending);
        call.arguments = input
            .clone()
            .map(|value| OwnedJson::from_untracked(&authority, value).unwrap());
        let selected = source(&call).unwrap();
        assert_eq!(selected.label, "Tool details");
        assert_eq!(selected.text, literal);
        call.title = "ordinary title".into();
        assert!(source(&call).is_none());
    }
}

fn settle(f: &Fixture, cx: &mut TestAppContext) {
    for _ in 0..3 {
        cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
}

fn tool_chat(cx: &mut TestAppContext) -> Fixture {
    let f = fixture_with_width(cx, 18., usage_source(), workspace_resolver(), image_deps());
    new_chat(&f, cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    f
}

fn tool_fixture(cx: &mut TestAppContext, title: &str, input: Option<Value>) -> Fixture {
    let f = tool_chat(cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        let mut call =
            super::provider_intersections::provider_tool("script", title, ToolStatus::Pending);
        call.status = ToolStatus::Succeeded;
        call.arguments =
            input.map(|value| OwnedJson::from_untracked(&f.installation.retention, value).unwrap());
        call.output = vec![super::tool_output::text_output("original result".into())];
        restore_entries(&f, thread, vec![Entry::Tool(call)], cx);
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
        let facts = thread.read(cx).tool_facts();
        assert_eq!(facts[0].arguments, Some(json!({"command":script})));
        thread.read(cx).with_tool(0, |call| {
            let nocterm_ai::history::physical::OutputSlot::Content { content, .. } =
                &call.output()[0]
            else {
                panic!("expected original result content")
            };
            let nocterm_ai::history::physical::ContentSlot::Text(text) = content.as_ref() else {
                panic!("expected original result text")
            };
            assert_eq!(&*text.text(), "original result");
        });
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
            .session()
            .clone()
            .unwrap()
    });
    deliver(
        &f,
        &session,
        SessionUpdate::ToolChanged(ToolPatch {
            id: ToolId::new("script"),
            changes: ToolChanges {
                arguments: Some(
                    OwnedJson::from_untracked(&f.installation.retention, json!({"script":updated}))
                        .unwrap(),
                ),
                ..Default::default()
            },
            extensions: None,
            claims: Default::default(),
            literal: Default::default(),
        }),
        cx,
    );
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

fn set_verified(f: &Fixture, mut call: ToolRecord, cx: &mut TestAppContext) {
    call.status = ToolStatus::Succeeded;
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        restore_entries(f, thread, vec![Entry::Tool(call)], cx);
        thread.mark_dirty(0);
        cx.notify();
    });
    settle(f, cx);
}

#[gpui_kit::test]
fn verified_server_and_command_remain_readable_in_both_themes(cx: &mut TestAppContext) {
    use nocterm_ai::history::SavedChat;
    let command = r#"printf '%s\n' '```literal```' "$HOME""#;
    let f = tool_chat(cx);
    let mut call = verified_call(
        &f.installation.retention,
        "run_command",
        json!({"terminal_id":"t1","command":command}),
    );
    // Restored entries carry their original destination with no attachment.
    let retention = &f.installation.retention;
    let saved = SavedChat::for_identity(
        retention,
        "fixture".into(),
        "00000000-0000-4000-8000-0000000000d2".into(),
        0,
    );
    let (id, metadata, resume, mut transcript, composer, _) = saved.into_parts();
    transcript.entries = vec![Entry::Tool(call)];
    let saved =
        SavedChat::from_parts(retention, id, metadata, resume, transcript, composer).unwrap();
    f.repository.seed_chat(&saved).unwrap();
    let restored = f.repository.transcript(saved.id()).unwrap();
    let Entry::Tool(restored) = &restored.transcript().entries[0] else {
        unreachable!()
    };
    call = restored.clone();
    set_verified(&f, call, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click(("tool-call", 0usize), cx)
    })
    .unwrap();
    for mode in [
        gpui_kit::component::ThemeMode::Light,
        gpui_kit::component::ThemeMode::Dark,
    ] {
        cx.update(|cx| gpui_kit::component::Theme::change(mode, None, cx));
        settle(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            assert_eq!(
                window.find(("tool-call", 0usize)).label(),
                Some("Production · root@actual.example:2222 · Run command · Completed")
            );
            assert_eq!(
                window.find(("copy-tool-input", 0usize)).label(),
                Some("Copy command")
            );
            window.click(("copy-tool-input", 0usize), cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), command);
            let entry = window.find(("agent-entry", 0usize)).bounds();
            let scroll = window.find(("tool-input-scroll", 0usize)).bounds();
            assert!(scroll.right() <= entry.right() + px(1.));
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn copying_a_redacted_snapshot_cannot_reveal_original_input(cx: &mut TestAppContext) {
    let f = tool_chat(cx);
    let original = "password=1 ".repeat(1400);
    let request = tool_display::parse(
        "run_command",
        json!({"terminal_id":"t1","command":original}),
    )
    .unwrap();
    let mut display = ToolDisplay::new(
        &f.installation.retention,
        &request,
        Some("Original server".into()),
    )
    .unwrap();
    display.redact().unwrap();
    let redacted = display.arguments["command"].as_str().unwrap().to_owned();
    let mut call = verified_call(
        &f.installation.retention,
        "run_command",
        json!({"terminal_id":"t1","command":original}),
    );
    display.attach(&mut call);
    set_verified(&f, call, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click(("tool-call", 0usize), cx)
    })
    .unwrap();
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        assert_eq!(
            window.find(("tool-call", 0usize)).label(),
            Some("Original server · Run command · Completed")
        );
        window.click(("copy-tool-input", 0usize), cx);
        let copied = cx.read_from_clipboard().unwrap().text().unwrap();
        assert_eq!(copied, redacted);
        assert!(!copied.contains("password=1"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn structured_arguments_and_stdin_are_independently_copyable(cx: &mut TestAppContext) {
    let args = json!(["-c", "printf '%s' 'a b'", "", "\t"]);
    let stdin = "\t*literal*\r\n```\r\n![not an image](url)\r\n";
    let f = tool_chat(cx);
    set_verified(
        &f,
        verified_call(
            &f.installation.retention,
            "exec_command",
            json!({"terminal_id":"t1","program":"sh","args":args,"stdin":stdin}),
        ),
        cx,
    );
    cx.update_window(f.handle, |_, window, cx| {
        window.click(("tool-call", 0usize), cx)
    })
    .unwrap();
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        assert_eq!(
            window.find(("copy-tool-input", 0usize)).label(),
            Some("Copy command")
        );
        assert_eq!(
            window.find(("copy-tool-input-extra-1", 0usize)).label(),
            Some("Copy arguments")
        );
        assert_eq!(
            window.find(("copy-tool-input-extra-2", 0usize)).label(),
            Some("Copy standard input")
        );
        window.click(("copy-tool-input-extra-1", 0usize), cx);
        assert_eq!(
            serde_json::from_str::<Value>(&cx.read_from_clipboard().unwrap().text().unwrap())
                .unwrap(),
            args
        );
        window.click(("copy-tool-input-extra-2", 0usize), cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), stdin);
    })
    .unwrap();
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn verified_destination_uses_the_connected_host_and_survives_rename_detach_and_reconnect(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let access = Rc::new(ConnectedAccess {
        base: f.access.clone(),
        target: std::cell::RefCell::new(nocterm_session::Target::new(
            "original",
            "original.example",
            2222,
        )),
    });
    let entry = nocterm_workspace::TerminalEntry {
        item: f.terminal,
        capabilities: TerminalCapabilities::new(access.clone())
            .with_text(access.clone())
            .with_input(access.clone())
            .with_prompts(access.clone()),
        title: "Production".into(),
        active: true,
        bottom: false,
        background: false,
    };
    let summary = nocterm_workspace::ConnectionSummary {
        id: "edited-profile".into(),
        name: "Edited profile".into(),
        group: None,
        description: "".into(),
        target: nocterm_session::Target::new("edited", "edited.example", 22),
        icon: None,
        flag: None,
    };
    let directory = Rc::new(Directory(std::cell::RefCell::new(vec![summary])));
    cx.update(|cx| {
        f.workspace.update(cx, |workspace, _| {
            workspace.set_connection_directory(directory.clone())
        })
    });
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let request = tool_display::parse(
        "run_command",
        json!({"terminal_id":"t1","command":"echo snapshot"}),
    )
    .unwrap();
    let server = cx.update(|cx| {
        format!(
            "nocterm-{}",
            thread.read(cx).registration().as_ref().unwrap().id
        )
    });
    let mut call = super::provider_intersections::provider_tool(
        "snapshot",
        format!("mcp.{server}.run_command"),
        ToolStatus::Pending,
    );
    let end = call.title.len();
    super::provider_intersections::known_title(
        &mut call,
        12..4 + server.len(),
        5 + server.len()..end,
    );
    call.arguments=Some(OwnedJson::from_untracked(&f.installation.retention,json!({"server":server,"tool":"run_command","arguments":{"terminal_id":"t1","command":"echo snapshot"}})).unwrap());
    wrapped_identity(&mut call, 8..server.len());
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            report_control(&f, thread, SessionUpdate::ToolStarted(call), cx);
            let (respond, _) = oneshot::channel();
            let mut request = BridgeCall {
                arguments: Some(json!({"terminal_id":"t1","command":"echo snapshot"})),
                display_token: None,
                registration_id: thread.registration().as_ref().unwrap().id,
                call: request.clone(),
                respond,
            };
            thread
                .record_tool_display(&mut request, Some(&entry), cx)
                .unwrap();
            thread.clear_composer_context(cx);
        })
    });
    directory.0.borrow_mut()[0].name = "Renamed".into();
    *access.target.borrow_mut() = nocterm_session::Target::new("reconnected", "new.example", 22);
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            report_control(
                &f,
                thread,
                SessionUpdate::ToolChanged(ToolPatch {
                    id: ToolId::new("snapshot"),
                    changes: ToolChanges {
                        title: Some("Renamed".into()),
                        status: Some(ToolStatus::Succeeded),
                        ..Default::default()
                    },
                    extensions: None,
                    claims: Default::default(),
                    literal: Default::default(),
                }),
                cx,
            );
            let call = thread.tool_slot(0);
            assert_eq!(
                tool_display::managed_header(call),
                "Production · original@original.example:2222 · Run command · Completed"
            );
            assert!(!tool_display::managed_header(call).contains("edited.example"));
            assert!(!tool_display::managed_header(call).contains("new.example"));
        })
    });
    *access.target.borrow_mut() = nocterm_session::Target::new("root", "::1", 2222);
    let ipv6 = tool_display::parse(
        "run_command",
        json!({"terminal_id":"t1","command":"echo ipv6"}),
    )
    .unwrap();
    let mut call = super::provider_intersections::provider_tool(
        "ipv6",
        format!("mcp.{server}.run_command"),
        ToolStatus::Pending,
    );
    let end = call.title.len();
    super::provider_intersections::known_title(
        &mut call,
        12..4 + server.len(),
        5 + server.len()..end,
    );
    call.arguments=Some(OwnedJson::from_untracked(&f.installation.retention,json!({"server":server,"tool":"run_command","arguments":{"terminal_id":"t1","command":"echo ipv6"}})).unwrap());
    wrapped_identity(&mut call, 8..server.len());
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            report_control(&f, thread, SessionUpdate::ToolStarted(call), cx);
            let (respond, _) = oneshot::channel();
            let mut request = BridgeCall {
                arguments: Some(json!({"terminal_id":"t1","command":"echo ipv6"})),
                display_token: None,
                registration_id: thread.registration().as_ref().unwrap().id,
                call: ipv6.clone(),
                respond,
            };
            thread
                .record_tool_display(&mut request, Some(&entry), cx)
                .unwrap();
            let call = thread.tool_slot(1);
            assert!(
                tool_display::managed_header(call)
                    .starts_with("Production · root@[::1]:2222 · Run command")
            );
        })
    });
}
