//! Admission behavior through actual controlled payloads and real producer roles.
use super::*;
use crate::{
    history::{
        ChatMetadata, ComposerData, ResumeDescriptor, TranscriptData,
        physical::{RowBody, SnapshotBuilder, SnapshotStaging},
    },
    model::{
        json::deferred::{DecodeLedger, literal::LiteralBuilder},
        native::{NativeList, NativeOwned},
    },
    thread::Entry,
};
pub(crate) fn text(ledger: &DecodeLedger, value: &str) -> NativeOwned<String> {
    let mut literal = LiteralBuilder::with_cleanup_frontier(ledger, 1).unwrap();
    let mut text = literal.text().unwrap();
    text.append(value).unwrap();
    literal.string(text).unwrap();
    NativeOwned::text(literal.finish_native().unwrap().into_text().unwrap())
}
pub(crate) fn snapshot<'a>(role: &'a ConstructionRole, value: &str) -> SnapshotStaging<'a> {
    snapshot_with_ledger(role, value).0
}
fn snapshot_with_ledger<'a>(
    role: &'a ConstructionRole,
    value: &str,
) -> (SnapshotStaging<'a>, DecodeLedger) {
    let ledger = DecodeLedger::for_operation(role, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let chat = native_chat(&ledger, value);
    let stage = SnapshotBuilder::from_document(role, chat.into_document())
        .unwrap()
        .finish(None)
        .unwrap();
    (stage, ledger)
}
fn native_chat(ledger: &DecodeLedger, value: &str) -> crate::history::native::NativeChat {
    let id = text(ledger, "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c");
    let metadata = crate::history::native::document::metadata(
        text(ledger, "agent"),
        NativeOwned::absent(ledger).unwrap(),
        NativeOwned::absent(ledger).unwrap(),
        false,
        7,
        NativeOwned::absent(ledger).unwrap(),
    )
    .unwrap();
    let resume = crate::history::native::document::resume(
        NativeOwned::absent(ledger).unwrap(),
        NativeOwned::absent(ledger).unwrap(),
        crate::history::native::document::empty_choices(ledger).unwrap(),
        NativeOwned::absent(ledger).unwrap(),
        false,
    )
    .unwrap();
    let mut rows = NativeList::<Entry>::new(ledger, 1).unwrap();
    rows.push(crate::history::native::entries::agent(text(ledger, value)))
        .unwrap();
    let times = NativeList::<u64>::new(ledger, 0).unwrap().finish();
    let transcript = crate::history::native::document::transcript(rows.finish(), times).unwrap();
    let composer = crate::history::native::document::composer(
        NativeOwned::absent(ledger).unwrap(),
        NativeList::new(ledger, 0).unwrap().finish(),
        NativeList::new(ledger, 0).unwrap().finish(),
    )
    .unwrap();
    crate::history::native::NativeChat::new(id, metadata, resume, transcript, composer).unwrap()
}

#[test]
fn real_snapshot_aliases_in_independent_producer_roles_count_once() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    let mut active = roles.take_writer_active().unwrap();
    let baseline = scope.retained_and_pending_bytes().unwrap();
    live.replace(snapshot(&operation, "kept literal")).unwrap();
    let one = scope.retained_and_pending_bytes().unwrap();
    active.clone_from_role(&live).unwrap();
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), one);
    live.clear();
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), one);
    let row = &active.snapshot().unwrap().rows()[0];
    let RowBody::Agent(text) = row.view() else {
        panic!("agent row");
    };
    assert_eq!(&*text.read(), "kept literal");
    active.clear();
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), baseline);
}
#[test]
fn cancelling_unpublished_real_payload_returns_common_credit_after_cleanup() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let operation = roles.take_reader().unwrap();
    let baseline = scope.retained_and_pending_bytes().unwrap();
    let staged = snapshot(&operation, "cancelled payload");
    assert!(scope.retained_and_pending_bytes().unwrap() > baseline);
    drop(staged);
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), baseline);
}
#[test]
fn replacing_latest_does_not_release_old_snapshot_still_owned_by_active_writer() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let operation = roles.take_foreground().unwrap();
    let mut latest = roles.take_writer_latest().unwrap();
    let mut active = roles.take_writer_active().unwrap();
    latest.replace(snapshot(&operation, "old literal")).unwrap();
    active.clone_from_role(&latest).unwrap();
    let one = scope.retained_and_pending_bytes().unwrap();
    latest.replace(snapshot(&operation, "new literal")).unwrap();
    assert!(scope.retained_and_pending_bytes().unwrap() > one);
    active.clear();
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), one);
}
#[test]
fn separately_loaded_same_file_identity_has_independent_document_admission() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (left, mut left_roles) = DocumentRetention::new(&authority).unwrap();
    let (right, mut right_roles) = DocumentRetention::new(&authority).unwrap();
    let left_operation = left_roles.take_reader().unwrap();
    let right_operation = right_roles.take_reader().unwrap();
    let mut left_live = left_roles.take_live().unwrap();
    let mut right_live = right_roles.take_live().unwrap();
    left_live
        .replace(snapshot(&left_operation, "left"))
        .unwrap();
    let left_bytes = left.retained_and_pending_bytes().unwrap();
    right_live
        .replace(snapshot(&right_operation, "right"))
        .unwrap();
    assert_eq!(left.retained_and_pending_bytes().unwrap(), left_bytes);
    assert!(!left.same_document(&right));
    assert!(matches!(
        left_live.clone_from_role(&right_live),
        Err(RetentionError::ForeignAuthority)
    ));
    assert_eq!(
        &*right_live.snapshot().unwrap().id(),
        "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c"
    );
}

