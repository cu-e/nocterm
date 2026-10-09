use super::tool_output::{call, exec_result};
use crate::panel::entries::tool_output::source;
use serde_json::json;

#[test]
fn provider_wrappers_accept_metadata_json_strings_and_multiple_text_blocks() {
    let payload = exec_result("exited");
    let args = json!({"terminal_id":"t1","program":"sh"});
    for wrapper in [
        json!({"result":payload, "jsonrpc":"2.0", "id":3,"error":null,"_meta":{"future":true},"future":"ignored"}),
        json!({"content":[{"type":"text","text":payload.to_string(),"annotations":{"audience":["user"]}}],"structuredContent":payload,"_meta":{},"annotations":{}}),
        json!([{ "type":"text","text":payload.to_string() }]),
        json!(payload.to_string()),
    ] {
        let output = source(&call("exec_command", args.clone(), wrapper, false), false).unwrap();
        assert_eq!(output.sections[0].label, "Standard output");
        assert_eq!(output.sections[0].text, payload["stdout"].as_str().unwrap());
    }
    let wrapper = json!({"content":[{"type":"text","text":payload.to_string()},{"type":"text","text":"second block"}]});
    let output = source(&call("exec_command", args, wrapper, false), false).unwrap();
    assert_eq!(output.sections.len(), 3);
    assert_eq!(output.sections[2].text, "second block");
}

#[test]
fn provider_errors_take_precedence_over_null_results_and_allow_metadata() {
    let args = json!({"terminal_id":"t1","program":"sh"});
    for wrapper in [
        json!({"error":"message","_meta":{}}),
        json!({"error":{"message":"message","code":-32602},"result":null,"jsonrpc":"2.0","id":42}),
        json!({"isError":true,"content":[{"type":"text","text":"message"}],"structuredContent":{},"_meta":{}}),
    ] {
        let output = source(&call("exec_command", args.clone(), wrapper, false), false).unwrap();
        assert_eq!(output.sections.len(), 1);
        assert_eq!(output.sections[0].label, "Error");
        assert_eq!(output.sections[0].text, "message");
    }
}

#[test]
fn future_payloads_are_readable_and_recursive_wrappers_stay_bounded() {
    let args = json!({"terminal_id":"t1","program":"sh"});
    let payload = json!({"result":{"future":{"lines":["one","two"]}}, "_meta":{}});
    let output = source(&call("exec_command", args.clone(), payload, false), false).unwrap();
    assert_eq!(output.sections[0].text, "future: lines: one\n\ntwo");
    let mut nested = exec_result("exited");
    for _ in 0..32 {
        nested = json!({"result":nested});
    }
    let output = source(&call("exec_command", args.clone(), nested, false), false).unwrap();
    assert!(output.sections[0].text.contains("exceeds display limits"));
    let large = json!({"result":"x".repeat(512*1024)});
    let output = source(&call("exec_command", args, large, false), false).unwrap();
    assert!(output.sections[0].text.contains("exceeds display limits"));
}

#[test]
fn familiar_but_malformed_neighbor_keys_do_not_block_unwrapping() {
    let payload = exec_result("exited");
    for extra in [
        json!({"accepted":null}),
        json!({"context":7}),
        json!({"terminal":false}),
        json!({"command_id":"metadata","stdout":7}),
        json!({"text":null,"first_line":"metadata"}),
    ] {
        let mut wrapper = json!({"result":payload});
        wrapper
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let output = source(
            &call(
                "exec_command",
                json!({"terminal_id":"t1","program":"sh"}),
                wrapper,
                false,
            ),
            false,
        )
        .unwrap();
        assert_eq!(output.sections[0].label, "Standard output");
    }
}

#[test]
fn is_error_applies_to_recursive_result_wrappers() {
    let output = source(
        &call(
            "exec_command",
            json!({"terminal_id":"t1","program":"sh"}),
            json!({"isError":true,"result":{"result":"denied"},"_meta":{}}),
            false,
        ),
        false,
    )
    .unwrap();
    assert_eq!(output.sections[0].label, "Error");
    assert_eq!(output.sections[0].text, "denied");
}

#[test]
fn malformed_neighbors_do_not_impersonate_other_tools_payloads() {
    for (tool, args, payload, neighbor, expected) in [
        (
            "list_terminals",
            json!({}),
            json!({"context":"attached"}),
            json!({"context":7}),
            "Context",
        ),
        (
            "send_input",
            json!({"terminal_id":"t1","text":"hello"}),
            json!({"accepted":true}),
            json!({"accepted":null}),
            "Tool output",
        ),
        (
            "open_terminal",
            json!({"server_id":"s1"}),
            json!({"terminal":{"id":"t1"}}),
            json!({"terminal":false}),
            "Terminal",
        ),
    ] {
        let mut wrapper = json!({"result":payload});
        wrapper
            .as_object_mut()
            .unwrap()
            .extend(neighbor.as_object().unwrap().clone());
        assert_eq!(
            source(&call(tool, args, wrapper, false), false)
                .unwrap()
                .sections[0]
                .label,
            expected
        );
    }
}
