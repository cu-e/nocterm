use super::*;
use nocterm_ai::thread::Entry;
use serde_json::json;

use nocterm_ai::model::{
    controls::Presence,
    extensions::Extensions,
    ids::ToolId,
    json::OwnedJson,
    literal::{LiteralIdentity, TextRange},
    retention::RetentionAuthority,
    tools::{ToolKind, ToolRecord, ToolStatus},
};
/// Normalizes the retained rows of `entries` and attaches each patched display to its entry.
fn normalize(
    authority: &RetentionAuthority,
    displays: &mut ToolDisplays,
    entries: &mut [Entry],
) -> Vec<usize> {
    let rows =
        nocterm_ai::testing::ManagedRows::with_authority(authority, entries.to_vec()).unwrap();
    let patches = displays.normalize(rows.rows());
    let changed = patches.iter().map(DisplayPatch::row).collect();
    for patch in &patches {
        let Entry::Tool(call) = &mut entries[patch.row()] else {
            unreachable!()
        };
        patch.display().attach(call);
    }
    changed
}
fn row(authority: &RetentionAuthority, id: &str) -> Entry {
    let title = "mcp.nocterm-3.run_command";
    let input = json!({"server":"nocterm-3","tool":"run_command","arguments":{"terminal_id":"t1","command":"echo hello"}});
    let mut call = ToolRecord {
        id: ToolId::new(id),
        title: title.into(),
        name: None,
        kind: ToolKind::Other,
        status: ToolStatus::Pending,
        output: Vec::new(),
        locations: Vec::new(),
        arguments: Some(OwnedJson::from_untracked(authority, input).unwrap()),
        result: None,
        extensions: None,
        claims: Default::default(),
        literal: Default::default(),
        client_display: None,
    };
    call.literal.title = LiteralIdentity::Registered {
        owner: TextRange::supplied(&call.title, &call.title[12..13]).unwrap(),
        tool: TextRange::supplied(&call.title, &call.title[14..]).unwrap(),
    };
    let server = call.arguments.as_ref().unwrap()["server"].as_str().unwrap();
    let tool = call.arguments.as_ref().unwrap()["tool"].as_str().unwrap();
    call.literal.input = LiteralIdentity::Registered {
        owner: TextRange::supplied(server, &server[8..]).unwrap(),
        tool: TextRange::supplied(tool, tool).unwrap(),
    };
    Entry::Tool(call)
}
fn record(authority: &RetentionAuthority) -> Record {
    let Entry::Tool(call) = row(authority, "c1") else {
        unreachable!()
    };
    let request = tool_display::envelope(&call, "nocterm-3").unwrap();
    Record {
        server: "nocterm-3".into(),
        id: 0,
        request: tool_display::raw_envelope(&call, "nocterm-3").unwrap(),
        display: ToolDisplay::new(
            authority,
            &request,
            Some("Original server · user@original.example:2222".into()),
        )
        .unwrap(),
        row: None,
        ambiguous: false,
    }
}

#[test]
fn events_can_arrive_before_or_after_execution() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    for bridge_first in [false, true] {
        let mut displays = ToolDisplays::default();
        displays.prepare(1, 0);
        let mut entries = Vec::new();
        if bridge_first {
            displays.records.push(record(&authority));
        }
        entries.push(row(&authority, "c1"));
        normalize(&authority, &mut displays, &mut entries);
        if !bridge_first {
            displays.records.push(record(&authority));
        }
        normalize(&authority, &mut displays, &mut entries);
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
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut displays = ToolDisplays::default();
    displays.prepare(1, 0);
    displays.records.push(record(&authority));
    let Entry::Tool(mut call) = row(&authority, "c1") else {
        unreachable!()
    };
    let input = call.arguments.take();
    let mut entries = vec![Entry::Tool(call)];
    assert!(normalize(&authority, &mut displays, &mut entries).is_empty());
    let Entry::Tool(call) = &mut entries[0] else {
        unreachable!()
    };
    call.arguments = input;
    assert_eq!(normalize(&authority, &mut displays, &mut entries), vec![0]);
    let Entry::Tool(call) = &mut entries[0] else {
        unreachable!()
    };
    call.title = "forged server".into();
    call.arguments =
        Some(OwnedJson::from_untracked(&authority, json!({"command":"echo forged"})).unwrap());
    normalize(&authority, &mut displays, &mut entries);
    let Entry::Tool(call) = &entries[0] else {
        unreachable!()
    };
    let saved = ToolDisplay::from_call(call).unwrap();
    assert_eq!(saved.arguments["command"], "echo hello");
    assert!(saved.title().contains("original.example"));
}