#[test]
fn a_fork_keeps_real_source_payload_and_weak_headers_after_origin_scope_drops() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (target, mut target_roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = target_roles.take_foreground().unwrap();
    let mut live = target_roles.take_live().unwrap();
    let baseline = target.retained_and_pending_bytes().unwrap();
    let before_origin_drop;
    {
        let (_origin, mut roles) = DocumentRetention::new(&authority).unwrap();
        let source_operation = roles.take_foreground().unwrap();
        let mut source = roles.take_live().unwrap();
        source
            .replace(snapshot(&source_operation, "shared original payload"))
            .unwrap();
        let builder = SnapshotBuilder::from_fork_role(&mut operation, &source).unwrap();
        live.replace(builder.finish(None).unwrap()).unwrap();
        operation.clear_source();
        before_origin_drop = target.retained_and_pending_bytes().unwrap();
    }
    // All originating document capabilities have gone. Its genuine weak header
    // and source phase metadata remain physical children of the forked graph.
    assert_eq!(
        target.retained_and_pending_bytes().unwrap(),
        before_origin_drop
    );
    let RowBody::Agent(text) = live.snapshot().unwrap().rows()[0].view() else {
        panic!("agent row");
    };
    assert_eq!(&*text.read(), "shared original payload");
    live.clear();
    assert_eq!(target.retained_and_pending_bytes().unwrap(), baseline);
}
#[test]
fn closing_a_phase_revokes_old_ledger_while_published_receipts_keep_its_headers() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    let (stage, old_ledger) = snapshot_with_ledger(&operation, "published source");
    live.replace(stage).unwrap();
    operation.close_phase();
    let diagnostic = old_ledger.live_requested_bytes();
    assert!(NativeList::<u64>::new(&old_ledger, 1).is_err());
    assert_eq!(old_ledger.live_requested_bytes(), diagnostic);
    operation.begin_phase().unwrap();
    assert!(NativeList::<u64>::new(&old_ledger, 1).is_err());
    let new_ledger =
        DecodeLedger::for_operation(&operation, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let fresh = NativeList::<u64>::new(&new_ledger, 1).unwrap();
    drop(fresh);
    // Old ledger's concrete metadata is still registered through Live's actual
    // source receipt, not an unregistered historical ledger list.
    assert!(scope.retained_and_pending_bytes().unwrap() > 0);
    drop(old_ledger);
    live.clear();
}
#[test]
fn rejected_preallocation_cleans_marks_and_preserves_existing_actual_roots() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&operation, "untouched after refusal"))
        .unwrap();
    let ledger = DecodeLedger::for_operation(&operation, usize::MAX).unwrap();
    let before = scope.retained_and_pending_bytes().unwrap();
    assert!(NativeList::<u8>::new(&ledger, crate::history::MAX_RESIDENT_BYTES).is_err());
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), before);
    let mut small = NativeList::<u8>::new(&ledger, 4).unwrap();
    small
        .push(NativeOwned::scalar(9, &ledger).unwrap())
        .unwrap();
    drop(small);
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), before);
    let RowBody::Agent(text) = live.snapshot().unwrap().rows()[0].view() else {
        panic!("agent row");
    };
    assert_eq!(&*text.read(), "untouched after refusal");
}
#[test]
fn panicking_borrow_consumer_does_not_poison_closed_payload_slots_or_release_credit() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    let baseline = scope.retained_and_pending_bytes().unwrap();
    live.replace(snapshot(&operation, "read after panic"))
        .unwrap();
    let before = scope.retained_and_pending_bytes().unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let RowBody::Agent(text) = live.snapshot().unwrap().rows()[0].view() else {
            panic!("agent row");
        };
        let _borrow = text.read();
        panic!("external read consumer");
    }));
    assert!(result.is_err());
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), before);
    let RowBody::Agent(text) = live.snapshot().unwrap().rows()[0].view() else {
        panic!("agent row");
    };
    assert_eq!(&*text.read(), "read after panic");
    live.clear();
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), baseline);
}

