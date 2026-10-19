use super::*;
use crate::panel::entries::tool_output::source;
use gpui_kit::px;
use nocterm_ai::model::{
    ids::ToolId,
    json::OwnedJson,
    retention::RetentionAuthority,
    tools::{ToolChanges, ToolOutput, ToolPatch, ToolRecord, ToolStatus},
};
use nocterm_ai::{
    thread::Entry,
    tool_display::{self, ToolDisplay},
};
use serde_json::{Value, json};
pub(super) fn text_output(text: String) -> ToolOutput {
    ToolOutput::Content {
        content: Box::new(Content::Text(Text {
            text,
            annotations: None,
            extensions: None,
        })),
        extensions: None,
    }
}

pub(super) fn exec_result(state: &str) -> Value {
    json!({"command_id":"c7", "state":state, "stdout":"line\r\n\t```literal```\n界\n", "stderr":"warning\n",
        "exit_status":null, "total_bytes":27, "truncated":false, "error":null})
}

pub(super) fn call(
    authority: &nocterm_ai::model::retention::RetentionAuthority,
    tool: &str,
    args: Value,
    output: Value,
    verified: bool,
) -> nocterm_ai::model::tools::ToolRecord {
    use nocterm_ai::model::{
        json::OwnedJson,
        literal::{LiteralIdentity, TextRange},
        tools::ToolStatus,
    };
    let request = tool_display::parse(tool, args.clone()).unwrap();
    let mut call = super::provider_intersections::provider_tool(
        "screenshot",
        format!("mcp__nocterm-17__{tool}"),
        ToolStatus::Succeeded,
    );
    call.literal.title = LiteralIdentity::Registered {
        owner: TextRange::supplied(&call.title, &call.title[13..15]).unwrap(),
        tool: TextRange::supplied(&call.title, &call.title[17..]).unwrap(),
    };
    call.arguments = Some(OwnedJson::from_untracked(authority, args).unwrap());
    call.result = Some(OwnedJson::from_untracked(authority, output).unwrap());
    if verified {
        let mut display = ToolDisplay::new(
            authority,
            &request,
            Some("Production · root@actual.example:22".into()),
        )
        .unwrap();
        display.version = 1;
        display.outcome = nocterm_ai::model::controls::Presence::Absent;
        display.attach(&mut call);
    }
    call
}

pub(super) fn settle(f: &Fixture, cx: &mut TestAppContext) {
    for _ in 0..3 {
        cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
}

pub(super) fn set_call(f: &Fixture, call: ToolRecord, cx: &mut TestAppContext) {
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        restore_entries(f, thread, vec![Entry::Tool(call)], cx);
        thread.mark_dirty(0);
        cx.notify();
    });
    settle(f, cx);
}

pub(super) fn expanded_fixture(
    build: impl FnOnce(&RetentionAuthority) -> ToolRecord,
    cx: &mut TestAppContext,
) -> Fixture {
    let f = fixture_with_width(cx, 18., usage_source(), workspace_resolver(), image_deps());
    new_chat(&f, cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let call = build(&f.installation.retention);
    set_call(&f, call, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click(("tool-call", 0usize), cx)
    })
    .unwrap();
    settle(&f, cx);
    f
}

#[test]
fn known_result_decodes_direct_mcp_text_and_escaped_raw_output() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let payload = exec_result("running");
    let mcp = json!({"content":[{"type":"text","text":payload.to_string()}]});
    for output in [
        payload.clone(),
        json!(payload.to_string()),
        mcp.clone(),
        json!(mcp.to_string()),
        json!({"jsonrpc":"2.0","id":1,"result":mcp}),
        json!(json!({"jsonrpc":"2.0","id":1,"result":mcp}).to_string()),
    ] {
        let call = call(
            &authority,
            "exec_command",
            json!({"terminal_id":"t1","program":"bash","args":["-lc","echo hello"]}),
            output,
            true,
        );
        let decoded = source(&call, false).unwrap();
        assert_eq!(decoded.sections[0].label, "Standard output");
        assert_eq!(decoded.sections[0].text, payload["stdout"]);
        assert_eq!(decoded.sections[1].text, payload["stderr"]);
        assert!(decoded.parameters.contains(&"Process: Running".into()));
        assert!(decoded.parameters.contains(&"Exit status: unknown".into()));
        assert!(decoded.parameters.contains(&"Command handle: c7".into()));
        assert_eq!(call.status, ToolStatus::Succeeded);
    }
}