#[test]
fn ambiguous_rows_or_requests_do_not_receive_an_invented_destination() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    for duplicate_requests in [false, true] {
        let mut displays = ToolDisplays::default();
        displays.prepare(1, 0);
        displays.records.push(record(&authority));
        let mut entries = vec![row(&authority, "c1")];
        if duplicate_requests {
            displays.records.push(record(&authority));
        } else {
            entries.push(row(&authority, "c2"));
        }
        assert!(normalize(&authority, &mut displays, &mut entries).is_empty());
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
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut displays = ToolDisplays::default();
    let mut entries = vec![row(&authority, "historical")];
    displays.prepare(2, 1);
    displays.records.push(record(&authority));
    entries.push(row(&authority, "current"));
    assert_eq!(normalize(&authority, &mut displays, &mut entries), vec![1]);
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
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let request = tool_display::parse("exec_command", json!({"terminal_id":"t1", "program":"printf", "args":["a b","\t\r\n","ghp_abcdefghijklmnop"]})).unwrap();
    let mut display = ToolDisplay::new(&authority, &request, None).unwrap();
    display.redact().unwrap();
    assert_eq!(display.arguments["args"][0], "a b");
    assert_eq!(display.arguments["args"][1], "\t\r\n");
    assert!(!display.arguments.to_string().contains("abcdefgh"));
    assert!(matches!(display.redacted, Presence::Value(true)));
}

