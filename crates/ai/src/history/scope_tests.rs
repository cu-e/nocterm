//! Transcript ownership is explicit in scoped formats and absent in legacy bytes.
use super::*;
use serde_json::{Value, json};

const ID: &str = "00000000-0000-4000-8000-0000000000a1";
const PROJECT: &str = "01010101010101010101010101010101";
const LEGACY: &[u8] = include_bytes!("fixtures/v1/canonical.json");

fn scoped(scope: Value) -> Value {
    let mut chat = decode(LEGACY, ID).unwrap();
    chat.version = 2;
    let mut value = serde_json::to_value(chat).unwrap();
    value["catalog"]["scope"] = scope.clone();
    value["chat"]["scope"] = scope;
    value
}

fn project() -> Value {
    json!({"kind": "project", "id": PROJECT})
}

#[test]
fn explicit_general_and_project_ownership_survive_every_transcript_projection() {
    for scope in [json!({"kind": "general"}), project()] {
        let bytes = serde_json::to_vec(&scoped(scope.clone())).unwrap();
        let chat = decode(&bytes, ID).unwrap();
        let encoded: Value = serde_json::from_slice(&encode(&chat).unwrap()).unwrap();
        assert_eq!(encoded["version"], 2);
        assert_eq!(encoded["catalog"]["scope"], scope);
        let mut metadata = chat;
        let queue = std::mem::take(&mut metadata.queue)
            .into_iter()
            .map(std::sync::Arc::new)
            .collect();
        let shared = SharedChat { metadata, queue };
        let encoded: Value = serde_json::from_slice(&encode_shared(&shared).unwrap()).unwrap();
        assert_eq!(encoded["catalog"]["scope"], scope);
        let restored =
            prepare(decode(&serde_json::to_vec(&encoded).unwrap(), ID).unwrap()).unwrap();
        let restored: Value = serde_json::from_slice(&encode(&restored).unwrap()).unwrap();
        assert_eq!(restored["catalog"]["scope"], scope);
    }
}

#[test]
fn new_chats_explicitly_belong_to_general_until_a_project_is_supplied() {
    let chat = SavedChat::for_identity("agent".into(), ID.into(), 1);
    let value: Value = serde_json::from_slice(&encode(&chat).unwrap()).unwrap();
    assert_eq!(value["version"], 2);
    assert_eq!(value["catalog"]["scope"], json!({"kind": "general"}));
}

#[test]
fn scoped_transcripts_refuse_missing_unknown_and_malformed_ownership() {
    let mut missing = scoped(project());
    missing["chat"].as_object_mut().unwrap().remove("scope");
    for value in [
        missing,
        scoped(Value::Null),
        scoped(json!({"kind": "other"})),
        scoped(json!({"kind": "project"})),
        scoped(json!({"kind": "project", "id": "not-a-project-id"})),
        scoped(json!({"kind": "general", "id": PROJECT})),
    ] {
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(decode(&bytes, ID).is_err(), "accepted {value}");
        let stamp = FileStamp {
            bytes: bytes.len() as u64,
            modified_nanos: 1,
        };
        assert!(
            decode_summary(&bytes, ID, stamp).is_err(),
            "catalog accepted {value}"
        );
    }
}

#[test]
fn legacy_version_cannot_carry_a_project_ownership_claim() {
    let mut value: Value = serde_json::from_slice(LEGACY).unwrap();
    value["scope"] = project();
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(decode(&bytes, ID).is_err());
    let stamp = FileStamp {
        bytes: bytes.len() as u64,
        modified_nanos: 1,
    };
    assert!(decode_summary(&bytes, ID, stamp).is_err());
}

#[test]
fn scoped_metadata_cache_carries_ownership_and_rejects_legacy_cache_versions() {
    let bytes = serde_json::to_vec(&scoped(project())).unwrap();
    let stamp = FileStamp {
        bytes: bytes.len() as u64,
        modified_nanos: 19,
    };
    let summary = decode_summary(&bytes, ID, stamp).unwrap().unwrap();
    assert!(summary.matches_stamp(ID, stamp));
    let mut value = serde_json::to_value(&summary).unwrap();
    assert_eq!(value["scope"], project());
    assert_eq!(value["cache_version"], 3);
    value["cache_version"] = json!(1);
    let old: ChatSummary = serde_json::from_value(value).unwrap();
    assert!(!old.matches_stamp(ID, stamp));
}