fn agent_ptr(row: &crate::history::physical::RowSlot) -> *const u8 {
    let RowBody::Agent(text) = row.view() else {
        panic!("agent row")
    };
    text.read().as_ptr()
}

fn assert_saved_window_matches_legacy_encoding(
    authority: &RetentionAuthority,
    snapshot: &crate::history::physical::SnapshotOwner,
) {
    let original_document = crate::history::SavedChat::from_parts(
        authority,
        "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c".into(),
        ChatMetadata {
            agent_id: "agent".into(),
            title: None,
            name: None,
            pinned: false,
            updated: 7,
            model: None,
        },
        ResumeDescriptor::default(),
        TranscriptData {
            entries: std::iter::once(Entry::Agent("original payload".into()))
                .chain((1..=2005).map(|_| Entry::Agent("new payload".into())))
                .collect(),
            times: (0..=2005).collect(),
        },
        ComposerData::default(),
    )
    .unwrap();
    let encoded =
        serde_json::to_vec(&crate::history::physical::SnapshotWire::new(snapshot)).unwrap();
    assert_eq!(encoded, crate::history::encode(&original_document).unwrap());
    let bounded: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    let transcript = &bounded["document"]["transcript"];
    assert_eq!(transcript["entries"].as_array().unwrap().len(), 2000);
    assert_eq!(transcript["times"][0], 6);
}

#[test]
fn live_row_changes_share_payloads_and_keep_all_rows_until_the_actual_owner_releases_them() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    let mut active = roles.take_writer_active().unwrap();
    live.replace(snapshot(&operation, "original payload"))
        .unwrap();
    active.clone_from_role(&live).unwrap();
    let original = agent_ptr(&active.snapshot().unwrap().rows()[0]);
    let ledger =
        DecodeLedger::for_operation(&operation, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    for now in 1..=2005 {
        builder
            .append_row(
                crate::history::native::entries::agent(text(&ledger, "new payload")),
                now,
            )
            .unwrap();
    }
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    let snapshot = live.snapshot().unwrap();
    assert_eq!(snapshot.rows().len(), 2006);
    assert_eq!(snapshot.times().len(), 2006);
    assert_eq!(snapshot.times()[0], 0);
    assert_eq!(snapshot.times()[2005], 2005);
    assert_saved_window_matches_legacy_encoding(&authority, snapshot);
    assert_eq!(snapshot.rows().len(), 2006);
    assert_eq!(agent_ptr(&snapshot.rows()[0]), original);
    active.clone_from_role(&live).unwrap();
    let full = scope.retained_and_pending_bytes().unwrap();
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    assert!(
        builder
            .replace_row(
                0,
                crate::history::native::entries::agent(text(&ledger, "replacement"))
            )
            .unwrap()
    );
    assert!(builder.truncate_rows(1).unwrap());
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    assert_eq!(live.snapshot().unwrap().rows().len(), 1);
    assert_eq!(active.snapshot().unwrap().rows().len(), 2006);
    assert_eq!(agent_ptr(&active.snapshot().unwrap().rows()[0]), original);
    assert!(scope.retained_and_pending_bytes().unwrap() >= full);
    active.clear();
    assert!(scope.retained_and_pending_bytes().unwrap() < full);
    drop(ledger);
    live.clear();
}

