//! Semantic transitions, capture offers and publication acknowledgements are distinct.
use super::super::{Composer, config::ChatConfig};
use super::*;
use nocterm_ai::model::content::Content;
use nocterm_ai::model::controls::Change;
use nocterm_ai::model::plan::Plan;
use nocterm_ai::model::session::{SessionInfo, SessionUpdate};
use nocterm_ai::model::tools::{ToolChanges, ToolPatch, ToolRecord, ToolStatus};
use nocterm_ai::model::usage::ContextUsage;
use serde_json::json;
const ID: &str = "00000000-0000-4000-8000-000000000001";
fn imported(entries: Vec<Entry>) -> ChatDocument {
    let mut saved = SavedChat::for_identity("agent".into(), ID.into(), 40);
    saved.entries = entries;
    saved.title = Some("Recorded title".into());
    let mut document = ChatDocument::new("agent".into(), ID.into(), 40);
    document.imported_clean(&mut saved, false);
    document
}
fn input<'a>(composer: &'a Composer, config: &'a ChatConfig) -> Inputs<'a> {
    Inputs::new(composer, config, Vec::new(), Resume::default())
}
/// Builds the neutral update from the JSON an agent would send.
fn update(value: serde_json::Value) -> SessionUpdate {
    fn field<T: serde::de::DeserializeOwned>(value: &serde_json::Value, name: &str) -> Option<T> {
        value
            .get(name)
            .map(|v| serde_json::from_value(v.clone()).unwrap())
    }
    let content = || field::<Content>(&value, "content").unwrap();
    match value["sessionUpdate"].as_str().unwrap() {
        "agent_message_chunk" => SessionUpdate::AgentMessageChunk(content()),
        "agent_thought_chunk" => SessionUpdate::AgentThoughtChunk(content()),
        "tool_call_update" => {
            let changes = ToolChanges {
                status: field(&value, "status"),
                title: field(&value, "title"),
                raw_output: field(&value, "rawOutput"),
                ..ToolChanges::default()
            };
            SessionUpdate::ToolCallUpdate(Box::new(ToolPatch::new(
                field::<String>(&value, "toolCallId").unwrap(),
                changes,
            )))
        }
        "plan" => SessionUpdate::Plan(Plan {
            entries: Vec::new(),
        }),
        "session_info_update" => SessionUpdate::SessionInfoUpdate(SessionInfo {
            title: Change::Set(value["title"].as_str().unwrap().into()),
            meta: None,
        }),
        "usage_update" => SessionUpdate::UsageUpdate(ContextUsage::new(
            field(&value, "used").unwrap(),
            field(&value, "size").unwrap(),
        )),
        other => panic!("unsupported test update {other}"),
    }
}
fn failure() -> crate::runtime::AgentPersistenceError {
    nocterm_store::StoreError::Closing.into()
}

#[test]
fn imported_browse_projection_equal_metadata_and_capture_do_not_advance_revision() {
    let composer = Composer::default();
    let config = ChatConfig::default();
    let inputs = input(&composer, &config);
    let mut document = imported(vec![Entry::Agent("Saved answer".into())]);
    document.establish_clean_inputs(&inputs);
    assert!(document.snapshot(&inputs).is_some());
    assert!(!document.rename(None, 100));
    assert!(!document.pin(false, 100));
    assert!(!document.accept_inputs(&inputs, 100, false));
    assert!(document.propose_candidate(&inputs).is_none());
    assert_eq!(document.desired_revision(), 0);
    assert_eq!(document.updated(), 40);
    assert!(document.clean());
}

#[test]
fn empty_matching_chunks_identical_tool_patches_and_view_updates_are_clean() {
    let call = ToolRecord::new("call", "Tool")
        .status(ToolStatus::Pending)
        .raw_output(json!({"out":"same"}));
    let mut document = imported(vec![Entry::Tool(call), Entry::Agent("answer".into())]);
    document.apply_transcript(
        update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":""}})),
        80,
    );
    document.apply_transcript(update(json!({"sessionUpdate":"tool_call_update","toolCallId":"call","status":"pending","title":"Tool","rawOutput":{"out":"same"}})),80);
    document.apply_transcript(update(json!({"sessionUpdate":"plan","entries":[]})), 80);
    document.apply_transcript(
        update(json!({"sessionUpdate":"usage_update","used":1,"size":10})),
        80,
    );
    document.apply_transcript(
        update(json!({"sessionUpdate":"session_info_update","title":"Recorded title"})),
        80,
    );
    document.tokens(None);
    assert_eq!(document.desired_revision(), 0);
    assert_eq!(document.updated(), 40);
}

