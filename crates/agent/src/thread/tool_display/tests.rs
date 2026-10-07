use super::*;
use serde_json::json;

fn row(id: &str) -> Entry {
    Entry::Tool(acp::ToolCall::new(id.to_owned(), "mcp.nocterm-3.run_command").raw_input(json!({
        "server":"nocterm-3","tool":"run_command","arguments":{"terminal_id":"t1","command":"echo hello"}
    })))
}

fn record() -> Record {
    let Entry::Tool(call) = row("c1") else {
        unreachable!()
    };
    let request = tool_display::envelope(&call, "nocterm-3").unwrap();
    Record {
        server: "nocterm-3".into(),
        request: tool_display::request(&request),
        display: ToolDisplay::new(
            &request,
            Some("Original server · user@original.example:2222".into()),
        ),
        row: None,
        ambiguous: false,
    }
}

#[test]
fn events_can_arrive_before_or_after_execution() {
    for bridge_first in [false, true] {
        let mut displays = ToolDisplays::default();
        displays.prepare(1, 0);
        let mut entries = Vec::new();
        if bridge_first {
            displays.records.push(record());
        }
        entries.push(row("c1"));
        displays.normalize(&mut entries);
        if !bridge_first {
            displays.records.push(record());
        }
        displays.normalize(&mut entries);
        let Entry::Tool(call) = &entries[0] else {
            unreachable!()
        };
        assert!(
            tool_display::header(call).starts_with("Original server · user@original.example:2222")
        );
    }
}

#[test]
fn late_input_can_match_but_later_provider_updates_cannot_rewrite_verified_payload() {
    let mut displays = ToolDisplays::default();
    displays.prepare(1, 0);
    displays.records.push(record());
    let Entry::Tool(mut call) = row("c1") else {
        unreachable!()
    };
    let input = call.raw_input.take();
    let mut entries = vec![Entry::Tool(call)];
    assert!(displays.normalize(&mut entries).is_empty());
    let Entry::Tool(call) = &mut entries[0] else {
        unreachable!()
    };
    call.raw_input = input;
    assert_eq!(displays.normalize(&mut entries), vec![0]);
    let Entry::Tool(call) = &mut entries[0] else {
        unreachable!()
    };
    call.title = "forged server".into();
    call.raw_input = Some(json!({"command":"echo forged"}));
    displays.normalize(&mut entries);
    let Entry::Tool(call) = &entries[0] else {
        unreachable!()
    };
    let saved = ToolDisplay::from_call(call).unwrap();
    assert_eq!(saved.arguments["command"], "echo hello");
    assert!(saved.title().contains("original.example"));
}

#[test]
fn ambiguous_rows_or_requests_do_not_receive_an_invented_destination() {
    for duplicate_requests in [false, true] {
        let mut displays = ToolDisplays::default();
        displays.prepare(1, 0);
        displays.records.push(record());
        let mut entries = vec![row("c1")];
        if duplicate_requests {
            displays.records.push(record());
        } else {
            entries.push(row("c2"));
        }
        assert!(displays.normalize(&mut entries).is_empty());
        for entry in entries {
            let Entry::Tool(call) = entry else {
                unreachable!()
            };
            assert!(ToolDisplay::from_call(&call).is_none());
        }
    }
}

#[test]
fn a_new_turn_cannot_backfill_an_old_unmatched_row() {
    let mut displays = ToolDisplays::default();
    let mut entries = vec![row("historical")];
    displays.prepare(2, 1);
    displays.records.push(record());
    entries.push(row("current"));
    assert_eq!(displays.normalize(&mut entries), vec![1]);
    let Entry::Tool(old) = &entries[0] else {
        unreachable!()
    };
    assert!(ToolDisplay::from_call(old).is_none());
    displays.prepare(3, entries.len());
    assert!(displays.records.is_empty());
    let Entry::Tool(current) = &entries[1] else {
        unreachable!()
    };
    assert!(ToolDisplay::from_call(current).is_some());
}

#[test]
fn redaction_walk_preserves_literal_boundaries() {
    let request = tool_display::parse("exec_command", json!({"terminal_id":"t1", "program":"printf", "args":["a b","\t\r\n","ghp_abcdefghijklmnop"]})).unwrap();
    let mut display = ToolDisplay::new(&request, None);
    display.redact();
    assert_eq!(display.arguments["args"][0], "a b");
    assert_eq!(display.arguments["args"][1], "\t\r\n");
    assert!(!display.arguments.to_string().contains("abcdefgh"));
    assert!(display.redacted);
}