pub(crate) fn tool_row(ledger: &DecodeLedger, id: &str) -> NativeOwned<Entry> {
    use crate::model::tools::{ToolKind, ToolStatus};
    let header = crate::model::native::tools::header(
        crate::model::native::tools::tool_id(text(ledger, id)).unwrap(),
        text(ledger, "Read file"),
        NativeOwned::supplied(text(ledger, "read")),
        ToolKind::Read,
        ToolStatus::Running,
    )
    .unwrap();
    crate::history::native::entries::tool_entry(
        crate::model::native::tools::record(
            header,
            NativeList::new(ledger, 0).unwrap().finish(),
            NativeList::new(ledger, 0).unwrap().finish(),
            NativeOwned::absent(ledger).unwrap(),
            NativeOwned::absent(ledger).unwrap(),
            NativeOwned::absent(ledger).unwrap(),
            NativeOwned::absent(ledger).unwrap(),
        )
        .unwrap(),
    )
}

pub(crate) fn user_text(
    ledger: &DecodeLedger,
    value: &str,
) -> NativeOwned<crate::model::content::Content> {
    crate::model::native::content::from_text(
        crate::model::native::content::text_content(
            text(ledger, value),
            NativeOwned::absent(ledger).unwrap(),
            NativeOwned::absent(ledger).unwrap(),
        )
        .unwrap(),
    )
}

fn user_items(row: &crate::history::physical::RowSlot) -> Vec<(*const u8, String)> {
    let RowBody::User(user) = row.view() else {
        panic!("user row")
    };
    user.contents()
        .map(|item| match item {
            crate::history::physical::ContentSlot::Text(text) => {
                (text.text().as_ptr(), text.text().to_string())
            }
            _ => panic!("text content"),
        })
        .collect()
}

#[test]
fn user_chunks_extend_the_final_user_row_and_share_earlier_items() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    let mut active = roles.take_writer_active().unwrap();
    live.replace(snapshot(&operation, "original payload"))
        .unwrap();
    let ledger =
        DecodeLedger::for_operation(&operation, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder
        .append_user_content(user_text(&ledger, "first"), 11)
        .unwrap();
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    active.clone_from_role(&live).unwrap();
    let before = user_items(&active.snapshot().unwrap().rows()[1]);

    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder
        .append_user_content(user_text(&ledger, "second"), 12)
        .unwrap();
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    let live_snapshot = live.snapshot().unwrap();
    assert_eq!(live_snapshot.rows().len(), 2);
    assert_eq!(live_snapshot.times()[1], 11);
    let after = user_items(&live_snapshot.rows()[1]);
    assert_eq!(after.len(), 2);
    assert_eq!(after[0], before[0]);
    assert_eq!(after[1].1, "second");
    assert_eq!(user_items(&active.snapshot().unwrap().rows()[1]).len(), 1);

    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder
        .append_row(
            crate::history::native::entries::agent(text(&ledger, "reply")),
            13,
        )
        .unwrap();
    builder
        .append_user_content(user_text(&ledger, "third"), 14)
        .unwrap();
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    let live_snapshot = live.snapshot().unwrap();
    assert_eq!(live_snapshot.rows().len(), 4);
    assert_eq!(live_snapshot.times()[3], 14);
    assert_eq!(user_items(&live_snapshot.rows()[1]), after);
    assert_eq!(user_items(&live_snapshot.rows()[3])[0].1, "third");
    active.clear();
    drop(ledger);
    live.clear();
}

