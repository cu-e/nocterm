//! Public repository publication retains original writer loans and wire semantics.
use crate::FileChatRepository;
use nocterm_ai::{
    history::{
        self, ReaderSource,
        physical::SnapshotWire,
        writing::{ManagedWriteIntent, WriterProducer},
    },
    model::retention::{RetentionAuthority, document::DocumentRetention},
    storage::ChatRepository,
};
const ID: &str = "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c";
fn source() -> Vec<u8> {
    format!(r#"{{"version":1,"id":"{ID}","agent_id":"original","updated":17,"draft":"original draft","entries":[{{"kind":"agent","value":"original row"}},{{"kind":"tool","value":{{"toolCallId":"original-tool","title":"original tool","_meta":{{"before":1,"nocterm/tool-display":{{"arguments":null,"tool":"old","version":2,"redacted":false}},"after":2}}}}}}],"times":[11,12],"queue":[{{"id":7,"text":"queued","images":[],"attachments":[]}}]}}"#).into_bytes()
}
#[test]
fn managed_repository_publication_preserves_borrowed_v2_and_original_capabilities() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(format!("{ID}.json"));
    let source = source();
    std::fs::write(&path, &source).unwrap();
    let authority = RetentionAuthority::new(history::MAX_RESIDENT_BYTES).unwrap();
    let repository = FileChatRepository::with_retention(directory.path().into(), authority.clone());
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let reader =
        ReaderSource::new(roles.take_reader().unwrap(), roles.take_delivery().unwrap()).unwrap();
    let mut live = roles.take_live().unwrap();
    futures::executor::block_on(repository.load_into(ID.into(), reader.capture().unwrap()))
        .install_into(&mut live)
        .unwrap();
    let original = crate::legacy::decode(&authority, &source, ID).unwrap();
    let snapshot = live.snapshot().unwrap();
    let prepared = snapshot.prepared();
    let grammar = prepared.as_ref().unwrap().legacy().unwrap();
    let expected_legacy = grammar.measure(original.view().unwrap()).unwrap();
    assert_eq!(grammar.managed_measure(snapshot).unwrap(), expected_legacy);
    let legacy_decision = history::EncodingDecision {
        encoding: history::HistoryEncoding::LegacyV1,
        size: expected_legacy,
    };
    assert_eq!(
        serde_json::to_vec(
            &crate::legacy::projection_managed(snapshot, legacy_decision, grammar.as_ref())
                .unwrap()
        )
        .unwrap(),
        serde_json::to_vec(
            &crate::legacy::projection(
                original.view().unwrap(),
                legacy_decision,
                original.prepared().unwrap().legacy().unwrap().as_ref()
            )
            .unwrap()
        )
        .unwrap()
    );
    drop(prepared);
    let expected = serde_json::to_vec(&SnapshotWire::new(snapshot)).unwrap();
    let writer = WriterProducer::new(
        roles.take_writer_latest().unwrap(),
        roles.take_writer_active().unwrap(),
        roles.take_writer().unwrap(),
    )
    .unwrap();
    let latest = writer
        .latest(&live, None, ManagedWriteIntent::Snapshot)
        .unwrap();
    let cancelled = repository.save_owned(latest.capture().unwrap());
    drop(cancelled);
    assert_eq!(std::fs::read(&path).unwrap(), source);
    futures::executor::block_on(repository.save_owned(latest.capture().unwrap())).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), expected);
    let reload =
        futures::executor::block_on(repository.load_into(ID.into(), reader.capture().unwrap()));
    assert_eq!(
        serde_json::to_vec(&SnapshotWire::new(reload.snapshot().unwrap())).unwrap(),
        expected
    );
    reload.release().unwrap();
    futures::executor::block_on(repository.save_owned(latest.capture().unwrap())).unwrap();
}