#[test]
fn later_duplicate_requests_revoke_an_eager_destination_association() {
    let mut displays = ToolDisplays::default();
    displays.prepare(1, 0);
    let mut entries = vec![row("provider-a")];
    let mut bridge_b = record();
    bridge_b.display.destination = Some("Host B · root@b.example:22".into());
    displays.records.push(bridge_b);
    displays.normalize(&mut entries);

    // ACP and bridge identifiers are unrelated. The request that actually
    // belongs to provider-b can arrive while provider-a is the only row.
    entries.push(row("provider-b"));
    let mut bridge_a = record();
    bridge_a.display.destination = Some("Host A · root@a.example:22".into());
    displays.records.push(bridge_a);
    displays.normalize(&mut entries);

    for entry in &entries {
        let Entry::Tool(call) = entry else {
            unreachable!()
        };
        assert!(
            ToolDisplay::from_call(call).is_none_or(|display| display.destination.is_none()),
            "indistinguishable reordered requests must not claim a verified destination: {}",
            tool_display::header(call),
        );
        let header = tool_display::header(call);
        assert!(
            !header.contains("Host A") && !header.contains("Host B"),
            "{header}"
        );
    }

    // The executed literal remains authoritative even after losing the proof
    // of which destination belongs to this ACP row.
    let Entry::Tool(call) = &mut entries[0] else {
        unreachable!()
    };
    call.raw_input = Some(json!({"command":"echo provider-forged"}));
    displays.normalize(&mut entries);
    let Entry::Tool(call) = &entries[0] else {
        unreachable!()
    };
    let display = ToolDisplay::from_call(call).expect("keep the verified execution payload");
    assert_eq!(display.arguments["command"], "echo hello");
    assert!(display.destination.is_none());
}

#[test]
fn a_late_duplicate_row_permanently_revokes_only_its_destination() {
    let mut displays = ToolDisplays::default();
    displays.prepare(1, 0);
    displays.records.push(record());
    let mut entries = vec![row("first")];
    displays.normalize(&mut entries);
    let Entry::Tool(call) = &mut entries[0] else {
        unreachable!()
    };
    call.meta
        .as_mut()
        .unwrap()
        .insert("external/key".into(), json!({"preserved":true}));
    entries.push(row("later"));
    displays.normalize(&mut entries);
    entries.pop();
    let Entry::Tool(call) = &mut entries[0] else {
        unreachable!()
    };
    call.raw_input = Some(json!({"command":"provider replacement"}));
    displays.normalize(&mut entries);
    let Entry::Tool(call) = &entries[0] else {
        unreachable!()
    };
    let display = ToolDisplay::from_call(call).unwrap();
    assert_eq!(display.destination, None);
    assert_eq!(display.arguments["command"], "echo hello");
    assert_eq!(
        call.meta.as_ref().unwrap()["external/key"],
        json!({"preserved":true})
    );
    assert!(tool_display::header(call).starts_with("Nocterm · Run command"));
}

#[test]
fn a_reordered_request_at_the_tracking_limit_revokes_the_old_host_without_losing_its_payload() {
    let mut displays = ToolDisplays::default();
    displays.prepare(1, 0);
    let mut initial = record();
    initial.display.destination = Some("Host B · root@b.example".into());
    displays.records.push(initial);
    for index in 1..128 {
        let request = tool_display::parse(
            "run_command",
            json!({"terminal_id":"t1","command":format!("echo unrelated{index}")}),
        )
        .unwrap();
        let display = ToolDisplay::new(&request, Some("Unrelated host".into()));
        displays.record("nocterm-3".into(), &request, display);
    }
    let mut entries = vec![row("provider-a")];
    displays.normalize(&mut entries);
    let Entry::Tool(call) = row("provider-b") else {
        unreachable!()
    };
    let request = tool_display::envelope(&call, "nocterm-3").unwrap();
    displays.record(
        "nocterm-3".into(),
        &request,
        ToolDisplay::new(&request, Some("Host A · root@a.example".into())),
    );
    assert_eq!(displays.records.len(), 128);
    entries.push(Entry::Tool(call));
    displays.normalize(&mut entries);
    let Entry::Tool(call) = &mut entries[0] else {
        unreachable!()
    };
    call.raw_input = Some(json!({"command":"provider replacement"}));
    displays.normalize(&mut entries);
    let Entry::Tool(call) = &entries[0] else {
        unreachable!()
    };
    let display = ToolDisplay::from_call(call).unwrap();
    assert_eq!(display.destination, None);
    assert_eq!(display.arguments["command"], "echo hello");
    for entry in entries {
        let Entry::Tool(call) = entry else {
            unreachable!()
        };
        assert!(
            ToolDisplay::from_call(&call)
                .and_then(|display| display.destination)
                .is_none()
        );
    }
}