pub(crate) fn agent_texts(rows: &[crate::history::physical::RowSlot]) -> Vec<(*const u8, String)> {
    rows.iter()
        .map(|row| match row.view() {
            RowBody::Agent(text) => (text.read().as_ptr(), format!("agent:{}", &*text.read())),
            RowBody::Thought(text) => (text.read().as_ptr(), format!("thought:{}", &*text.read())),
            _ => (std::ptr::null(), "other".into()),
        })
        .collect()
}

#[test]
fn agent_text_chunks_concatenate_only_the_final_matching_row() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&operation, "original payload"))
        .unwrap();
    let ledger =
        DecodeLedger::for_operation(&operation, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    assert!(
        builder
            .append_agent_text(true, text(&ledger, "think"), 20)
            .unwrap()
    );
    assert!(
        builder
            .append_agent_text(false, text(&ledger, "Hel"), 21)
            .unwrap()
    );
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    let before = agent_texts(live.snapshot().unwrap().rows());
    assert_eq!(before[1].1, "thought:think");
    assert_eq!(before[2].1, "agent:Hel");

    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    assert!(
        !builder
            .append_agent_text(false, text(&ledger, ""), 22)
            .unwrap()
    );
    assert!(
        builder
            .append_agent_text(false, text(&ledger, "lo"), 22)
            .unwrap()
    );
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    let snapshot_after = live.snapshot().unwrap();
    let after = agent_texts(snapshot_after.rows());
    assert_eq!(after.len(), 3);
    assert_eq!(after[0], before[0]);
    assert_eq!(after[1], before[1]);
    assert_eq!(after[2].1, "agent:Hello");
    assert_eq!(snapshot_after.times()[2], 21);
    drop(ledger);
    live.clear();
}

pub(crate) fn tool_of(
    row: &crate::history::physical::RowSlot,
) -> &crate::history::physical::ToolSlot {
    match row.view() {
        RowBody::Tool(tool) => tool,
        _ => panic!("tool row"),
    }
}

