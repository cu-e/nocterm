//! Version-one saved chat files stay readable and byte-stable.
use super::*;
use serde_json::{Value, json};

const ID: &str = "00000000-0000-4000-8000-0000000000a1";
const CANONICAL: &[u8] = include_bytes!("fixtures/v1/canonical.json");
const LENIENT_LEGACY: &[u8] = include_bytes!("fixtures/v1/lenient_legacy.json");

fn value(chat: &SavedChat) -> Value {
    serde_json::from_slice(&encode(chat).unwrap()).unwrap()
}

#[test]
fn a_canonical_saved_chat_reencodes_byte_for_byte() {
    let chat = decode(CANONICAL, ID).unwrap();
    assert_eq!(encode(&chat).unwrap(), CANONICAL);
}

#[test]
fn a_canonical_saved_chat_keeps_every_field() {
    let chat = decode(CANONICAL, ID).unwrap();
    assert_eq!(chat.agent_id, "codex");
    assert_eq!(chat.title.as_deref(), Some("Disk usage"));
    assert_eq!(chat.name.as_deref(), Some("Disks"));
    assert!(chat.pinned);
    assert_eq!(chat.session_id.as_deref(), Some("session-1"));
    assert_eq!(chat.model.as_deref(), Some("opus"));
    assert_eq!(chat.mode.as_deref(), Some("plan"));
    assert!(chat.pending_history);
    assert_eq!(chat.draft.as_deref(), Some("unsent \n text"));
    assert_eq!(chat.updated, 1_700_000_000);
    assert_eq!(chat.entries.len(), 6);
    assert_eq!(chat.times, [10, 11, 12, 13, 14, 15]);
    assert_eq!(chat.queue.len(), 1);
    assert_eq!(chat.queue[0].images.len(), 1);
    assert_eq!(
        chat.attachments,
        [
            SavedAttachment::Connection("prod".into()),
            SavedAttachment::Group("web".into()),
            SavedAttachment::LocalTerminal("shell".into()),
        ]
    );
    assert!(chat.config.contains_key("model"));
}

#[test]
fn metadata_survives_on_every_saved_protocol_object() {
    let saved = value(&decode(CANONICAL, ID).unwrap());
    let entries = &saved["entries"];
    assert_eq!(entries[0]["value"][0]["_meta"]["origin"], "composer");
    assert_eq!(entries[0]["value"][0]["annotations"]["_meta"]["a"], 1);
    assert_eq!(entries[0]["value"][1]["_meta"]["image"], true);
    assert_eq!(entries[0]["value"][2]["_meta"]["link"], true);
    assert_eq!(entries[0]["value"][3]["_meta"]["resource"], true);
    assert_eq!(entries[5]["value"]["_meta"]["tool"], "meta");
    assert_eq!(entries[5]["value"]["content"][0]["_meta"]["item"], 1);
    assert_eq!(entries[5]["value"]["locations"][0]["_meta"]["loc"], 1);
}

#[test]
fn lenient_legacy_files_are_normalized_when_saved_again() {
    let chat = decode(LENIENT_LEGACY, ID).unwrap();
    let saved = value(&chat);
    let user = &saved["entries"][0]["value"][0];
    assert_eq!(user["text"], "legacy question");
    assert!(
        user.get("annotations").is_none(),
        "invalid annotations kept"
    );
    assert!(user.get("_meta").is_none(), "invalid _meta kept");
    let tool = &saved["entries"][1]["value"];
    assert!(tool.get("kind").is_none(), "unknown kind must become other");
    assert!(
        tool.get("status").is_none(),
        "unknown status must be pending"
    );
    assert!(tool.get("rawInput").is_none());
    assert_eq!(tool["rawOutput"], json!({"exit": 0}));
    assert_eq!(tool["content"].as_array().unwrap().len(), 1);
    assert_eq!(tool["content"][0]["content"]["text"], "kept item");
    assert!(tool.get("locations").is_none());
    assert_eq!(chat.queue[0].images.len(), 1);
    assert_eq!(
        encode(&decode(&encode(&chat).unwrap(), ID).unwrap()).unwrap(),
        encode(&chat).unwrap()
    );
}

#[test]
fn images_over_the_saved_limit_are_replaced_when_saved() {
    let mut chat = decode(CANONICAL, ID).unwrap();
    let big = "A".repeat(MAX_SAVED_IMAGE_BYTES + 1);
    let image: acp::ContentBlock =
        serde_json::from_value(json!({"type": "image", "data": big, "mimeType": "image/png"}))
            .unwrap();
    chat.entries = vec![Entry::User(vec![image.clone()]), Entry::Content(image)];
    chat.times.clear();
    let saved = value(&chat);
    for entry in saved["entries"].as_array().unwrap() {
        let block = if entry["kind"] == "user" {
            &entry["value"][0]
        } else {
            &entry["value"]
        };
        assert_eq!(block["type"], "text");
        assert_eq!(block["text"], "[Image not kept in history]");
    }
}

#[test]
fn queued_prompt_images_stay_complete_when_saved() {
    let chat = decode(CANONICAL, ID).unwrap();
    let saved = value(&chat);
    assert_eq!(saved["queue"][0]["images"][0]["type"], "image");
    assert_eq!(saved["queue"][0]["attachments"][0]["kind"], "group");
}

#[test]
fn summaries_agree_with_the_fully_decoded_chat() {
    let stamp = FileStamp {
        bytes: CANONICAL.len() as u64,
        modified_nanos: 7,
    };
    let summary = decode_summary(CANONICAL, ID, stamp).unwrap().unwrap();
    let chat = decode(CANONICAL, ID).unwrap();
    let full = ChatSummary::from_chat(&chat).with_stamp(stamp, false);
    assert_eq!(summary.title(), full.title());
    assert_eq!(summary.updated, full.updated);
    assert_eq!(summary.has_content, full.has_content);
    assert_eq!(summary.pinned, full.pinned);

    let stamp = FileStamp {
        bytes: LENIENT_LEGACY.len() as u64,
        modified_nanos: 8,
    };
    let summary = decode_summary(LENIENT_LEGACY, ID, stamp).unwrap().unwrap();
    let chat = decode(LENIENT_LEGACY, ID).unwrap();
    assert_eq!(summary.title(), ChatSummary::from_chat(&chat).title());
    assert_eq!(summary.title(), "legacy question");
}

#[test]
fn other_versions_and_identities_are_rejected() {
    let mut other: Value = serde_json::from_slice(CANONICAL).unwrap();
    other["version"] = json!(2);
    let bytes = serde_json::to_vec(&other).unwrap();
    assert!(matches!(
        decode(&bytes, ID),
        Err(HistoryCodecError::UnsupportedVersion(2))
    ));
    assert!(matches!(
        decode(CANONICAL, "00000000-0000-4000-8000-0000000000a2"),
        Err(HistoryCodecError::IdentityMismatch)
    ));
}