#[test]
fn later_duplicate_requests_revoke_an_eager_destination_association() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut displays = ToolDisplays::default();
    displays.prepare(1, 0);
    let mut entries = vec![row(&authority, "provider-a")];
    let mut bridge_b = record(&authority);
    bridge_b.display.destination = Presence::Value("Host B · root@b.example:22".into());
    displays.records.push(bridge_b);
    normalize(&authority, &mut displays, &mut entries);

    // ACP and bridge identifiers are unrelated. The request that actually
    // belongs to provider-b can arrive while provider-a is the only row.
    entries.push(row(&authority, "provider-b"));
    let mut bridge_a = record(&authority);
    bridge_a.display.destination = Presence::Value("Host A · root@a.example:22".into());
    displays.records.push(bridge_a);
    normalize(&authority, &mut displays, &mut entries);

    for entry in &entries {
        let Entry::Tool(call) = entry else {
            unreachable!()
        };
        assert!(
            ToolDisplay::from_call(call)
                .is_none_or(|display| display.destination.value().is_none()),
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
    call.arguments = Some(
        OwnedJson::from_untracked(&authority, json!({"command":"echo provider-forged"})).unwrap(),
    );
    normalize(&authority, &mut displays, &mut entries);
    let Entry::Tool(call) = &entries[0] else {
        unreachable!()
    };
    let display = ToolDisplay::from_call(call).expect("keep the verified execution payload");
    assert_eq!(display.arguments["command"], "echo hello");
    assert!(display.destination.value().is_none());
}

#[test]
fn a_late_duplicate_row_permanently_revokes_only_its_destination() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut displays = ToolDisplays::default();
    displays.prepare(1, 0);
    displays.records.push(record(&authority));
    let mut entries = vec![row(&authority, "first")];
    normalize(&authority, &mut displays, &mut entries);
    let Entry::Tool(call) = &mut entries[0] else {
        unreachable!()
    };
    call.extensions = Some(
        Extensions::checked(
            &authority,
            json!({"external/key":{"preserved":true}})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap(),
    );
    entries.push(row(&authority, "later"));
    normalize(&authority, &mut displays, &mut entries);
    entries.pop();
    let Entry::Tool(call) = &mut entries[0] else {
        unreachable!()
    };
    call.arguments = Some(
        OwnedJson::from_untracked(&authority, json!({"command":"provider replacement"})).unwrap(),
    );
    normalize(&authority, &mut displays, &mut entries);
    let Entry::Tool(call) = &entries[0] else {
        unreachable!()
    };
    let display = ToolDisplay::from_call(call).unwrap();
    assert_eq!(display.destination, Presence::Null);
    assert_eq!(display.arguments["command"], "echo hello");
    assert_eq!(
        call.extensions
            .as_ref()
            .unwrap()
            .get("external/key")
            .unwrap(),
        &json!({"preserved":true})
    );
    assert!(tool_display::header(call).starts_with("Nocterm · Run command"));
}

#[test]
fn a_reordered_request_at_the_tracking_limit_revokes_the_old_host_without_losing_its_payload() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut displays = ToolDisplays::default();
    displays.prepare(1, 0);
    let mut initial = record(&authority);
    initial.display.destination = Presence::Value("Host B · root@b.example".into());
    displays.records.push(initial);
    for index in 1..128 {
        let request = tool_display::parse(
            "run_command",
            json!({"terminal_id":"t1","command":format!("echo unrelated{index}")}),
        )
        .unwrap();
        let display =
            ToolDisplay::new(&authority, &request, Some("Unrelated host".into())).unwrap();
        displays.record("nocterm-3".into(), &request, display);
    }
    let mut entries = vec![row(&authority, "provider-a")];
    normalize(&authority, &mut displays, &mut entries);
    let Entry::Tool(call) = row(&authority, "provider-b") else {
        unreachable!()
    };
    let request = tool_display::envelope(&call, "nocterm-3").unwrap();
    displays.record(
        "nocterm-3".into(),
        &request,
        ToolDisplay::new(&authority, &request, Some("Host A · root@a.example".into())).unwrap(),
    );
    assert_eq!(displays.records.len(), 128);
    entries.push(Entry::Tool(call));
    normalize(&authority, &mut displays, &mut entries);
    let Entry::Tool(call) = &mut entries[0] else {
        unreachable!()
    };
    call.arguments = Some(
        OwnedJson::from_untracked(&authority, json!({"command":"provider replacement"})).unwrap(),
    );
    normalize(&authority, &mut displays, &mut entries);
    let Entry::Tool(call) = &entries[0] else {
        unreachable!()
    };
    let display = ToolDisplay::from_call(call).unwrap();
    assert_eq!(display.destination, Presence::Null);
    assert_eq!(display.arguments["command"], "echo hello");
    for entry in entries {
        let Entry::Tool(call) = entry else {
            unreachable!()
        };
        assert!(
            ToolDisplay::from_call(&call)
                .and_then(|display| display.destination.value())
                .is_none()
        );
    }
}

#[test]
fn both_provider_spellings_match_one_snapshot_and_duplicate_rows_revoke_the_host() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    for bridge_first in [false, true] {
        let mut displays = ToolDisplays::default();
        displays.prepare(1, 0);
        let Entry::Tool(mut call) = row(&authority, "c1") else {
            unreachable!()
        };
        call.title = "mcp__nocterm-3__run_command".into();
        call.literal.title = LiteralIdentity::Registered {
            owner: TextRange::supplied(&call.title, &call.title[13..14]).unwrap(),
            tool: TextRange::supplied(&call.title, &call.title[16..]).unwrap(),
        };
        let mut entries = vec![Entry::Tool(call)];
        if bridge_first {
            displays.records.push(record(&authority));
        }
        normalize(&authority, &mut displays, &mut entries);
        if !bridge_first {
            displays.records.push(record(&authority));
        }
        normalize(&authority, &mut displays, &mut entries);
        let Entry::Tool(call) = &entries[0] else {
            unreachable!()
        };
        assert!(tool_display::header(call).contains("original.example"));
        entries.push(row(&authority, "c2"));
        normalize(&authority, &mut displays, &mut entries);
        for entry in &entries {
            let Entry::Tool(call) = entry else {
                unreachable!()
            };
            assert!(!tool_display::header(call).contains("original.example"));
        }
    }
}