#[test]
fn a_new_empty_row_is_durable_but_a_patch_outside_the_saved_window_is_not() {
    let mut entries = vec![Entry::Tool(ToolRecord::new("old", "Old"))];
    entries.extend((0..2000).map(|_| Entry::Agent("saved".into())));
    let mut document = imported(entries);
    document.apply_transcript(update(json!({"sessionUpdate":"tool_call_update","toolCallId":"old","title":"Changed old row"})),80);
    assert_eq!(document.desired_revision(), 0);
    document.apply_transcript(
        update(json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":""}})),
        81,
    );
    assert_eq!(document.desired_revision(), 1);
    assert_eq!(document.updated(), 81);
}

#[test]
fn genuine_saved_row_transitions_remain_dirty_even_when_the_text_returns_to_its_old_value() {
    let mut document = imported(vec![Entry::Tool(ToolRecord::new("call", "A"))]);
    for (title, time) in [("B", 81), ("A", 82)] {
        document.apply_transcript(
            update(json!({"sessionUpdate":"tool_call_update","toolCallId":"call","title":title})),
            time,
        );
    }
    assert_eq!(document.desired_revision(), 2);
    assert_eq!(document.updated(), 82);
    assert!(!document.clean());
}

#[test]
fn repeated_capture_and_failed_retry_share_the_original_revision_and_snapshot() {
    let composer = Composer::default();
    let config = ChatConfig::default();
    let inputs = input(&composer, &config);
    let mut document = imported(vec![Entry::Agent("saved".into())]);
    document.establish_clean_inputs(&inputs);
    document.rename(Some("changed".into()), 81);
    let (first, revision) = document.propose_candidate(&inputs).unwrap();
    let (again, same) = document.propose_candidate(&inputs).unwrap();
    assert_eq!(revision, same);
    assert!(Arc::ptr_eq(&first, &again));
    assert!(document.mark_normal_offer(revision));
    assert!(!document.mark_normal_offer(revision));
    assert!(
        document.existing_offer().is_some(),
        "scheduling is not admission"
    );
    document.acknowledge_admission(revision);
    assert!(document.propose_candidate(&inputs).is_none());
    assert!(document.failed(revision, failure()));
    let (retry, version) = document.propose_candidate(&inputs).unwrap();
    assert_eq!(version, revision);
    assert!(Arc::ptr_eq(&first, &retry));
    assert_eq!(document.updated(), 81);
}

#[test]
fn old_failure_before_new_admission_and_old_success_cannot_settle_newer_desired_state() {
    let composer = Composer::default();
    let config = ChatConfig::default();
    let inputs = input(&composer, &config);
    let mut document = imported(vec![Entry::Agent("saved".into())]);
    document.establish_clean_inputs(&inputs);
    document.rename(Some("first".into()), 81);
    let (_, old) = document.propose_candidate(&inputs).unwrap();
    document.acknowledge_admission(old);
    document.rename(Some("new desired".into()), 82);
    assert!(!document.failed(old, failure()));
    assert!(document.persistence_error().is_none());
    let (_, new) = document.propose_candidate(&inputs).unwrap();
    assert!(document.failed(new, failure()));
    assert!(!document.published(old));
    assert!(document.persistence_error().is_some());
    assert!(!document.clean());
    assert!(document.published(new));
    assert!(document.persistence_error().is_none());
}

#[test]
fn clearing_a_saved_draft_yields_an_empty_candidate_and_a_never_saved_empty_chat_does_not() {
    let mut composer = Composer::restored(Some("draft".into()), Vec::new(), Vec::new());
    let config = ChatConfig::default();
    let mut document = imported(Vec::new());
    document.establish_clean_inputs(&input(&composer, &config));
    assert!(composer.set_draft(String::new()));
    document.accept_inputs(&input(&composer, &config), 81, true);
    let (snapshot, _) = document
        .propose_candidate(&input(&composer, &config))
        .unwrap();
    assert!(snapshot.draft.is_none());
    assert!(snapshot.entries.is_empty());
    let mut empty = ChatDocument::new("agent".into(), ID.into(), 40);
    empty.establish_clean_inputs(&input(&composer, &config));
    assert!(
        empty
            .propose_candidate(&input(&composer, &config))
            .is_none()
    );
}

#[test]
fn hydration_overlay_and_restart_keep_the_existing_semantic_revision() {
    let composer = Composer::default();
    let config = ChatConfig::default();
    let inputs = input(&composer, &config);
    let mut document = imported(Vec::new());
    document.establish_clean_inputs(&inputs);
    document.edited_archived_draft(81);
    let mut loaded = SavedChat::for_identity("agent".into(), ID.into(), 40);
    loaded.entries = vec![Entry::Agent("preserved".into())];
    document.imported_with_overlay(&mut loaded, false, 81);
    document.establish_clean_inputs(&inputs);
    assert_eq!(document.desired_revision(), 1);
    assert_eq!(document.updated(), 81);
    let (_, revision) = document.propose_candidate(&inputs).unwrap();
    document.acknowledge_admission(revision);
    let mut replacement = ChatDocument::restarted(document.restart_snapshot());
    assert_eq!(replacement.chat_id(), ID);
    assert_eq!(replacement.desired_revision(), revision);
    assert!(
        replacement.propose_candidate(&inputs).is_some(),
        "the replacement must hand off pending intent under its own owner"
    );
}

#[test]
fn metadata_normalization_changes_only_its_actual_key_and_only_when_different() {
    let mut document = imported(vec![Entry::Tool(
        ToolRecord::new("call", "Tool").meta(nocterm_ai::model::extensions::Extensions::from(
            json!({"other":{"large":"preserved"}})
                .as_object()
                .unwrap()
                .clone(),
        )),
    )]);
    document.patch_tool_rows(vec![ToolMetadataPatch::new(0, json!({"version":2}))], 81);
    document.patch_tool_rows(vec![ToolMetadataPatch::new(0, json!({"version":2}))], 82);
    assert_eq!(document.desired_revision(), 1);
    assert_eq!(document.updated(), 81);
    let Entry::Tool(call) = &document.entries()[0] else {
        panic!()
    };
    assert_eq!(
        call.meta.as_ref().unwrap()["other"],
        json!({"large":"preserved"})
    );
}

#[test]
fn admission_of_an_old_offer_does_not_consume_the_new_literal_draft_change() {
    let mut composer = Composer::default();
    let config = ChatConfig::default();
    let mut document = imported(vec![Entry::Agent("saved".into())]);
    document.establish_clean_inputs(&input(&composer, &config));
    document.rename(Some("old offered name".into()), 81);
    let (_, old) = document
        .propose_candidate(&input(&composer, &config))
        .unwrap();
    composer.set_draft("new literal draft".into());
    document.accept_inputs(&input(&composer, &config), 82, true);
    document.acknowledge_admission(old);
    assert!(document.draft_changed());
    assert!(!document.clean());
    assert!(!document.evict_clean());
    let (new, revision) = document
        .propose_candidate(&input(&composer, &config))
        .unwrap();
    assert!(revision > old);
    assert_eq!(new.draft.as_deref(), Some("new literal draft"));
}

#[test]
fn disabled_release_retains_unadmitted_or_failed_input_and_an_old_ack_cannot_release_new_input() {
    let composer = Composer::default();
    let config = ChatConfig::default();
    let inputs = input(&composer, &config);
    let mut document = imported(vec![Entry::Agent("retained answer".into())]);
    document.establish_clean_inputs(&inputs);
    document.rename(Some("first change".into()), 81);
    let (first, old) = document.propose_candidate(&inputs).unwrap();
    assert!(!document.release_disabled());
    assert!(document.failed(old, failure()));
    assert!(!document.release_disabled());
    assert_eq!(document.entries().len(), 1);
    document.acknowledge_admission(old);
    document.rename(Some("newer desire".into()), 82);
    document.acknowledge_admission(old);
    assert!(!document.release_disabled());
    let (newest, current) = document.propose_candidate(&inputs).unwrap();
    document.acknowledge_admission(current);
    assert!(document.release_disabled());
    assert!(document.entries().is_empty());
    assert_eq!(
        newest.entries.len(),
        1,
        "the driver retains the full accepted transcript"
    );
    assert_eq!(first.name.as_deref(), Some("first change"));
    assert_eq!(newest.name.as_deref(), Some("newer desire"));
    assert!(
        document.existing_offer().is_none(),
        "the writer already owns this immutable intent"
    );
    assert!(document.failed(current, failure()));
    let (retry, revision) = document.propose_candidate(&inputs).unwrap();
    assert_eq!(revision, current);
    assert!(
        Arc::ptr_eq(&retry, &newest),
        "failure retains the original input after releasing presentation"
    );
}
