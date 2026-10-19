//! Repository grammar compatibility and native held-source ownership regressions.
use crate::FileChatRepository;
use nocterm_ai::{
    history::{MAX_RESIDENT_BYTES, SavedChat},
    model::retention::RetentionAuthority,
};
use std::fs;
const ID: &str = "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c";
fn load(value: &str) -> std::result::Result<SavedChat, nocterm_ai::storage::AgentStorageError> {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join(format!("{ID}.json")), value).unwrap();
    let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
    futures::executor::block_on(
        FileChatRepository::with_retention(directory.path().into(), authority).load(ID.into()),
    )
}
fn load_managed(value: &str) -> nocterm_ai::history::ReaderDelivery {
    use nocterm_ai::{history::ReaderSource, model::retention::document::DocumentRetention};
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join(format!("{ID}.json")), value).unwrap();
    let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let source =
        ReaderSource::new(roles.take_reader().unwrap(), roles.take_delivery().unwrap()).unwrap();
    let repository = FileChatRepository::with_retention(directory.path().into(), authority);
    let delivery =
        futures::executor::block_on(repository.load_into(ID.into(), source.capture().unwrap()));
    assert!(
        delivery.error().is_none(),
        "controlled public load: {:?}",
        delivery.error()
    );
    delivery
}
fn legacy(rows: &str, tail: &str) -> String {
    format!(
        r#"{{"version":1,"id":"{ID}","agent_id":"provider","updated":17,"entries":{rows}{tail}}}"#
    )
}
#[test]
fn ignored_rawvalue_spelling_does_not_apply_value_semantics() {
    let chat = load(&legacy(
        "[]",
        r#", "ignored":{"$serde_json::private::RawValue":"not json"}"#,
    ))
    .unwrap();
    assert!(chat.transcript().entries.is_empty());
}
#[test]
fn overwritten_invalid_rawvalue_defaults_the_original_tool_field() {
    let rows = r#"[{"kind":"tool","value":{"toolCallId":"t","title":"old","rawInput":{"a":{"$serde_json::private::RawValue":"invalid"},"a":1}}}]"#;
    let chat = load(&legacy(rows, "")).unwrap();
    let nocterm_ai::thread::Entry::Tool(tool) = &chat.transcript().entries[0] else {
        panic!("tool")
    };
    assert!(tool.arguments.is_none());
}
#[test]
fn legacy_opaque_duplicates_keep_first_position_and_last_value() {
    let rows = r#"[{"kind":"tool","value":{"toolCallId":"t","title":"old","rawInput":{"a":1,"b":2,"a":3}}}]"#;
    let chat = load(&legacy(rows, "")).unwrap();
    let nocterm_ai::thread::Entry::Tool(tool) = &chat.transcript().entries[0] else {
        panic!("tool")
    };
    assert_eq!(
        serde_json::to_string(tool.arguments.as_ref().unwrap()).unwrap(),
        r#"{"a":3,"b":2}"#
    );
}
#[test]
fn invalid_output_elements_skip_without_erasing_valid_neighbors() {
    let rows = r#"[{"kind":"tool","value":{"toolCallId":"t","title":"old","content":[{"type":"diff","newText":"missing path"},{"type":"terminal","terminalId":"original"}]}}]"#;
    let chat = load(&legacy(rows, "")).unwrap();
    let nocterm_ai::thread::Entry::Tool(tool) = &chat.transcript().entries[0] else {
        panic!("tool")
    };
    assert_eq!(tool.output.len(), 1);
}
#[test]
fn legacy_deep_metadata_expansion_resets_only_the_original_raw_parser() {
    let inner = format!("{{\"payload\":{}0{}}}", "[".repeat(100), "]".repeat(100));
    let raw = serde_json::to_string(&inner).unwrap();
    let meta = format!(
        r#"{{"future":{}{{"$serde_json::private::RawValue":{raw}}}{}}}"#,
        "[".repeat(100),
        "]".repeat(100)
    );
    let call = format!(r#"{{"toolCallId":"t","title":"old","_meta":{meta}}}"#);
    let original: agent_client_protocol_schema::v1::ToolCall = serde_json::from_str(&call).unwrap();
    assert!(original.meta.as_ref().unwrap().contains_key("future"));
    let rows = format!(r#"[{{"kind":"tool","value":{call}}}]"#);
    let chat = load_managed(&legacy(&rows, ""));
    let nocterm_ai::history::physical::RowBody::Tool(tool) =
        chat.snapshot().unwrap().rows()[0].view()
    else {
        panic!("tool")
    };
    assert!(tool.extensions().unwrap().get("future").is_some());
}
#[test]
fn typed_duplicate_null_is_rejected_before_last_value_collapse() {
    assert!(load(&legacy("[]", r#", "draft":null,"draft":"changed""#)).is_err());
}
#[test]
fn controlled_source_strings_keep_original_fourteen_plus_seventeen_mib_acceptance() {
    let first = "a".repeat(14 * 1024 * 1024);
    let second = "b".repeat(17 * 1024 * 1024);
    let rows = format!(
        r#"[{{"kind":"agent","value":"{first}"}},{{"kind":"thought","value":"{second}"}}]"#
    );
    let chat = load_managed(&legacy(&rows, ""));
    assert_eq!(chat.snapshot().unwrap().rows().len(), 2);
}

fn native_semantic_value(
    source: &str,
) -> std::result::Result<nocterm_ai::model::json::OwnedJson, super::legacy_value::ValueError> {
    use crate::loading::{held_file::HeldFile, json_frames::RootFrames};
    use nocterm_ai::model::json::deferred::DecodeLedger;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("literal.json");
    fs::write(&path, format!(r#"{{"payload":{source}}}"#)).unwrap();
    let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
    let ledger = DecodeLedger::new(&authority, MAX_RESIDENT_BYTES).unwrap();
    let mut held = HeldFile::open(&path).unwrap();
    let stamp = held.preflight_literal(&ledger).unwrap();
    let plan = held
        .plan_pass(&stamp)
        .scan(|reader| {
            let RootFrames::Object(mut frames) = RootFrames::new_literal(reader, 0).unwrap() else {
                panic!("object")
            };
            frames.next_member().unwrap().unwrap()
        })
        .unwrap();
    let node = super::legacy_value::authenticated(
        &plan,
        &mut held,
        |field| field.native_span(),
        &ledger,
        false,
    )
    .unwrap()?;
    Ok(node.into_decoded(&ledger)?.as_owned().clone())
}
#[test]
fn native_semantic_scalar_retains_the_original_null_value() {
    let actual = native_semantic_value("null").unwrap();
    assert_eq!(actual.as_value(), &serde_json::Value::Null);
}
#[test]
fn native_semantic_wide_raw_expansion_matches_the_original_sdk_value() {
    let raw = format!("[{}]", vec!["null"; 128].join(","));
    let source = format!(
        r#"{{"$serde_json::private::RawValue":{}}}"#,
        serde_json::to_string(&raw).unwrap()
    );
    let expected: serde_json::Value = serde_json::from_str(&source).unwrap();
    let actual = native_semantic_value(&source).unwrap();
    assert_eq!(
        serde_json::to_vec(&actual).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
}

mod compatibility;

#[test]
fn managed_reader_routes_both_formats_into_registered_delivery_and_live_owners() {
    use nocterm_ai::{
        history::{ReaderOutcome, physical::RowBody},
        model::retention::document::DocumentRetention,
    };
    let legacy = legacy(r#"[{"kind":"agent","value":"managed literal"}]"#, "");
    let version2 = format!(
        r#"{{"format":"nocterm.chat","version":2,"document":{{"id":"{ID}","metadata":{{"agent_id":"provider","pinned":false,"updated":17}},"resume":{{"config":{{}},"pending_history":false}},"transcript":{{"entries":[{{"kind":"agent","value":"managed literal"}}]}},"composer":{{}}}}}}"#
    );
    for source in [legacy, version2] {
        let expected = nocterm_ai::history::encode(&load(&source).unwrap()).unwrap();
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join(format!("{ID}.json")), source).unwrap();
        let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
        let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
        let reader = roles.take_reader().unwrap();
        let delivery = roles.take_delivery().unwrap();
        let mut live = roles.take_live().unwrap();
        let baseline = scope.retained_and_pending_bytes().unwrap();
        let completion = super::loading::read(directory.path(), ID, reader, delivery);
        let (reader, outcome) = completion.into_parts();
        let parts = match outcome {
            ReaderOutcome::Ready(parts) => parts,
            ReaderOutcome::Failed(error) => panic!("actual native reader: {}", error.error()),
        };
        assert_eq!(&*parts.snapshot().unwrap().id(), ID);
        let delivery = parts.install_into(&mut live).unwrap();
        assert!(delivery.snapshot().is_none());
        let snapshot = live.snapshot().unwrap();
        let encoded =
            serde_json::to_vec(&nocterm_ai::history::physical::SnapshotWire::new(snapshot))
                .unwrap();
        assert_eq!(encoded, expected);
        assert_eq!(
            nocterm_ai::history::encode(&load(std::str::from_utf8(&encoded).unwrap()).unwrap())
                .unwrap(),
            encoded
        );
        assert_eq!(snapshot.rows().len(), 1);
        let RowBody::Agent(text) = snapshot.rows()[0].view() else {
            panic!("agent row")
        };
        assert_eq!(&*text.read(), "managed literal");
        live.clear();
        assert_eq!(scope.retained_and_pending_bytes().unwrap(), baseline);
        drop(reader);
    }
}

#[test]
fn managed_wire_preserves_content_tools_queue_and_selective_image_policy() {
    use nocterm_ai::{
        history::{ReaderOutcome, physical::SnapshotWire},
        model::retention::document::DocumentRetention,
    };
    use serde_json::json;
    let annotation = json!({"audience":["user","assistant"],"modified":"original time","priority":0.25,"extensions":{"z":1,"a":2}});
    let text = json!({"kind":"text","value":{"text":"literal\nUnicode λ","annotations":annotation,"extensions":{"z":1,"a":2}}});
    let image = json!({"kind":"media","value":{"kind":"image","encoded":"i".repeat(nocterm_ai::history::MAX_SAVED_IMAGE_BYTES+1),"mime":"image/png","uri":"original:uri","annotations":null,"extensions":null}});
    let audio = json!({"kind":"media","value":{"kind":"audio","encoded":"audio","mime":"audio/wav","uri":null,"annotations":null,"extensions":null}});
    let reference = json!({"kind":"resource_reference","value":{"uri":"resource:original","name":"original name","title":"title","description":"description","mime":"text/plain","reported_size":42,"annotations":null,"extensions":null}});
    let embedded = [
        json!({"kind":"embedded_resource","value":{"body":{"kind":"text","value":{"uri":"text:original","text":"resource text","mime":"text/plain","extensions":{"future":true}}},"annotations":null,"extensions":null}}),
        json!({"kind":"embedded_resource","value":{"body":{"kind":"encoded","value":{"uri":"blob:original","encoded":"blob","mime":"application/octet-stream","extensions":null}},"annotations":null,"extensions":null}}),
    ];
    let contents = vec![
        text.clone(),
        image.clone(),
        audio,
        reference,
        embedded[0].clone(),
        embedded[1].clone(),
    ];
    let tool = json!({"kind":"tool","value":{"id":"original-tool-id","title":"literal title","name":"exact name","kind":"edit","status":"succeeded","output":[{"kind":"content","value":{"content":image.clone(),"extensions":{"output":true}}},{"kind":"file_diff","value":{"path":"dir/file","previous":"old","replacement":"new","extensions":null}},{"kind":"terminal_reference","value":{"id":"original-terminal-id","extensions":null}}],"locations":[{"path":"dir/file","line":7,"extensions":{"line":true}}],"arguments":{"presence":"supplied","value":null},"result":{"presence":"supplied","value":{"z":1,"a":[true,null]}},"extensions":{"tool":true},"client_display":{"version":1,"tool":"literal display","arguments":{"b":2,"a":1},"destination":{"presence":"value","value":"file"},"redacted":{"presence":"value","value":false},"outcome":{"presence":"value","value":{"Ok":{"b":2,"a":1}}}}}});
    let attachment = json!({"kind":"local_terminal","id":"original local id"});
    let source=json!({"format":"nocterm.chat","version":2,"document":{"id":ID,"metadata":{"agent_id":"provider","title":"title","name":"name","pinned":true,"updated":17,"model":"model"},"resume":{"session_id":"original-session","workdir":"dir","config":{"z":true,"a":"value"},"mode":"mode","pending_history":true},"transcript":{"entries":[{"kind":"agent","value":"agent"},{"kind":"thought","value":"thought"},{"kind":"user","value":contents},{"kind":"content","value":text},tool],"times":[1,2,3,4,5]},"composer":{"draft":"original draft","queue":[{"id":73,"text":"queued","images":[image],"attachments":[attachment.clone()]}],"attachments":[attachment,{"kind":"connection","id":"connection"},{"kind":"group","id":"group"}]}}}).to_string();
    let expected = nocterm_ai::history::encode(&load(&source).unwrap()).unwrap();
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join(format!("{ID}.json")), source).unwrap();
    let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let completion = super::loading::read(
        directory.path(),
        ID,
        roles.take_reader().unwrap(),
        roles.take_delivery().unwrap(),
    );
    let (_reader, outcome) = completion.into_parts();
    let parts = match outcome {
        ReaderOutcome::Ready(parts) => parts,
        ReaderOutcome::Failed(error) => panic!("native materialization: {}", error.error()),
    };
    let actual = serde_json::to_vec(&SnapshotWire::new(parts.snapshot().unwrap())).unwrap();
    assert_eq!(actual, expected);
    let reloaded = load(std::str::from_utf8(&actual).unwrap()).unwrap();
    assert_eq!(nocterm_ai::history::encode(&reloaded).unwrap(), actual);
    assert_eq!(reloaded.composer().queue[0].id, 73);
    let nocterm_ai::model::content::Content::Media(image) = &reloaded.composer().queue[0].images[0]
    else {
        panic!("full queued image")
    };
    assert_eq!(
        image.encoded.len(),
        nocterm_ai::history::MAX_SAVED_IMAGE_BYTES + 1
    );
}

#[test]
fn repository_owned_load_returns_original_reader_after_cancel_stale_and_failure() {
    use nocterm_ai::{
        history::{ReaderSource, physical::SnapshotWire},
        model::retention::document::DocumentRetention,
    };
    let source = legacy(
        r#"[{"kind":"agent","value":"original repository row"}]"#,
        "",
    );
    let expected = nocterm_ai::history::encode(&load(&source).unwrap()).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(format!("{ID}.json"));
    fs::write(&path, &source).unwrap();
    let authority = RetentionAuthority::new(MAX_RESIDENT_BYTES).unwrap();
    let repository = FileChatRepository::with_retention(directory.path().into(), authority.clone());
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let reader =
        ReaderSource::new(roles.take_reader().unwrap(), roles.take_delivery().unwrap()).unwrap();
    let mut live = roles.take_live().unwrap();
    let unpolled = repository.load_into(ID.into(), reader.capture().unwrap());
    drop(unpolled);
    let stale =
        futures::executor::block_on(repository.load_into(ID.into(), reader.capture().unwrap()));
    assert_eq!(
        serde_json::to_vec(&SnapshotWire::new(stale.snapshot().unwrap())).unwrap(),
        expected
    );
    drop(stale);
    let accepted =
        futures::executor::block_on(repository.load_into(ID.into(), reader.capture().unwrap()));
    assert!(accepted.error().is_none());
    accepted.install_into(&mut live).unwrap();
    fs::write(&path, "invalid source").unwrap();
    let failed =
        futures::executor::block_on(repository.load_into(ID.into(), reader.capture().unwrap()));
    assert!(failed.error().is_some());
    failed.release().unwrap();
    assert_eq!(
        serde_json::to_vec(&SnapshotWire::new(live.snapshot().unwrap())).unwrap(),
        expected
    );
    fs::write(&path, &source).unwrap();
    futures::executor::block_on(repository.load_into(ID.into(), reader.capture().unwrap()))
        .release()
        .unwrap();
    assert!(live.snapshot().is_some());
}

mod read_errors;