#[test]
fn process_states_exit_failure_and_truncation_remain_explicit() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    for (state, label) in [
        ("starting", "Starting"),
        ("running", "Running"),
        ("exited", "Exited"),
        ("timed_out", "Timed out"),
        ("cancelled", "Cancelled"),
        ("failed", "Failed"),
    ] {
        let mut payload = exec_result(state);
        payload["stdout"] = json!("");
        payload["stderr"] = json!("");
        payload["truncated"] = json!(true);
        payload["exit_status"] = json!(7);
        payload["error"] = json!("transport error\n");
        let call = call(
            &authority,
            "read_command",
            json!({"terminal_id":"t1","command_id":"c7"}),
            payload,
            false,
        );
        let output = source(&call, false).unwrap();
        assert_eq!(output.sections[0].text, "");
        assert!(output.parameters.contains(&format!("Process: {label}")));
        assert!(output.parameters.contains(&"Exit status: 7".into()));
        assert!(output.parameters.contains(&"Output truncated".into()));
        assert_eq!(output.sections[2].label, "Error");
    }
}

#[test]
fn live_observation_does_not_invent_an_exit_from_acp_completion_or_idle() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    for (reason, completed) in [
        ("prompt_returned", true),
        ("output_idle", false),
        ("timeout", false),
    ] {
        let payload = json!({"text":"literal output\n", "first_line":4, "next_line":8, "cursor_semantics":"inclusive snapshot; replace overlapping lines", "truncated":false, "alt_screen":false, "completion":reason, "completed":completed, "exit_status":null});
        let call = call(
            &authority,
            "run_command",
            json!({"terminal_id":"t1","command":"echo hello"}),
            payload,
            false,
        );
        let output = source(&call, false).unwrap();
        assert_eq!(output.sections[0].label, "Terminal output");
        assert!(output.parameters.contains(&"Exit status: unknown".into()));
        if !completed {
            assert!(
                output
                    .parameters
                    .iter()
                    .any(|parameter| parameter.contains("process completion unknown"))
            );
        }
        assert!(
            !output
                .parameters
                .iter()
                .any(|parameter| parameter.contains("Exited"))
        );
    }
}

#[test]
fn future_fields_malformed_payloads_and_mixed_blocks_remain_readable() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let args = json!({"terminal_id":"t1","program":"true"});
    let mut future = exec_result("exited");
    future["future_field"] = json!("keep me");
    let output = source(
        &call(&authority, "exec_command", args.clone(), future, false),
        false,
    )
    .unwrap();
    assert_eq!(output.sections[0].label, "Standard output");
    assert!(output.parameters.contains(&"future_field: keep me".into()));
    let mixed = json!({"content":[{"type":"text","text":exec_result("running").to_string()},{"type":"image","data":"AA==","mimeType":"image/png"}]});
    let output = source(
        &call(&authority, "exec_command", args.clone(), mixed, false),
        false,
    )
    .unwrap();
    assert_eq!(output.sections[0].label, "Standard output");
    assert!(
        output
            .sections
            .iter()
            .any(|section| section.text.contains("AA=="))
    );
    for (payload, expected) in [
        (json!({"stdout":3}), "stdout: 3"),
        (json!({"arbitrary":"keep me"}), "arbitrary: keep me"),
    ] {
        assert_eq!(
            source(
                &call(&authority, "exec_command", args.clone(), payload, false),
                false
            )
            .unwrap()
            .sections[0]
                .text,
            expected
        );
    }
    let mut foreign = call(
        &authority,
        "exec_command",
        args,
        exec_result("running"),
        false,
    );
    foreign.title = "mcp__foreign__exec_command".into();
    foreign.literal.title = nocterm_ai::model::literal::LiteralIdentity::Invalid;
    assert_eq!(
        source(&foreign, false).unwrap().sections[0].label,
        "Tool output"
    );
    assert!(
        source(&foreign, false).unwrap().sections[0]
            .text
            .starts_with('{')
    );
}

