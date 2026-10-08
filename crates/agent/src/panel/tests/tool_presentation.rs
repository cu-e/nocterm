use super::tool_output::{call, exec_result, expanded_fixture, settle};
use super::*;
use crate::panel::entries::tool_input::source;
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