#[test]
fn managed_archive_draft_keeps_untouched_v1_and_merges_only_actual_draft() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(format!("{ID}.json"));
    let source = source();
    std::fs::write(&path, &source).unwrap();
    let authority = RetentionAuthority::new(history::MAX_RESIDENT_BYTES).unwrap();
    let repository = FileChatRepository::with_retention(directory.path().into(), authority.clone());
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let reader =
        ReaderSource::new(roles.take_reader().unwrap(), roles.take_delivery().unwrap()).unwrap();
    let mut live = roles.take_live().unwrap();
    futures::executor::block_on(repository.load_into(ID.into(), reader.capture().unwrap()))
        .install_into(&mut live)
        .unwrap();
    let writer = WriterProducer::new(
        roles.take_writer_latest().unwrap(),
        roles.take_writer_active().unwrap(),
        roles.take_writer().unwrap(),
    )
    .unwrap();
    let latest = writer
        .latest(
            &live,
            None,
            ManagedWriteIntent::ArchiveDraft { updated: 99 },
        )
        .unwrap();
    futures::executor::block_on(repository.save_owned(latest.capture().unwrap())).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), source);
    let disk = String::from_utf8(source)
        .unwrap()
        .replace("original draft", "disk draft")
        .replace("original row", "disk row");
    std::fs::write(&path, disk.as_bytes()).unwrap();
    futures::executor::block_on(repository.save_owned(latest.capture().unwrap())).unwrap();
    let result =
        futures::executor::block_on(repository.load_into(ID.into(), reader.capture().unwrap()));
    let snapshot = result.snapshot().unwrap();
    assert_eq!(
        snapshot.composer().draft().as_deref(),
        Some("original draft")
    );
    assert_eq!(snapshot.metadata().updated(), 99);
    assert_eq!(snapshot.times(), &[11, 12]);
    let nocterm_ai::history::physical::RowBody::Agent(text) = snapshot.rows()[0].view() else {
        panic!("original disk row kind")
    };
    assert_eq!(&*text.read(), "disk row");
    assert_eq!(snapshot.composer().queue()[0].id(), 7);
    result.release().unwrap();
}

#[test]
fn managed_legacy_projection_matches_all_borrowed_content_and_full_queued_images() {
    use serde_json::json;
    let annotation = json!({"audience":["user","assistant"],"lastModified":"original","priority":0.5,"_meta":{"z":1,"a":2}});
    let image = json!({"type":"image","data":"x".repeat(history::MAX_SAVED_IMAGE_BYTES+1),"mimeType":"image/png","uri":"original"});
    let contents = json!([
        {"type":"text","text":"literal λ","annotations":annotation,"_meta":{"z":1,"a":2}},
        image,
        {"type":"audio","data":"full audio","mimeType":"audio/wav"},
        {"type":"resource_link","name":"name","uri":"uri","description":"description","size":42,"title":"title","mimeType":"text/plain"},
        {"type":"resource","resource":{"mimeType":"text/plain","text":"body","uri":"uri","_meta":{"future":true}}},
        {"type":"resource","resource":{"blob":"encoded","uri":"uri"}}
    ]);
    let source = serde_json::to_vec(&json!({"version":1,"id":ID,"agent_id":"original","updated":17,"entries":[{"kind":"user","value":contents}],"queue":[{"id":7,"text":"queued","images":[image],"attachments":[{"kind":"group","id":"original-group"}]}]})).unwrap();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join(format!("{ID}.json")), &source).unwrap();
    let authority = RetentionAuthority::new(history::MAX_RESIDENT_BYTES).unwrap();
    let original = crate::legacy::decode(&authority, &source, ID).unwrap();
    let grammar = original.prepared().unwrap().legacy().unwrap();
    let decision = history::EncodingDecision {
        encoding: history::HistoryEncoding::LegacyV1,
        size: grammar.measure(original.view().unwrap()).unwrap(),
    };
    let expected = serde_json::to_vec(
        &crate::legacy::projection(original.view().unwrap(), decision, grammar.as_ref()).unwrap(),
    )
    .unwrap();
    let repository = FileChatRepository::with_retention(directory.path().into(), authority.clone());
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let reader =
        ReaderSource::new(roles.take_reader().unwrap(), roles.take_delivery().unwrap()).unwrap();
    let delivery =
        futures::executor::block_on(repository.load_into(ID.into(), reader.capture().unwrap()));
    let snapshot = delivery.snapshot().unwrap();
    let prepared = snapshot.prepared();
    let grammar = prepared.as_ref().unwrap().legacy().unwrap();
    assert_eq!(grammar.managed_measure(snapshot).unwrap(), decision.size);
    let actual = serde_json::to_vec(
        &crate::legacy::projection_managed(snapshot, decision, grammar.as_ref()).unwrap(),
    )
    .unwrap();
    assert_eq!(actual, expected);
    let value: serde_json::Value = serde_json::from_slice(&actual).unwrap();
    assert_eq!(
        value["entries"][0]["value"][1]["text"],
        "[Image not kept in history]"
    );
    assert_eq!(
        value["queue"][0]["images"][0]["data"]
            .as_str()
            .unwrap()
            .len(),
        history::MAX_SAVED_IMAGE_BYTES + 1
    );
}