#[test]
fn duplicate_raw_result_does_not_hide_additional_content() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let payload = exec_result("running");
    let mut call = call(
        &authority,
        "exec_command",
        json!({"terminal_id":"t1","program":"true"}),
        payload.clone(),
        false,
    );
    call.output = vec![text_output(payload.to_string())];
    assert_eq!(source(&call, false).unwrap().sections.len(), 2);
    call.result = Some(
        OwnedJson::from_untracked(
            &authority,
            nocterm_ai::mcp::tool_result(json!(1), Ok(payload.clone()))["result"].clone(),
        )
        .unwrap(),
    );
    assert_eq!(source(&call, false).unwrap().sections.len(), 2);
    let mut result = call.result.as_ref().unwrap().as_value().clone();
    result["unknown"] = json!("provider metadata");
    call.result = Some(OwnedJson::from_untracked(&authority, result).unwrap());
    assert_eq!(
        source(&call, false).unwrap().sections.len(),
        2,
        "unknown envelope metadata does not duplicate a payload"
    );
    call.result = Some(OwnedJson::from_untracked(&authority, payload).unwrap());

    call.output.push(text_output("extra literal text".into()));
    let decoded = source(&call, false).unwrap();
    assert!(
        decoded
            .sections
            .iter()
            .any(|section| section.text.contains("extra literal text"))
    );
}

#[test]
fn errors_and_redaction_preserve_literal_result_boundaries() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut payload = exec_result("failed");
    payload["stdout"] = json!("password=secret\r\n");
    payload["stderr"] = json!("ghp_abcdefghijklmnop\t\n");
    payload["error"] = json!("token=hidden\n");
    let call = call(
        &authority,
        "exec_command",
        json!({"terminal_id":"t1","program":"true"}),
        payload,
        true,
    );
    let output = source(&call, true).unwrap();
    for section in &output.sections {
        assert_eq!(section.text, nocterm_ai::redact::redact(&section.text));
        assert!(!section.text.contains("secret") && !section.text.contains("abcdefghijklmnop"));
    }
    let mut error = call;
    error.result = Some(
        OwnedJson::from_untracked(
            &authority,
            json!({"isError":true,"content":[{"type":"text","text":"permission denied\n"}]}),
        )
        .unwrap(),
    );
    let output = source(&error, false).unwrap();
    assert_eq!(output.sections[0].label, "Error");
    assert_eq!(output.sections[0].text, "permission denied\n");
}

