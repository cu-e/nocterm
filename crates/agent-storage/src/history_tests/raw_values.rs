//! Legacy reserved raw JSON semantics at the public storage boundary.
use super::*;
use futures::executor::block_on;
use nocterm_ai::storage::ChatRepository;

const RAW_KEY: &str = "$serde_json::private::RawValue";

fn raw(document: &str) -> serde_json::Value {
    serde_json::json!({ (RAW_KEY): document })
}

fn load_tool(
    authority: &nocterm_ai::model::retention::RetentionAuthority,
    value: serde_json::Value,
    expected: (usize, usize, usize),
) -> (tempfile::TempDir, SavedChat, Vec<u8>) {
    let directory = tempfile::tempdir().unwrap();
    let id = "f5a91dea-3e07-4c07-a4de-2784980039a2";
    let original = serde_json::to_vec(&serde_json::json!({
        "version":1,"id":id,"agent_id":"provider","updated":1,
        "entries":[{"kind":"tool","value":value}]
    }))
    .unwrap();
    assert_eq!(original.len(), expected.0);
    assert!(original.len() < MAX_FILE_BYTES as usize);
    let mut decoder = serde_json::Deserializer::from_slice(&original);
    let bounds = nocterm_ai::history::preflight_decoder(&mut decoder).unwrap();
    decoder.end().unwrap();
    assert_eq!(
        (bounds.nodes, bounds.decoded_text_bytes),
        (expected.1, expected.2)
    );
    assert_eq!(bounds.structural_bytes, 512 * expected.1 + 2 * expected.2);
    assert!(bounds.structural_bytes < nocterm_ai::history::MAX_RESIDENT_BYTES);
    std::fs::write(directory.path().join(format!("{id}.json")), &original).unwrap();
    let repository =
        FileChatRepository::with_retention(directory.path().to_owned(), authority.clone());
    let loaded = block_on(repository.load(id.into())).unwrap();
    assert_eq!(
        std::fs::read(directory.path().join(format!("{id}.json"))).unwrap(),
        original
    );
    (directory, loaded, original)
}

#[test]
fn public_repository_expands_nested_reserved_raw_arguments() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let inner = serde_json::to_string(&"retained semantic text").unwrap();
    let outer = serde_json::to_string(&raw(&inner)).unwrap();
    let (_directory, loaded, _original) = load_tool(
        &authority,
        serde_json::json!({
            "toolCallId":"original","title":"raw","rawInput":raw(&outer)
        }),
        (287, 24, 215),
    );
    let Entry::Tool(call) = &loaded.transcript().entries[0] else {
        panic!("tool row")
    };
    assert_eq!(
        call.arguments.as_ref().unwrap().as_value(),
        "retained semantic text"
    );
}

#[test]
fn public_repository_discards_missing_path_diff_with_reserved_raw_metadata() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let (_directory, loaded, _original) = load_tool(
        &authority,
        serde_json::json!({
            "toolCallId":"original","title":"raw",
            "content":[{"type":"diff","newText":"unused",
                "_meta":{"vendor":raw("[null,null,null]")}}]
        }),
        (287, 33, 199),
    );
    let Entry::Tool(call) = &loaded.transcript().entries[0] else {
        panic!("tool row")
    };
    assert!(call.output.is_empty());
    assert_eq!(call.id.as_str(), "original");
}

#[test]
fn public_repository_defaults_invalid_reserved_raw_arguments_without_losing_tool() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let (_directory, loaded, _original) = load_tool(
        &authority,
        serde_json::json!({
            "toolCallId":"original","title":"raw","rawInput":raw("not json")
        }),
        (224, 24, 160),
    );
    let Entry::Tool(call) = &loaded.transcript().entries[0] else {
        panic!("tool row")
    };
    assert!(call.arguments.is_none());
    assert_eq!(call.title, "raw");
}