#[test]
fn imported_live_edits_and_saved_candidate_keep_original_legacy_provenance() {
    use nocterm_ai::{
        history::live::{LiveEpoch, LiveOwner},
        model::ids::SessionId,
        session::SessionState,
    };
    let directory = tempfile::tempdir().unwrap();
    let source = source();
    std::fs::write(directory.path().join(format!("{ID}.json")), &source).unwrap();
    let authority = RetentionAuthority::new(history::MAX_RESIDENT_BYTES).unwrap();
    let original = crate::legacy::decode(&authority, &source, ID).unwrap();
    let repository = FileChatRepository::with_retention(directory.path().into(), authority.clone());
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let reader =
        ReaderSource::new(roles.take_reader().unwrap(), roles.take_delivery().unwrap()).unwrap();
    let mut owner = LiveOwner::new(
        roles.take_live().unwrap(),
        roles.take_foreground().unwrap(),
        roles.take_prompt_dispatch().unwrap(),
    )
    .unwrap();
    owner
        .install_delivery(futures::executor::block_on(
            repository.load_into(ID.into(), reader.capture().unwrap()),
        ))
        .unwrap();
    let epoch = LiveEpoch::new(
        SessionState::default().ticket(),
        1,
        SessionId::new("original-session"),
    );
    owner.accept_epoch(epoch.clone());
    owner
        .set_title(owner.admission(&epoch).unwrap(), Some("changed title"))
        .unwrap();
    owner
        .set_draft(owner.admission(&epoch).unwrap(), Some("changed draft"))
        .unwrap();
    let mut candidate = roles.take_candidate().unwrap();
    owner
        .prepare_saved_candidate(owner.admission(&epoch).unwrap(), &mut candidate)
        .unwrap();
    let original_grammar = original.prepared().unwrap().legacy().unwrap().clone();
    let modified = crate::history_fixtures::edited(original, |metadata, _, _, composer| {
        metadata.title = Some("changed title".into());
        composer.draft = Some("changed draft".into());
    });
    assert_edited_legacy(candidate.snapshot().unwrap(), &modified, &original_grammar);
    let writer = WriterProducer::new(
        roles.take_writer_latest().unwrap(),
        roles.take_writer_active().unwrap(),
        roles.take_writer().unwrap(),
    )
    .unwrap();
    let latest = writer
        .latest(&candidate, None, ManagedWriteIntent::Snapshot)
        .unwrap();
    futures::executor::block_on(repository.save_owned(latest.capture().unwrap())).unwrap();
    let summaries = futures::executor::block_on(repository.catalog()).unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].title.as_deref(), Some("changed title"));
    assert!(directory.path().join(format!("{ID}.meta")).is_file());
}

fn assert_edited_legacy(
    snapshot: &history::physical::SnapshotOwner,
    expected: &history::SavedChat,
    expected_grammar: &history::retention::LegacyFormat,
) {
    let prepared = snapshot.prepared();
    assert_eq!(
        prepared.as_ref().unwrap().origin(),
        history::HistoryOrigin::LegacyV1
    );
    let grammar = prepared.as_ref().unwrap().legacy().unwrap();
    let decision = history::EncodingDecision {
        encoding: history::HistoryEncoding::LegacyV1,
        size: expected_grammar.measure(expected.view().unwrap()).unwrap(),
    };
    let actual = serde_json::to_vec(
        &crate::legacy::projection_managed(snapshot, decision, grammar.as_ref()).unwrap(),
    )
    .unwrap();
    let expected = serde_json::to_vec(
        &crate::legacy::projection(
            expected.view().unwrap(),
            decision,
            expected_grammar.as_ref(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(actual, expected);
    assert!(String::from_utf8(actual).unwrap().contains(r#""_meta":{"before":1,"nocterm/tool-display":{"arguments":null,"tool":"old","version":2,"redacted":false},"after":2}"#));
}