#[gpui_kit::test]
fn literal_output_copy_updates_with_late_results_and_stays_bounded(cx: &mut TestAppContext) {
    let mut payload = exec_result("running");
    let literal = format!(
        "{}\n{}",
        "界🚀longline".repeat(100),
        "\t```*literal*```\r\n".repeat(16000)
    );
    payload["stdout"] = json!(literal);
    let f = expanded_fixture(
        |authority| {
            call(
                authority,
                "exec_command",
                json!({"terminal_id":"t1","program":"true"}),
                payload,
                true,
            )
        },
        cx,
    );
    cx.update_window(f.handle, |_, window, cx| {
        assert_eq!(
            window.find(("copy-tool-output", 0usize)).label(),
            Some("Copy standard output")
        );
        window.click(("copy-tool-output", 0usize), cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), literal);
        let viewport = window.find(("tool-output-scroll", 0usize)).bounds();
        let code = window.find(("tool-output-code", 0usize)).bounds();
        let entry = window.find(("agent-entry", 0usize)).bounds();
        assert!(viewport.size.height <= px(240.));
        assert!(viewport.right() <= entry.right() + px(1.));
        assert!(code.size.width > viewport.size.width);
        assert!(code.size.height > viewport.size.height);
        window.click(("copy-tool-output-extra-0", 0usize), cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "warning\n"
        );
    })
    .unwrap();
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
    let payload = exec_result("exited");
    f.events
        .try_send(AgentEvent::Session(SessionNotice {
            session,
            update: SessionUpdate::ToolChanged(ToolPatch {
                id: ToolId::new("screenshot"),
                changes: ToolChanges {
                    result: Some(
                        OwnedJson::from_untracked(&f.installation.retention, payload.clone())
                            .unwrap(),
                    ),
                    ..Default::default()
                },
                extensions: None,
                claims: Default::default(),
                literal: Default::default(),
            }),
        }))
        .unwrap();
    cx.run_until_parked();
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click(("copy-tool-output", 0usize), cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            payload["stdout"].as_str().unwrap()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn displayed_and_copied_output_follow_redaction_policy(cx: &mut TestAppContext) {
    let mut payload = exec_result("running");
    let original = "password=secret\r\n```literal```\n";
    payload["stdout"] = json!(original);
    let f = expanded_fixture(
        |authority| {
            call(
                authority,
                "exec_command",
                json!({"terminal_id":"t1","program":"true"}),
                payload,
                true,
            )
        },
        cx,
    );
    for redacts in [true, false] {
        cx.update(|cx| {
            f.installation
                .ready
                .settings()
                .update::<nocterm_ai::AiSettings>(cx, move |settings| {
                    settings.approval.redact_secrets = redacts;
                })
                .detach();
        });
        cx.run_until_parked();
        settle(&f, cx);
        cx.update_window(f.handle, |_, window, cx| {
            window.click(("copy-tool-output", 0usize), cx);
            let copied = cx.read_from_clipboard().unwrap().text().unwrap();
            assert_eq!(
                copied,
                if redacts {
                    nocterm_ai::redact::redact(original)
                } else {
                    original.into()
                }
            );
            let thread = f.panel.read(cx).current().unwrap();
            let facts = thread.read(cx).tool_facts();
            assert_eq!(facts[0].result.as_ref().unwrap()["stdout"], original);
        })
        .unwrap();
    }
}

#[test]
fn many_generic_blocks_share_one_literal_preview_and_keep_every_block() {
    let mut call = super::provider_intersections::provider_tool(
        "foreign",
        "Foreign tool",
        ToolStatus::Pending,
    );
    call.output = (0..1000)
        .map(|index| text_output(format!("block {index}\n")))
        .collect();
    let output = source(&call, false).unwrap();
    assert_eq!(output.sections.len(), 1);
    assert!(output.sections[0].text.contains("block 0\n"));
    assert!(output.sections[0].text.contains("block 999\n"));
}

#[gpui_kit::test]
fn all_output_sections_share_the_preview_budget_but_copy_keeps_the_full_result(
    cx: &mut TestAppContext,
) {
    let mut payload = exec_result("running");
    let stdout = "x".repeat(200 * 1024 - 32);
    let stderr = "error\n".repeat(10000);
    payload["stdout"] = json!(stdout);
    payload["stderr"] = json!(stderr);
    let f = expanded_fixture(
        |authority| {
            call(
                authority,
                "exec_command",
                json!({"terminal_id":"t1","program":"true"}),
                payload,
                true,
            )
        },
        cx,
    );
    cx.update_window(f.handle, |_, window, cx| {
        let viewport = window.find(("tool-output-scroll-extra-0", 0usize)).bounds();
        let code = window.find(("tool-output-code-extra-0", 0usize)).bounds();
        assert!(
            code.size.height < px(160.),
            "stderr gets the remaining32-byte preview, not200KiB: {code:?}, {viewport:?}"
        );
        window.click(("copy-tool-output-extra-0", 0usize), cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), stderr);
    })
    .unwrap();
}

#[test]
fn windows_process_exit_status_keeps_its_unsigned_value() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut payload = exec_result("exited");
    payload["exit_status"] = json!(0xC0000005u32);
    let call = call(
        &authority,
        "exec_command",
        json!({"terminal_id":"t1", "program":"cmd"}),
        payload,
        false,
    );
    let output = source(&call, false).unwrap();
    assert_eq!(output.sections[0].label, "Standard output");
    assert!(
        output
            .parameters
            .contains(&"Exit status: 3221225477".into())
    );
}

#[test]
fn unexpected_large_metadata_stays_inside_generic_bounded_output() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut payload = exec_result("running");
    payload["command_id"] = json!("large".repeat(10000));
    let call = call(
        &authority,
        "read_command",
        json!({"terminal_id":"t1", "command_id":"c7"}),
        payload,
        false,
    );
    let output = source(&call, false).unwrap();
    assert!(output.parameters.is_empty());
    assert_eq!(output.sections[0].label, "Tool output");
    assert!(output.sections[0].text.contains("largelarge"));
    let call = self::call(
        &authority,
        "read_terminal",
        json!({"terminal_id":"t1"}),
        json!({"text":"literal", "first_line":0, "next_line":1, "cursor_semantics":"large".repeat(10000), "truncated":false, "alt_screen":false}),
        false,
    );
    let output = source(&call, false).unwrap();
    assert!(output.parameters.is_empty());
    assert!(output.sections[0].text.contains("largelarge"));
}
