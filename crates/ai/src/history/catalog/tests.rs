use super::*;
use crate::{
    history::{ComposerData, SavedPrompt, TranscriptData},
    model::content::{Content, Text},
    thread::Entry,
};

#[test]
fn cached_structural_estimates_cover_preflight_and_pending_only_rows() {
    let authority =
        crate::model::retention::RetentionAuthority::new(crate::history::MAX_RESIDENT_BYTES)
            .unwrap();
    let empty = SavedChat::for_identity(
        &authority,
        "codex".into(),
        "00000000-0000-4000-8000-000000000066".into(),
        1,
    );
    let (id, metadata, resume, _, _, _) = empty.into_parts();
    let chat = SavedChat::from_parts(
        &authority,
        id,
        metadata,
        resume,
        TranscriptData {
            entries: vec![Entry::User(vec![Content::Text(Text {
                text: "first message".into(),
                annotations: None,
                extensions: None,
            })])],
            times: vec![1],
        },
        ComposerData {
            queue: vec![SavedPrompt {
                id: 1,
                text: "界\n\"".into(),
                images: vec![],
                attachments: vec![],
            }],
            ..ComposerData::default()
        },
    )
    .unwrap();
    let wire = super::super::wire::Wire::new(chat.view().unwrap());
    let bytes = serde_json::to_vec(&wire).unwrap();
    let preflight = super::super::preflight::check(&bytes).unwrap();
    let measured = super::super::size::measure(&wire).unwrap();
    assert_eq!(measured.structural_bytes, preflight);
    assert_eq!(measured.encoded_bytes, bytes.len());
    assert!(measured.structural_bytes >= bytes.len());
}

#[test]
fn cached_resident_policy_adds_each_rows_larger_shape_before_unique_census() {
    let authority =
        crate::model::retention::RetentionAuthority::new(crate::history::MAX_RESIDENT_BYTES)
            .unwrap();
    let mut reserved = String::with_capacity(1024 * 1024);
    reserved.push('x');
    let rows = vec![Entry::Agent(reserved), Entry::Agent("y".repeat(512 * 1024))];
    let expected = rows
        .iter()
        .map(|row| {
            super::super::size::retained(row)
                .unwrap()
                .max(super::super::allocations::row(row).unwrap())
        })
        .sum::<usize>();
    let chat = SavedChat::from_parts(
        &authority,
        "00000000-0000-4000-8000-000000000067".into(),
        crate::history::ChatMetadata {
            agent_id: "literal".into(),
            title: None,
            name: None,
            pinned: false,
            updated: 0,
            model: None,
        },
        crate::history::ResumeDescriptor::default(),
        TranscriptData {
            entries: rows,
            times: Vec::new(),
        },
        ComposerData::default(),
    )
    .unwrap();
    let former_global_max = super::super::size::retained(&chat)
        .unwrap()
        .max(super::super::allocations::chat(&chat).unwrap());
    assert!(expected > former_global_max);
    let prepared = crate::history::prepare(chat).unwrap();
    assert!(prepared.resident_bytes() >= expected);
}