#[test]
fn tool_status_patch_shares_every_payload_and_title_patch_replaces_only_the_title() {
    use crate::model::tools::{ToolKind, ToolStatus};
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    let mut active = roles.take_writer_active().unwrap();
    live.replace(snapshot(&operation, "original payload"))
        .unwrap();
    let ledger =
        DecodeLedger::for_operation(&operation, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder.append_row(tool_row(&ledger, "call-1"), 1).unwrap();
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    active.clone_from_role(&live).unwrap();
    let before = tool_of(&active.snapshot().unwrap().rows()[1]);
    let (title, name) = (before.title().as_ptr(), before.name().unwrap().as_ptr());

    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    let id = crate::model::native::tools::tool_id(text(&ledger, "call-1")).unwrap();
    assert!(
        builder
            .patch_tool(crate::model::native::patch::tool_patch(
                id,
                None,
                Some(ToolStatus::Succeeded)
            ))
            .unwrap()
    );
    let unknown = crate::model::native::tools::tool_id(text(&ledger, "call-2")).unwrap();
    assert!(
        !builder
            .patch_tool(crate::model::native::patch::tool_patch(
                unknown,
                Some(ToolKind::Edit),
                None
            ))
            .unwrap()
    );
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    let after = tool_of(&live.snapshot().unwrap().rows()[1]);
    assert_eq!(after.status(), ToolStatus::Succeeded);
    assert_eq!(after.kind(), ToolKind::Read);
    assert_eq!(after.title().as_ptr(), title);
    assert_eq!(after.name().unwrap().as_ptr(), name);
    assert_eq!(
        tool_of(&active.snapshot().unwrap().rows()[1]).status(),
        ToolStatus::Running
    );

    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    let id = crate::model::native::tools::tool_id(text(&ledger, "call-1")).unwrap();
    let patch = crate::model::native::patch::with_title(
        crate::model::native::patch::tool_patch(id, None, None),
        text(&ledger, "Read README.md"),
    )
    .unwrap();
    assert!(builder.patch_tool(patch).unwrap());
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    let titled = tool_of(&live.snapshot().unwrap().rows()[1]);
    assert_eq!(&*titled.title(), "Read README.md");
    assert_eq!(titled.name().unwrap().as_ptr(), name);
    assert_eq!(titled.status(), ToolStatus::Succeeded);
    assert_eq!(
        &*tool_of(&active.snapshot().unwrap().rows()[1]).title(),
        "Read file"
    );
    active.clear();
    drop(ledger);
    live.clear();
}

pub(crate) fn display(
    ledger: &DecodeLedger,
    tool: &str,
) -> NativeOwned<crate::model::tools::ClientDisplay> {
    use crate::model::controls::Presence;
    crate::model::native::tools::display(
        1,
        text(ledger, tool),
        crate::model::native::content::null_json(ledger).unwrap(),
        NativeOwned::not_supplied(ledger).unwrap(),
        Presence::Absent,
        NativeOwned::not_supplied(ledger).unwrap(),
    )
    .unwrap()
}

#[test]
fn finishing_pending_tools_fails_them_in_one_pass_and_display_keeps_payloads_shared() {
    use crate::model::tools::ToolStatus;
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut operation = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&operation, "original payload"))
        .unwrap();
    let ledger =
        DecodeLedger::for_operation(&operation, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    builder.append_row(tool_row(&ledger, "call-1"), 1).unwrap();
    builder.append_row(tool_row(&ledger, "call-2"), 2).unwrap();
    let id = crate::model::native::tools::tool_id(text(&ledger, "call-2")).unwrap();
    assert!(
        builder
            .patch_tool(crate::model::native::patch::tool_patch(
                id,
                None,
                Some(ToolStatus::Succeeded)
            ))
            .unwrap()
    );
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    let snapshot_before = live.snapshot().unwrap();
    let first_title = tool_of(&snapshot_before.rows()[1]).title().as_ptr();
    let second_title = tool_of(&snapshot_before.rows()[2]).title().as_ptr();

    let mut builder = SnapshotBuilder::from_role(&mut operation, &live).unwrap();
    assert!(builder.patch_display(1, display(&ledger, "read")).unwrap());
    assert!(!builder.patch_display(1, display(&ledger, "read")).unwrap());
    assert!(!builder.patch_display(0, display(&ledger, "read")).unwrap());
    assert_eq!(builder.finish_pending_tools().unwrap().as_slice(), &[1]);
    assert!(
        builder
            .finish_pending_tools()
            .unwrap()
            .as_slice()
            .is_empty()
    );
    live.replace(builder.finish(None).unwrap()).unwrap();
    operation.clear_source();
    let after = live.snapshot().unwrap();
    let first = tool_of(&after.rows()[1]);
    assert_eq!(first.status(), ToolStatus::Failed);
    assert_eq!(first.title().as_ptr(), first_title);
    assert_eq!(first.client_display().unwrap().tool, "read");
    let second = tool_of(&after.rows()[2]);
    assert_eq!(second.status(), ToolStatus::Succeeded);
    assert_eq!(second.title().as_ptr(), second_title);
    drop(ledger);
    live.clear();
}

