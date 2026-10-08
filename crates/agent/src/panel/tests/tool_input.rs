use super::*;
use gpui_kit::px;
use nocterm_ai::thread::Entry;
use serde_json::{Value, json};

use crate::panel::{entries::tool_input::source, widgets::short_hover_label};
use nocterm_ai::tool_display::{self, ToolDisplay};

fn verified_call(tool: &str, input: Value) -> acp::ToolCall {
    let request = tool_display::parse(tool, input.clone()).unwrap();
    let mut call = acp::ToolCall::new("verified", format!("mcp.nocterm-3.{tool}"));
    call.raw_input = Some(json!({"server":"nocterm-3","tool":tool,"arguments":input}));
    ToolDisplay::new(
        &request,
        Some("Production · root@actual.example:2222".into()),
    )
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
    let command = r#"printf '%s\n' '```literal```' "$HOME""#;
    let call = verified_call(
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
        let selected = source(&verified_call(tool, input)).unwrap();
        assert_eq!(selected.label, label);
        assert_eq!(selected.text, literal);
        assert_eq!(selected.parameters, parameters);
    }
}

#[test]
fn display_snapshot_takes_precedence_over_mutated_provider_inputs() {
    let mut call = verified_call(
        "run_command",
        json!({"terminal_id":"t1","command":"echo original"}),
    );
    call.raw_input = Some(json!({"command":"echo forged"}));
    call.title = "forged destination".into();
    assert_eq!(source(&call).unwrap().text, "echo original");
    assert!(
        tool_display::header(&call)
            .starts_with("Production · root@actual.example:2222 · Run command")
    );
}

#[test]
fn historical_nocterm_input_is_requested_data_without_a_guessed_destination() {
    let mut call = acp::ToolCall::new("historical", "mcp.nocterm-3.run_command").raw_input(json!({
        "server":"nocterm-3", "tool":"run_command", "arguments":{"terminal_id":"t1", "command":"echo original", "timeout_ms":10000,"idle_ms":1000}
    }));
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
        call.raw_input = Some(input.clone());
        assert_eq!(source(&call).unwrap().label, "Tool input");
        assert_eq!(
            source(&call).unwrap().text,
            serde_json::to_string_pretty(&input).unwrap()
        );
    }
    call.title = "mcp.foreign.run_command".into();
    call.raw_input = Some(
        json!({"server":"nocterm-3", "tool":"run_command", "arguments":{"terminal_id":"t1", "command":"echo foreign"}}),
    );
    assert_eq!(source(&call).unwrap().label, "Tool input");
}

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
            .session()
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

fn set_verified(f: &Fixture, mut call: acp::ToolCall, cx: &mut TestAppContext) {
    call.status = acp::ToolCallStatus::Completed;
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.state.entries = vec![Entry::Tool(call)];
        thread.mark_dirty(0);
        cx.notify();
    });
    settle(f, cx);
}

#[gpui_kit::test]
fn verified_server_and_command_remain_readable_in_both_themes(cx: &mut TestAppContext) {
    let command = r#"printf '%s\n' '```literal```' "$HOME""#;
    let f = tool_fixture(cx, "placeholder", None);
    let mut call = verified_call("run_command", json!({"terminal_id":"t1","command":command}));
    // Restored entries carry their original destination with no attachment.
    call = serde_json::from_value(serde_json::to_value(call).unwrap()).unwrap();
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
    let original = "password=1 ".repeat(1400);
    let request = tool_display::parse(
        "run_command",
        json!({"terminal_id":"t1","command":original}),
    )
    .unwrap();
    let mut display = ToolDisplay::new(&request, Some("Original server".into()));
    display.redact();
    let redacted = display.arguments["command"].as_str().unwrap().to_owned();
    let mut call = verified_call(
        "run_command",
        json!({"terminal_id":"t1","command":original}),
    );
    display.attach(&mut call);
    let f = tool_fixture(cx, "placeholder", None);
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
    let f = tool_fixture(cx, "placeholder", None);
    set_verified(
        &f,
        verified_call(
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

struct ConnectedAccess {
    base: Rc<Access>,
    target: std::cell::RefCell<nocterm_session::Target>,
}
impl TerminalAccess for ConnectedAccess {
    fn info(&self, cx: &App) -> Option<TerminalInfo> {
        let mut info = self.base.info(cx)?;
        info.local = false;
        info.profile = Some("edited-profile".into());
        info.target = Some(self.target.borrow().clone());
        Some(info)
    }
    fn read(&self, request: TextRequest, cx: &App) -> Result<TerminalText, String> {
        self.base.read(request, cx)
    }
    fn send_text(&self, text: &str, cx: &mut App) -> Result<(), String> {
        self.base.send_text(text, cx)
    }
    fn run_command(&self, text: &str, cx: &mut App) -> Result<(), String> {
        self.base.run_command(text, cx)
    }
    fn answer_sign_in(&self, answer: String, cx: &mut App) -> Result<(), String> {
        self.base.answer_sign_in(answer, cx)
    }
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
        access: access.clone(),
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
    let mut call = acp::ToolCall::new("snapshot", format!("mcp.{server}.run_command"));
    call.raw_input = Some(
        json!({"server":server,"tool":"run_command","arguments":{"terminal_id":"t1","command":"echo snapshot"}}),
    );
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.apply_presented_update(acp::SessionUpdate::ToolCall(call));
            thread.record_tool_display(&request, Some(&entry), cx);
            thread.attachments.clear();
        })
    });
    directory.0.borrow_mut()[0].name = "Renamed".into();
    *access.target.borrow_mut() = nocterm_session::Target::new("reconnected", "new.example", 22);
    cx.update(|cx| {
        thread.update(cx, |thread, _| {
            thread.apply_presented_update(acp::SessionUpdate::ToolCallUpdate(
                acp::ToolCallUpdate::new(
                    "snapshot",
                    acp::ToolCallUpdateFields::new()
                        .title("Renamed")
                        .status(acp::ToolCallStatus::Completed),
                ),
            ));
            let Entry::Tool(call) = &thread.state.entries[0] else {
                unreachable!()
            };
            assert_eq!(
                tool_display::header(call),
                "Production · original@original.example:2222 · Run command · Completed"
            );
            assert!(!tool_display::header(call).contains("edited.example"));
            assert!(!tool_display::header(call).contains("new.example"));
        })
    });
    *access.target.borrow_mut() = nocterm_session::Target::new("root", "::1", 2222);
    let ipv6 = tool_display::parse(
        "run_command",
        json!({"terminal_id":"t1","command":"echo ipv6"}),
    )
    .unwrap();
    let mut call = acp::ToolCall::new("ipv6", format!("mcp.{server}.run_command"));
    call.raw_input = Some(
        json!({"server":server,"tool":"run_command","arguments":{"terminal_id":"t1","command":"echo ipv6"}}),
    );
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.apply_presented_update(acp::SessionUpdate::ToolCall(call));
            thread.record_tool_display(&ipv6, Some(&entry), cx);
            let Entry::Tool(call) = &thread.state.entries[1] else {
                unreachable!()
            };
            assert!(
                tool_display::header(call)
                    .starts_with("Production · root@[::1]:2222 · Run command")
            );
        })
    });
}