#[test]
fn public_repository_defaults_invalid_raw_descendant_even_when_its_key_is_overwritten() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let (_directory, loaded, _original) = load_tool(
        &authority,
        serde_json::json!({
            "toolCallId":"original","title":"raw",
            "rawInput":raw(r#"{"x":{"$serde_json::private::RawValue":"not json"},"x":null}"#)
        }),
        (284, 24, 212),
    );
    let Entry::Tool(call) = &loaded.transcript().entries[0] else {
        panic!("tool row")
    };
    assert!(call.arguments.is_none());
    assert_eq!(call.title, "raw");
}

#[test]
fn public_repository_preserves_tool_sequence_fields_and_defaulted_tail() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    // The original derived Call grammar accepts sequence representation, even
    // though its enclosing adjacent-tagged Entry remains an object.
    for (call, expected) in [
        (r#"["original","sequence"]"#, (151, 18, 104)),
        (
            r#"["original","sequence",null,null,null,null,null,null,null,null]"#,
            (191, 26, 104),
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let id = "c891fdc3-7ef4-4c43-a306-7f22f5df30f9";
        let bytes = format!(r#"{{"version":1,"id":"{id}","agent_id":"provider","updated":1,"entries":[{{"kind":"tool","value":{call}}}]}}"#).into_bytes();
        assert_eq!(bytes.len(), expected.0);
        let mut decoder = serde_json::Deserializer::from_slice(&bytes);
        let bounds = nocterm_ai::history::preflight_decoder(&mut decoder).unwrap();
        decoder.end().unwrap();
        assert_eq!(
            (bounds.nodes, bounds.decoded_text_bytes),
            (expected.1, expected.2)
        );
        assert_eq!(bounds.structural_bytes, 512 * expected.1 + 2 * expected.2);
        let path = directory.path().join(format!("{id}.json"));
        std::fs::write(&path, &bytes).unwrap();
        let repository =
            FileChatRepository::with_retention(directory.path().to_owned(), authority.clone());
        let loaded = block_on(repository.load(id.into())).unwrap();
        let Entry::Tool(call) = &loaded.transcript().entries[0] else {
            panic!("tool row")
        };
        assert_eq!(call.id.as_str(), "original");
        assert_eq!(call.title, "sequence");
        assert!(call.name.is_none());
        assert!(call.arguments.is_none());
        assert!(call.output.is_empty());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

/// The first tool row's raw arguments keep all 200 nested arrays.
fn assert_deepest_argument_survives(live: &nocterm_ai::history::live::LiveOwner) {
    let nocterm_ai::history::physical::RowBody::Tool(tool) = live.rows()[0].view() else {
        panic!("tool row")
    };
    let arguments = tool.arguments().unwrap();
    let mut value = arguments.as_value();
    for _ in 0..200 {
        value = &value.as_array().unwrap()[0];
    }
    assert_eq!(value, &serde_json::json!(0));
}

#[test]
fn public_v2_roundtrip_preserves_expanded_legacy_raw_depth_across_parser_resets() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let inner = format!("{}0{}", "[".repeat(100), "]".repeat(100));
    let reset = serde_json::to_string(&raw(&inner)).unwrap();
    let outer = format!("{}{}{}", "[".repeat(100), reset, "]".repeat(100));
    let call =
        serde_json::json!({"toolCallId":"original","title":"deep raw","rawInput":raw(&outer)});
    let directory = tempfile::tempdir().unwrap();
    let id = "6b706a83-c5ce-4f46-a116-0170dc77164f";
    let bytes = serde_json::to_vec(&serde_json::json!({"version":1,"id":id,"agent_id":"provider","updated":1,"entries":[{"kind":"tool","value":call}]})).unwrap();
    assert_eq!(bytes.len(), 663);
    let mut decoder = serde_json::Deserializer::from_slice(&bytes);
    let bounds = nocterm_ai::history::preflight_decoder(&mut decoder).unwrap();
    decoder.end().unwrap();
    assert_eq!((bounds.nodes, bounds.decoded_text_bytes), (24, 595));
    assert_eq!(bounds.structural_bytes, 13478);
    let path = directory.path().join(format!("{id}.json"));
    std::fs::write(&path, &bytes).unwrap();
    let repository =
        FileChatRepository::with_retention(directory.path().to_owned(), authority.clone());
    let (_, mut roles) =
        nocterm_ai::model::retention::document::DocumentRetention::new(&authority).unwrap();
    let reader = nocterm_ai::history::ReaderSource::new(
        roles.take_reader().unwrap(),
        roles.take_delivery().unwrap(),
    )
    .unwrap();
    let writer = nocterm_ai::history::writing::WriterProducer::new(
        roles.take_writer_latest().unwrap(),
        roles.take_writer_active().unwrap(),
        roles.take_writer().unwrap(),
    )
    .unwrap();
    let mut candidate = roles.take_candidate().unwrap();
    let mut live = nocterm_ai::history::live::LiveOwner::new(
        roles.take_live().unwrap(),
        roles.take_foreground().unwrap(),
        roles.take_prompt_dispatch().unwrap(),
    )
    .unwrap();
    live.initialize("provider", id, 1).unwrap();
    let load = |live: &mut nocterm_ai::history::live::LiveOwner| {
        let identity = live.prepare_reader_id().unwrap();
        let work = repository
            .load_managed(identity, reader.capture().unwrap())
            .unwrap();
        let delivery = block_on(work);
        assert!(delivery.error().is_none(), "{:?}", delivery.error());
        delivery
    };
    let delivery = load(&mut live);
    live.install_delivery(delivery).unwrap();
    assert_deepest_argument_survives(&live);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    live.set_draft(live.local_admission(), Some("edited draft"))
        .unwrap();
    live.prepare_saved_candidate(live.local_admission(), &mut candidate)
        .unwrap();
    let latest = writer
        .latest(
            &candidate,
            None,
            nocterm_ai::history::writing::ManagedWriteIntent::Snapshot,
        )
        .unwrap();
    block_on(repository.save_owned(latest.capture().unwrap())).unwrap();
    assert_ne!(std::fs::read(&path).unwrap(), bytes);
    drop(latest);
    let delivery = load(&mut live);
    live.install_delivery(delivery).unwrap();
    assert_deepest_argument_survives(&live);
    assert_eq!(
        live.composer().unwrap().draft().as_deref(),
        Some("edited draft")
    );
}

#[test]
fn public_managed_load_keeps_legacy_physical_depth_guard() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let deep = format!("{}0{}", "[".repeat(128), "]".repeat(128));
    let id = "0f5d4b0e-8f4c-4d0b-9a55-0e4c1f2b7a10";
    // Physical source depth, unlike RawValue expansion, exceeds one parser budget.
    let bytes = format!(
        r#"{{"version":1,"id":"{id}","agent_id":"provider","updated":1,"entries":[{{"kind":"tool","value":{{"toolCallId":"original","title":"deep","rawInput":{deep}}}}}]}}"#
    )
    .into_bytes();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(format!("{id}.json"));
    std::fs::write(&path, &bytes).unwrap();
    let repository =
        FileChatRepository::with_retention(directory.path().to_owned(), authority.clone());
    let (_, mut roles) =
        nocterm_ai::model::retention::document::DocumentRetention::new(&authority).unwrap();
    let reader = nocterm_ai::history::ReaderSource::new(
        roles.take_reader().unwrap(),
        roles.take_delivery().unwrap(),
    )
    .unwrap();
    let mut live = nocterm_ai::history::live::LiveOwner::new(
        roles.take_live().unwrap(),
        roles.take_foreground().unwrap(),
        roles.take_prompt_dispatch().unwrap(),
    )
    .unwrap();
    live.initialize("provider", id, 1).unwrap();
    let identity = live.prepare_reader_id().unwrap();
    let work = repository
        .load_managed(identity, reader.capture().unwrap())
        .unwrap();
    let delivery = block_on(work);
    assert!(delivery.error().is_some());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