#[test]
fn general_legacy_catalog_ownership_does_not_rewrite_authoritative_bytes() {
    let stamp = FileStamp {
        bytes: LEGACY.len() as u64,
        modified_nanos: 5,
    };
    let summary = decode_summary(LEGACY, ID, stamp).unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(summary).unwrap()["scope"],
        json!({"kind": "general"})
    );
    assert_eq!(encode(&decode(LEGACY, ID).unwrap()).unwrap(), LEGACY);
}

#[test]
fn ownership_refusals_are_typed_and_invalid_scope_retains_its_json_cause() {
    let mut missing = scoped(project());
    missing["chat"].as_object_mut().unwrap().remove("scope");
    assert!(matches!(
        decode(&serde_json::to_vec(&missing).unwrap(), ID),
        Err(HistoryCodecError::MissingScope)
    ));
    let invalid = scoped(json!({"kind": "project", "id": "invalid"}));
    let error = decode(&serde_json::to_vec(&invalid).unwrap(), ID).unwrap_err();
    assert!(matches!(error, HistoryCodecError::Json(_)));
    let cloned = error.clone();
    fn original(error: &HistoryCodecError) -> &serde_json::Error {
        let mut cause = std::error::Error::source(error);
        while let Some(current) = cause {
            if let Some(json) = current.downcast_ref::<serde_json::Error>() {
                return json;
            }
            cause = current.source();
        }
        panic!("the original JSON error must remain in the source chain")
    }
    assert!(std::ptr::eq(original(&error), original(&cloned)));
}

#[test]
fn direct_serde_cannot_invent_missing_ownership_or_drop_a_legacy_project_claim() {
    let mut missing = scoped(project());
    missing["chat"].as_object_mut().unwrap().remove("scope");
    assert!(serde_json::from_value::<SavedChat>(missing).is_err());
    let mut chat = decode(&serde_json::to_vec(&scoped(project())).unwrap(), ID).unwrap();
    chat.version = 1;
    assert!(matches!(encode(&chat), Err(HistoryCodecError::LegacyScope)));
    assert!(serde_json::to_vec(&chat).is_err());
}

#[test]
fn scoped_documents_start_with_an_authoritative_catalog_before_the_payload() {
    let chat = decode(&serde_json::to_vec(&scoped(project())).unwrap(), ID).unwrap();
    let value: Value = serde_json::from_slice(&encode(&chat).unwrap()).unwrap();
    assert_eq!(value["catalog"]["id"], ID);
    assert_eq!(value["catalog"]["scope"], project());
    assert_eq!(value["catalog"]["title"], "Disk usage");
    assert_eq!(value["catalog"]["has_content"], true);
    assert!(value["chat"]["entries"].is_array());
}

#[test]
fn catalog_and_payload_ownership_must_agree_before_hydration() {
    let chat = decode(&serde_json::to_vec(&scoped(project())).unwrap(), ID).unwrap();
    let mut value: Value = serde_json::from_slice(&encode(&chat).unwrap()).unwrap();
    value["catalog"]["scope"] = json!({"kind": "general"});
    assert!(decode(&serde_json::to_vec(&value).unwrap(), ID).is_err());
}

#[test]
fn a_prefix_is_sufficient_without_a_payload_and_rejects_noncanonical_or_oversized_headers() {
    let chat = decode(LEGACY, ID).unwrap();
    let mut chat = chat;
    chat.version = 2;
    let bytes = encode(&chat).unwrap();
    let marker = b",\"chat\":";
    let end = bytes
        .windows(marker.len())
        .position(|part| part == marker)
        .unwrap()
        + marker.len();
    let header = decode_catalog_header(&bytes[..end], ID).unwrap();
    assert_eq!(header.scope(), ContextId::General);
    assert!(decode(&bytes[..end], ID).is_err());
    assert!(decode_catalog_header(&bytes[..end - 1], ID).is_err());
    let mut spaced = bytes.clone();
    spaced.insert(1, b' ');
    assert!(decode_catalog_header(&spaced, ID).is_err());
    let mut oversized = b"{\"version\":2,\"catalog\":{\"id\":\"".to_vec();
    oversized.extend(vec![b'x'; MAX_HEADER_BYTES]);
    assert!(matches!(
        decode_catalog_header(&oversized, ID),
        Err(HistoryCodecError::HeaderLimit)
    ));
    assert!(decode_catalog_header(&bytes, "00000000-0000-4000-8000-000000000002").is_err());
}
#[test]
fn escaped_multibyte_metadata_fits_the_header_bound_and_round_trips() {
    let mut chat = SavedChat::for_identity("\u{0}".repeat(512), ID.into(), 1);
    chat.header.title = Some("\u{1}".repeat(512));
    chat.header.name = Some("\u{2}".repeat(512));
    chat.model = Some("😀".repeat(512));
    let bytes = encode(&chat).unwrap();
    assert!(decode_catalog_header(&bytes[..bytes.len().min(MAX_HEADER_BYTES)], ID).is_ok());
    let restored = decode(&bytes, ID).unwrap();
    assert_eq!(restored.header.title, chat.header.title);
    assert_eq!(restored.header.name, chat.header.name);
    assert_eq!(restored.model, chat.model);
}