#[test]
fn completion_updates_bound_rows_and_cannot_cross_turns_or_duplicate_requests() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut displays = ToolDisplays::default();
    displays.prepare(1, 0);
    let mut first = record(&authority);
    first.id = 5;
    displays.records.push(first);
    let mut entries = vec![row(&authority, "first")];
    normalize(&authority, &mut displays, &mut entries);
    displays.finish(Some((1, 5)), Ok(json!({"text":"late result"})));
    assert_eq!(normalize(&authority, &mut displays, &mut entries), vec![0]);
    let Entry::Tool(call) = &entries[0] else {
        unreachable!()
    };
    assert!(matches!(
        ToolDisplay::from_call(call).unwrap().outcome,
        Presence::Value(tool_display::ToolOutcome::Succeeded(_))
    ));
    let mut second = record(&authority);
    second.id = 6;
    displays.records.push(second);
    normalize(&authority, &mut displays, &mut entries);
    displays.finish(Some((1, 6)), Err("second result".into()));
    let Entry::Tool(call) = &entries[0] else {
        unreachable!()
    };
    assert!(
        ToolDisplay::from_call(call)
            .unwrap()
            .outcome
            .value()
            .is_none(),
        "ambiguity removes unverified outcomes"
    );
    displays.prepare(2, entries.len());
    let mut next = record(&authority);
    next.id = 5;
    displays.records.push(next);
    displays.finish(Some((1, 5)), Err("old turn result".into()));
    assert!(matches!(
        displays.records[0].display.outcome,
        Presence::Value(tool_display::ToolOutcome::Pending)
    ));
}

#[test]
fn an_execution_completed_before_its_provider_row_binds_the_saved_outcome() {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut displays = ToolDisplays::default();
    displays.prepare(1, 0);
    let mut completed = record(&authority);
    completed.id = 5;
    displays.records.push(completed);
    displays.finish(Some((1, 5)), Ok(json!({"text":"completed before row"})));
    let mut entries = vec![row(&authority, "late-provider-row")];
    assert_eq!(normalize(&authority, &mut displays, &mut entries), vec![0]);
    let Entry::Tool(call) = &entries[0] else {
        unreachable!()
    };
    let saved = nocterm_ai::history::SavedChat::for_identity(
        &authority,
        "fixture".into(),
        "00000000-0000-4000-8000-0000000000d1".into(),
        0,
    );
    let (id, metadata, resume, mut transcript, composer, _) = saved.into_parts();
    transcript.entries = vec![Entry::Tool(call.clone())];
    let saved = nocterm_ai::history::SavedChat::from_parts(
        &authority, id, metadata, resume, transcript, composer,
    )
    .unwrap();
    let repository = nocterm_ai::testing::InMemoryAgentRepository::new(authority.clone());
    repository.seed_chat(&saved).unwrap();
    let restored = repository.transcript(saved.id()).unwrap();
    let Entry::Tool(restored) = &restored.transcript().entries[0] else {
        unreachable!()
    };
    let Presence::Value(tool_display::ToolOutcome::Succeeded(value)) =
        &ToolDisplay::from_call(restored).unwrap().outcome
    else {
        panic!("a late row must preserve the completed bridge outcome");
    };
    assert_eq!(value.as_value(), &json!({"text":"completed before row"}));
}