#[test]
fn managed_delivery_returns_original_capability_and_preserves_it_on_foreign_installation() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let (_, mut foreign_roles) = DocumentRetention::new(&authority).unwrap();
    let reader = roles.take_reader().unwrap();
    let mut delivery = roles.take_delivery().unwrap();
    let mut live = roles.take_live().unwrap();
    let mut foreign = foreign_roles.take_live().unwrap();
    delivery.replace(snapshot(&reader, "delivered")).unwrap();
    let parts = crate::history::physical::ManagedParts::from_role(delivery);
    let failed = match parts.install_into(&mut foreign) {
        Ok(_) => panic!("foreign scope must refuse installation"),
        Err(error) => error,
    };
    assert!(foreign.snapshot().is_none());
    let (parts, error) = failed.into_parts();
    assert!(matches!(error, RetentionError::ForeignAuthority));
    assert_eq!(
        &*parts.snapshot().unwrap().id(),
        "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c"
    );
    let mut delivery = parts.install_into(&mut live).unwrap();
    assert!(delivery.snapshot().is_none());
    assert!(delivery.scope().same_document(&scope));
    delivery
        .replace(snapshot(&reader, "next delivery"))
        .unwrap();
    assert!(delivery.snapshot().is_some());
    assert_eq!(
        &*live.snapshot().unwrap().id(),
        "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c"
    );
}

#[test]
fn native_reader_failure_cancels_real_payload_and_returns_the_same_reusable_capabilities() {
    use crate::{
        history::{NativePhase, ReaderOutcome},
        storage::AgentStorageError,
    };
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let reader = roles.take_reader().unwrap();
    let delivery = roles.take_delivery().unwrap();
    let baseline = scope.retained_and_pending_bytes().unwrap();
    let phase = match NativePhase::begin(reader, delivery) {
        Ok(phase) => phase,
        Err(_) => panic!("reader phase"),
    };
    let completion = phase.read(|ledger| {
        let pending = native_chat(ledger, "cancel this real native payload");
        assert!(scope.retained_and_pending_bytes().unwrap() > baseline);
        drop(pending);
        Err(AgentStorageError::Interrupted)
    });
    let (reader, outcome) = completion.into_parts();
    let ReaderOutcome::Failed(failed) = outcome else {
        panic!("failed native read");
    };
    let (delivery, _) = failed.into_parts();
    assert!(delivery.snapshot().is_none());
    assert!(delivery.scope().same_document(&scope));
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), baseline);
    let phase = match NativePhase::begin(reader, delivery) {
        Ok(phase) => phase,
        Err(_) => panic!("same reader restarts"),
    };
    let completion = phase.fail(AgentStorageError::Interrupted);
    assert!(matches!(completion.outcome(), ReaderOutcome::Failed(_)));
    assert_eq!(scope.retained_and_pending_bytes().unwrap(), baseline);
}

#[test]
fn native_reader_publishes_delivery_before_phase_closes_and_reuses_original_empty_role() {
    use crate::history::{HistoryOrigin, NativeMaterialized, NativePhase, ReaderOutcome};
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let reader = roles.take_reader().unwrap();
    let delivery = roles.take_delivery().unwrap();
    let mut live = roles.take_live().unwrap();
    let phase = match NativePhase::begin(reader, delivery) {
        Ok(phase) => phase,
        Err(_) => panic!("reader phase"),
    };
    let completion = phase.read(|ledger| {
        Ok(NativeMaterialized::new(
            native_chat(ledger, "published native payload"),
            HistoryOrigin::Version2,
            None,
            0,
        ))
    });
    let (reader, outcome) = completion.into_parts();
    let ReaderOutcome::Ready(parts) = outcome else {
        panic!("native delivery");
    };
    assert_eq!(
        &*parts.snapshot().unwrap().id(),
        "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c"
    );
    let delivery = parts.install_into(&mut live).unwrap();
    assert!(delivery.snapshot().is_none());
    assert!(delivery.scope().same_document(&scope));
    match live.snapshot().unwrap().rows()[0].view() {
        RowBody::Agent(text) => assert_eq!(&*text.read(), "published native payload"),
        _ => panic!("agent row"),
    }
    let phase = match NativePhase::begin(reader, delivery) {
        Ok(phase) => phase,
        Err(_) => panic!("same reader can start while prior Live owns payload"),
    };
    let completion = phase.fail(crate::storage::AgentStorageError::Interrupted);
    assert!(matches!(completion.outcome(), ReaderOutcome::Failed(_)));
    assert_eq!(live.snapshot().unwrap().rows().len(), 1);
}

mod capability_lifetimes;