#[test]
fn oversized_user_image_replacements_and_raw_serde_have_matching_catalog_titles() {
    use crate::{
        model::content::{Content, Image},
        thread::Entry,
    };
    let mut chat = SavedChat::for_identity("agent".into(), ID.into(), 1);
    chat.entries.push(Entry::User(vec![
        Content::Image(Image::new(
            "A".repeat(MAX_SAVED_IMAGE_BYTES + 1),
            "image/png",
        )),
        Content::text("literal question after the image"),
    ]));
    let raw = serde_json::to_vec(&chat).unwrap();
    let decoded = decode(&raw, ID).unwrap();
    let raw_summary = decode_catalog_header(&raw, ID)
        .unwrap()
        .summary(FileStamp {
            bytes: raw.len() as u64,
            modified_nanos: 0,
        })
        .unwrap();
    assert_eq!(raw_summary.title(), "literal question after the image");
    assert!(
        matches!(&decoded.entries[0], Entry::User(blocks) if matches!(blocks[0], Content::Image(_)))
    );
    let bytes = encode(&chat).unwrap();
    let decoded = decode(&bytes, ID).unwrap();
    let summary = decode_catalog_header(&bytes, ID)
        .unwrap()
        .summary(FileStamp {
            bytes: bytes.len() as u64,
            modified_nanos: 0,
        })
        .unwrap();
    assert_eq!(summary.title(), "[Image not kept in history]");
    assert_eq!(ChatSummary::from_chat(&chat).title(), summary.title());
    assert!(
        matches!(&decoded.entries[0], Entry::User(blocks) if matches!(&blocks[0], Content::Text(text) if text.text == "[Image not kept in history]"))
    );
    assert_eq!(
        encode_shared(&SharedChat {
            metadata: chat,
            queue: Vec::new()
        })
        .unwrap(),
        bytes
    );
}

#[test]
fn an_image_only_chat_can_reopen_after_its_large_image_is_omitted() {
    use crate::{
        model::content::{Content, Image},
        thread::Entry,
    };
    let mut chat = SavedChat::for_identity("agent".into(), ID.into(), 1);
    chat.entries
        .push(Entry::User(vec![Content::Image(Image::new(
            "A".repeat(MAX_SAVED_IMAGE_BYTES + 1),
            "image/png",
        ))]));
    assert!(decode(&serde_json::to_vec(&chat).unwrap(), ID).is_ok());
    let bytes = encode(&chat).unwrap();
    let restored = decode(&bytes, ID).unwrap();
    assert_eq!(
        ChatSummary::from_chat(&restored).title(),
        "[Image not kept in history]"
    );
    let stamp = FileStamp {
        bytes: bytes.len() as u64,
        modified_nanos: 0,
    };
    assert_eq!(
        decode_summary(&bytes, ID, stamp).unwrap().unwrap().title(),
        "[Image not kept in history]"
    );
}
#[test]
fn catalog_rows_preserve_the_transcript_version_independently_of_cache_schema() {
    let id = "00000000-0000-0000-0000-000000000001";
    let bytes = br#"{"version":1,"id":"00000000-0000-0000-0000-000000000001","agent_id":"codex","updated":1,"draft":"legacy","entries":[]}"#;
    let legacy = decode_summary(
        bytes,
        id,
        FileStamp {
            bytes: bytes.len() as u64,
            modified_nanos: 0,
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(legacy.transcript_version().wire(), 1);
    let current = ChatSummary::from_chat(&SavedChat::for_identity("codex".into(), id.into(), 1));
    assert_eq!(current.transcript_version().wire(), 2);
    let mut changed = serde_json::to_value(current).unwrap();
    assert_eq!(changed["cache_version"], 3);
    changed["transcript_version"] = serde_json::json!(99);
    assert!(serde_json::from_value::<ChatSummary>(changed).is_err());
}
