//! Live transitions through the actual Live role and real native payloads.
use super::*;
use crate::{
    history::physical::RowBody,
    model::{
        json::deferred::DecodeLedger,
        retention::{
            RetentionAuthority,
            document::{
                DocumentRetention,
                tests::{agent_texts, snapshot, text, tool_of, tool_row, user_text},
            },
        },
        tools::ToolStatus,
    },
    session::SessionState,
};

struct Fixture {
    owner: LiveOwner,
    ledger: DecodeLedger,
    epoch: LiveEpoch,
    _scope: DocumentRetention,
    _reader: ConstructionRole,
    candidate: RetainedRole,
}
fn fixture() -> Fixture {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let foreground = roles.take_foreground().unwrap();
    let reader = roles.take_reader().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&reader, "seed")).unwrap();
    let ledger =
        DecodeLedger::for_operation(&foreground, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let mut owner =
        LiveOwner::new(live, foreground, roles.take_prompt_dispatch().unwrap()).unwrap();
    let epoch = LiveEpoch::new(SessionState::default().ticket(), 1, SessionId::new("s-1"));
    owner.accept_epoch(epoch.clone());
    Fixture {
        owner,
        ledger,
        epoch,
        _scope: scope,
        _reader: reader,
        candidate: roles.take_candidate().unwrap(),
    }
}
fn applied(routed: LiveRouted) -> AppliedUpdate {
    match routed {
        LiveRouted::Applied(applied) => applied,
        LiveRouted::Forward(_) => panic!("transcript update was forwarded"),
    }
}

#[test]
fn reader_delivery_installs_live_and_recycles_original_capabilities() {
    use crate::history::{HistoryOrigin, NativeMaterialized, ReaderSource, native::NativeChat};
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let source =
        ReaderSource::new(roles.take_reader().unwrap(), roles.take_delivery().unwrap()).unwrap();
    let mut owner = LiveOwner::new(
        roles.take_live().unwrap(),
        roles.take_foreground().unwrap(),
        roles.take_prompt_dispatch().unwrap(),
    )
    .unwrap();
    let delivery = source.capture().unwrap().read(|ledger| {
        let chat = crate::history::SavedChat::for_identity(
            ledger.authority(),
            "agent".into(),
            "00000000-0000-0000-0000-000000000001".into(),
            1,
        );
        Ok(NativeMaterialized::new(
            NativeChat::from_fixture(chat, ledger).unwrap(),
            HistoryOrigin::Version2,
            None,
            0,
        ))
    });
    assert!(delivery.error().is_none(), "{:?}", delivery.error());
    owner.install_delivery(delivery).unwrap();
    assert!(owner.composer().is_some());
    assert_eq!(owner.revision(), 1);
    drop(source.capture().unwrap());
}

#[test]
fn failed_reader_delivery_recycles_capabilities_without_changing_live() {
    use crate::{history::ReaderSource, storage::AgentStorageError};
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let source =
        ReaderSource::new(roles.take_reader().unwrap(), roles.take_delivery().unwrap()).unwrap();
    let mut owner = LiveOwner::new(
        roles.take_live().unwrap(),
        roles.take_foreground().unwrap(),
        roles.take_prompt_dispatch().unwrap(),
    )
    .unwrap();
    let delivery = source
        .capture()
        .unwrap()
        .read(|_| Err(AgentStorageError::Interrupted));
    drop(owner.install_delivery(delivery).unwrap_err());
    assert!(owner.composer().is_none());
    assert_eq!(owner.revision(), 0);
    let delivery = source
        .capture()
        .unwrap()
        .read(|_| Err(AgentStorageError::Interrupted));
    drop(delivery);
    assert!(source.capture().is_ok());
}

#[test]
fn foreign_reader_delivery_returns_its_original_payload_for_retry() {
    use crate::history::{HistoryOrigin, NativeMaterialized, ReaderSource, native::NativeChat};
    let mut f = fixture();
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let source =
        ReaderSource::new(roles.take_reader().unwrap(), roles.take_delivery().unwrap()).unwrap();
    let delivery = source.capture().unwrap().read(|ledger| {
        let chat = crate::history::SavedChat::for_identity(
            ledger.authority(),
            "agent".into(),
            "00000000-0000-0000-0000-000000000001".into(),
            1,
        );
        Ok(NativeMaterialized::new(
            NativeChat::from_fixture(chat, ledger).unwrap(),
            HistoryOrigin::Version2,
            None,
            0,
        ))
    });
    assert!(delivery.error().is_none());
    let pointer = delivery.snapshot().unwrap().id().as_ptr();
    let before = agent_texts(f.owner.rows());
    let revision = f.owner.revision();
    let error = f.owner.install_delivery(delivery).unwrap_err();
    assert_eq!(f.owner.revision(), revision);
    assert_eq!(agent_texts(f.owner.rows()), before);
    let (delivery, _) = error.into_parts();
    assert_eq!(delivery.snapshot().unwrap().id().as_ptr(), pointer);
    assert!(source.capture().is_err());
    let mut destination = roles.take_live().unwrap();
    delivery.install_into(&mut destination).unwrap();
    assert_eq!(destination.snapshot().unwrap().id().as_ptr(), pointer);
    assert!(source.capture().is_ok());
}

#[test]
fn agent_and_thought_chunks_extend_only_the_final_matching_row() {
    let mut f = fixture();
    let chunk = |f: &Fixture, value: &str| user_text(&f.ledger, value);
    for (value, thought) in [("a", false), ("b", false), ("t", true), ("c", false)] {
        let update = if thought {
            crate::model::native::notice::thought_chunk(chunk(&f, value))
        } else {
            crate::model::native::notice::agent_chunk(chunk(&f, value))
        };
        let admission = f.owner.admission(&f.epoch).unwrap();
        assert!(applied(f.owner.apply_owned(admission, update, 5).unwrap()).saved_changed());
    }
    let texts: Vec<_> = agent_texts(f.owner.rows())
        .into_iter()
        .map(|(_, s)| s)
        .collect();
    assert_eq!(texts, ["agent:seedab", "thought:t", "agent:c"]);
    assert_eq!(f.owner.revision(), 4);
}

#[test]
fn stale_epoch_or_revision_is_rejected_without_publishing() {
    let mut f = fixture();
    let stale = f.owner.admission(&f.epoch).unwrap();
    let admission = f.owner.admission(&f.epoch).unwrap();
    let update = crate::model::native::notice::agent_chunk(user_text(&f.ledger, "x"));
    let _ = f.owner.apply_owned(admission, update, 1).unwrap();
    let rows = f.owner.rows().len();
    let update = crate::model::native::notice::agent_chunk(user_text(&f.ledger, "late"));
    assert!(matches!(
        f.owner.apply_owned(stale, update, 2),
        Err(LiveError::Stale)
    ));
    let other = LiveEpoch::new(f.epoch.ticket(), 2, SessionId::new("s-1"));
    assert!(matches!(f.owner.admission(&other), Err(LiveError::Stale)));
    assert_eq!(f.owner.rows().len(), rows);
    assert_eq!(f.owner.revision(), 1);
}

#[test]
fn tool_lifecycle_patches_the_first_matching_row_and_finishes_pending_tools_once() {
    let mut f = fixture();
    let started = |f: &Fixture| {
        let (entry, allocation) = tool_row(&f.ledger, "call-1").into_parts();
        let Entry::Tool(record) = entry else {
            unreachable!()
        };
        crate::model::native::notice::tool_started(NativeOwned::from_parts(record, allocation))
    };
    for _ in 0..2 {
        let admission = f.owner.admission(&f.epoch).unwrap();
        let _ = f.owner.apply_owned(admission, started(&f), 3).unwrap();
    }
    let id = crate::model::native::tools::tool_id(text(&f.ledger, "call-1")).unwrap();
    let patch = crate::model::native::notice::tool_changed(
        crate::model::native::patch::tool_patch(id, None, Some(ToolStatus::Succeeded)),
    );
    let admission = f.owner.admission(&f.epoch).unwrap();
    assert!(applied(f.owner.apply_owned(admission, patch, 4).unwrap()).saved_changed());
    // Parity with the transcript reducer: the first row with the id is patched.
    assert_eq!(tool_of(&f.owner.rows()[1]).status(), ToolStatus::Succeeded);
    assert_eq!(tool_of(&f.owner.rows()[2]).status(), ToolStatus::Running);

    let unknown = crate::model::native::tools::tool_id(text(&f.ledger, "nope")).unwrap();
    let patch = crate::model::native::notice::tool_changed(
        crate::model::native::patch::tool_patch(unknown, None, Some(ToolStatus::Failed)),
    );
    let admission = f.owner.admission(&f.epoch).unwrap();
    let revision = f.owner.revision();
    assert!(!applied(f.owner.apply_owned(admission, patch, 5).unwrap()).saved_changed());
    assert_eq!(f.owner.revision(), revision);

    let admission = f.owner.admission(&f.epoch).unwrap();
    assert_eq!(
        f.owner.finish_pending_tools(admission).unwrap().as_slice(),
        [2]
    );
    let admission = f.owner.admission(&f.epoch).unwrap();
    assert!(
        f.owner
            .finish_pending_tools(admission)
            .unwrap()
            .as_slice()
            .is_empty()
    );
    assert_eq!(tool_of(&f.owner.rows()[2]).status(), ToolStatus::Failed);
}

#[test]
fn user_chunks_and_truncation_go_through_one_published_snapshot_each() {
    let mut f = fixture();
    for value in ["one", "two"] {
        let admission = f.owner.admission(&f.epoch).unwrap();
        let update = crate::model::native::notice::user_chunk(user_text(&f.ledger, value));
        let _ = f.owner.apply_owned(admission, update, 9).unwrap();
    }
    let RowBody::User(user) = f.owner.rows()[1].view() else {
        panic!("user row")
    };
    assert_eq!(user.contents().count(), 2);
    let admission = f.owner.admission(&f.epoch).unwrap();
    assert!(f.owner.truncate(admission, 1).unwrap().saved_changed());
    assert_eq!(f.owner.rows().len(), 1);
    assert_eq!(f.owner.times().len(), 1);
}

#[test]
fn command_notices_publish_actual_controls_without_marking_saved_state() {
    let mut f = fixture();
    let (_, allocation) = text(&f.ledger, "").into_parts();
    let update = NativeOwned::from_parts(SessionUpdate::Commands(Vec::new()), allocation);
    let admission = f.owner.admission(&f.epoch).unwrap();
    let LiveRouted::Applied(applied) = f.owner.apply_owned(admission, update, 1).unwrap() else {
        panic!("controls must be owned by Live");
    };
    assert_eq!(applied.change(), ThreadChange::Controls);
    assert!(!applied.saved_changed());
    assert_eq!(f.owner.revision(), 1);
    assert!(f.owner.controls().unwrap().commands().is_empty());
}

#[test]
fn tool_patch_outside_the_saved_window_changes_live_but_not_durable_state() {
    let mut f = fixture();
    let (entry, allocation) = tool_row(&f.ledger, "old-call").into_parts();
    let Entry::Tool(record) = entry else {
        unreachable!()
    };
    let update =
        crate::model::native::notice::tool_started(NativeOwned::from_parts(record, allocation));
    let admission = f.owner.admission(&f.epoch).unwrap();
    let _ = f.owner.apply_owned(admission, update, 1).unwrap();
    for index in 0..crate::history::MAX_SAVED_ENTRIES {
        let content = user_text(&f.ledger, "row");
        let update = if index % 2 == 0 {
            crate::model::native::notice::agent_chunk(content)
        } else {
            crate::model::native::notice::thought_chunk(content)
        };
        let admission = f.owner.admission(&f.epoch).unwrap();
        let _ = f.owner.apply_owned(admission, update, 2).unwrap();
    }
    let revision = f.owner.revision();
    let admission = f.owner.admission(&f.epoch).unwrap();
    let id = crate::model::native::tools::tool_id(text(&f.ledger, "old-call")).unwrap();
    let patch = crate::model::native::notice::tool_changed(
        crate::model::native::patch::tool_patch(id, None, Some(ToolStatus::Succeeded)),
    );
    let result = applied(f.owner.apply_owned(admission, patch, 3).unwrap());
    assert!(!result.saved_changed());
    assert_eq!(result.change(), ThreadChange::Transcript);
    assert_eq!(result.revision(), revision + 1);
    assert_eq!(tool_of(&f.owner.rows()[1]).status(), ToolStatus::Succeeded);
    assert_eq!(f.owner.rows().len(), crate::history::MAX_SAVED_ENTRIES + 2);
    let admission = f.owner.admission(&f.epoch).unwrap();
    let display = crate::model::retention::document::tests::display(&f.ledger, "old-call");
    let result = f.owner.patch_display(admission, 1, display).unwrap();
    assert!(!result.saved_changed());
    assert_eq!(result.revision(), revision + 2);
    assert!(tool_of(&f.owner.rows()[1]).client_display().is_some());
}

#[test]
fn draft_edits_publish_once_and_equal_edits_preserve_admission() {
    let mut f = fixture();
    let draft = |value: &str| {
        let (value, allocation) = text(&f.ledger, value).into_parts();
        NativeOwned::from_parts(Some(value), allocation)
    };
    let admission = f.owner.admission(&f.epoch).unwrap();
    let changed = f.owner.replace_draft(admission, draft("draft")).unwrap();
    assert!(changed.saved_changed());
    assert_eq!(&*f.owner.composer().unwrap().draft().unwrap(), "draft");
    let admission = f.owner.admission(&f.epoch).unwrap();
    let unchanged = f
        .owner
        .replace_draft(admission.clone(), draft("draft"))
        .unwrap();
    assert!(!unchanged.saved_changed());
    assert_eq!(unchanged.revision(), changed.revision());
    let (_, allocation) = text(&f.ledger, "").into_parts();
    let cleared = f
        .owner
        .replace_draft(admission, NativeOwned::from_parts(None, allocation))
        .unwrap();
    assert!(cleared.saved_changed());
    assert!(f.owner.composer().unwrap().draft().is_none());
}

#[test]
fn rejected_draft_edits_leave_the_original_snapshot_and_admission_intact() {
    let mut f = fixture();
    let other = fixture();
    let admission = f.owner.admission(&f.epoch).unwrap();
    let (value, allocation) = text(&other.ledger, "foreign").into_parts();
    let result = f.owner.replace_draft(
        admission.clone(),
        NativeOwned::from_parts(Some(value), allocation),
    );
    assert!(matches!(
        result,
        Err(LiveError::Retention(RetentionError::ForeignAuthority))
    ));
    assert_eq!(f.owner.revision(), 0);
    assert!(f.owner.composer().unwrap().draft().is_none());
    let (value, allocation) = text(&f.ledger, "accepted").into_parts();
    assert!(
        f.owner
            .replace_draft(admission, NativeOwned::from_parts(Some(value), allocation),)
            .unwrap()
            .saved_changed()
    );
}

#[test]
fn queue_edits_keep_admission_on_noops_and_reject_stale_changes() {
    use crate::model::native::NativeList;
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut foreground = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&foreground, "seed")).unwrap();
    let ledger =
        DecodeLedger::for_operation(&foreground, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let mut queue = NativeList::new(&ledger, 3).unwrap();
    for id in [1, 2, 3] {
        queue
            .push(
                crate::history::native::document::prompt(
                    id,
                    text(&ledger, "queued"),
                    NativeList::new(&ledger, 0).unwrap().finish(),
                    NativeList::new(&ledger, 0).unwrap().finish(),
                )
                .unwrap(),
            )
            .unwrap();
    }
    let composer = crate::history::native::document::composer(
        NativeOwned::absent(&ledger).unwrap(),
        queue.finish(),
        NativeList::new(&ledger, 0).unwrap().finish(),
    )
    .unwrap();
    let mut builder = SnapshotBuilder::from_role(&mut foreground, &live).unwrap();
    builder.replace_composer(composer).unwrap();
    live.replace(builder.finish(None).unwrap()).unwrap();
    foreground.clear_source();
    let mut owner =
        LiveOwner::new(live, foreground, roles.take_prompt_dispatch().unwrap()).unwrap();
    let epoch = LiveEpoch::new(SessionState::default().ticket(), 1, SessionId::new("queue"));
    owner.accept_epoch(epoch.clone());
    let admission = owner.admission(&epoch).unwrap();
    assert!(
        !owner
            .remove_prompt(admission.clone(), 99)
            .unwrap()
            .saved_changed()
    );
    assert!(
        !owner
            .move_prompt(admission.clone(), 1, 0)
            .unwrap()
            .saved_changed()
    );
    assert!(
        owner
            .move_prompt(admission.clone(), 3, 0)
            .unwrap()
            .saved_changed()
    );
    assert!(matches!(
        owner.remove_prompt(admission, 1),
        Err(LiveError::Stale)
    ));
    let admission = owner.admission(&epoch).unwrap();
    assert!(owner.remove_prompt(admission, 2).unwrap().saved_changed());
    let ids: Vec<_> = owner
        .composer()
        .unwrap()
        .queue()
        .iter()
        .map(|p| p.id())
        .collect();
    assert_eq!(ids, [3, 1]);
    assert_eq!(owner.revision(), 2);
}

#[test]
fn volatile_context_changes_live_revision_without_changing_saved_state() {
    use crate::model::native::NativeList;
    let mut f = fixture();
    let values = |ledger: &DecodeLedger| {
        let mut values = NativeList::new(ledger, 1).unwrap();
        values
            .push(crate::history::live::inputs::connection(text(
                ledger, "context",
            )))
            .unwrap();
        values.finish()
    };
    let admission = f.owner.admission(&f.epoch).unwrap();
    let changed = f
        .owner
        .replace_attachments(admission.clone(), values(&f.ledger))
        .unwrap();
    assert_eq!(changed.revision(), 1);
    assert!(!changed.saved_changed());
    let context_ptr = f.owner.inputs().unwrap().attachments().as_ptr();
    assert!(matches!(
        f.owner.replace_attachments(admission, values(&f.ledger)),
        Err(LiveError::Stale)
    ));
    let admission = f.owner.admission(&f.epoch).unwrap();
    let unchanged = f
        .owner
        .replace_attachments(admission.clone(), values(&f.ledger))
        .unwrap();
    assert_eq!(unchanged.revision(), 1);
    assert!(!unchanged.saved_changed());
    let draft = NativeOwned::supplied(text(&f.ledger, "draft"));
    assert!(
        f.owner
            .replace_draft(admission, draft)
            .unwrap()
            .saved_changed()
    );
    assert!(f.owner.inputs().is_some());
    assert_eq!(
        f.owner.inputs().unwrap().attachments().as_ptr(),
        context_ptr
    );
    assert_eq!(f.owner.revision(), 2);
}

#[test]
fn foreign_context_is_rejected_without_invalidating_current_admission() {
    use crate::{history::live::inputs::LiveAttachment, model::native::NativeList};
    let mut f = fixture();
    let other = fixture();
    let admission = f.owner.admission(&f.epoch).unwrap();
    let empty = NativeList::<LiveAttachment>::new(&other.ledger, 0)
        .unwrap()
        .finish();
    assert!(matches!(
        f.owner.replace_attachments(admission.clone(), empty),
        Err(LiveError::Physical(PhysicalError::Retention(
            RetentionError::ForeignAuthority
        )))
    ));
    assert!(f.owner.inputs().is_none());
    assert_eq!(f.owner.revision(), 0);
    assert!(
        f.owner
            .replace_draft(admission, NativeOwned::supplied(text(&f.ledger, "valid")))
            .unwrap()
            .saved_changed()
    );
}

#[test]
fn owner_rejects_roles_of_another_kind() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let result = LiveOwner::new(
        roles.take_candidate().unwrap(),
        roles.take_foreground().unwrap(),
        roles.take_prompt_dispatch().unwrap(),
    );
    assert!(matches!(result, Err(LiveError::Role)));
}

#[test]
fn borrowed_draft_transition_uses_original_foreground_and_preserves_rows() {
    let mut f = fixture();
    let rows = f.owner.rows().as_ptr();
    let admission = f.owner.admission(&f.epoch).unwrap();
    let changed = f
        .owner
        .set_draft(admission.clone(), Some("typed draft"))
        .unwrap();
    assert!(changed.saved_changed());
    assert_eq!(
        f.owner.composer().unwrap().draft().as_deref(),
        Some("typed draft")
    );
    assert_eq!(f.owner.rows().as_ptr(), rows);
    assert!(matches!(
        f.owner.set_draft(admission, Some("stale")),
        Err(LiveError::Stale)
    ));
    let admission = f.owner.admission(&f.epoch).unwrap();
    assert!(
        !f.owner
            .set_draft(admission.clone(), Some("typed draft"))
            .unwrap()
            .saved_changed()
    );
    assert!(
        f.owner
            .set_draft(admission, Some(""))
            .unwrap()
            .saved_changed()
    );
    assert!(f.owner.composer().unwrap().draft().is_none());
}

#[test]
fn saved_candidate_shares_actual_live_rows_and_draft_without_changing_revision() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_scope, mut roles) = DocumentRetention::new(&authority).unwrap();
    let reader = roles.take_reader().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&reader, "kept")).unwrap();
    let mut owner = LiveOwner::new(
        live,
        roles.take_foreground().unwrap(),
        roles.take_prompt_dispatch().unwrap(),
    )
    .unwrap();
    let epoch = LiveEpoch::new(SessionState::default().ticket(), 1, SessionId::new("s"));
    owner.accept_epoch(epoch.clone());
    let admission = owner.admission(&epoch).unwrap();
    owner.set_draft(admission, Some("draft")).unwrap();
    let revision = owner.revision();
    let rows = owner.rows().as_ptr();
    let mut candidate = roles.take_candidate().unwrap();
    owner
        .prepare_saved_candidate(owner.admission(&epoch).unwrap(), &mut candidate)
        .unwrap();
    let snapshot = candidate.snapshot().unwrap();
    assert_eq!(snapshot.rows().as_ptr(), rows);
    assert_eq!(snapshot.composer().draft().as_deref(), Some("draft"));
    assert!(snapshot.live_inputs().is_none());
    assert_eq!(owner.revision(), revision);
    let admission = owner.admission(&epoch).unwrap();
    owner.set_draft(admission, Some("new draft")).unwrap();
    assert_eq!(
        candidate.snapshot().unwrap().composer().draft().as_deref(),
        Some("draft")
    );
}

#[test]
fn metadata_edits_share_unchanged_text_and_equal_values_keep_admission() {
    let mut f = fixture();
    let agent = f
        .owner
        .role()
        .snapshot()
        .unwrap()
        .metadata()
        .agent_id()
        .as_ptr();
    let rows = f.owner.rows().as_ptr();
    let admission = f.owner.admission(&f.epoch).unwrap();
    assert!(
        f.owner
            .set_title(admission.clone(), Some("title"))
            .unwrap()
            .saved_changed()
    );
    assert!(matches!(
        f.owner.rename(admission, Some("stale")),
        Err(LiveError::Stale)
    ));
    let title = f
        .owner
        .role()
        .snapshot()
        .unwrap()
        .metadata()
        .title()
        .unwrap()
        .as_ptr();
    let admission = f.owner.admission(&f.epoch).unwrap();
    assert!(
        !f.owner
            .set_title(admission.clone(), Some("title"))
            .unwrap()
            .saved_changed()
    );
    f.owner.rename(admission, Some("name")).unwrap();
    let metadata = f.owner.role().snapshot().unwrap().metadata();
    assert_eq!(metadata.agent_id().as_ptr(), agent);
    assert_eq!(metadata.title().unwrap().as_ptr(), title);
    assert_eq!(metadata.name().as_deref(), Some("name"));
    let admission = f.owner.admission(&f.epoch).unwrap();
    f.owner.stamp_metadata(admission, 99, true).unwrap();
    let metadata = f.owner.role().snapshot().unwrap().metadata();
    assert_eq!(metadata.agent_id().as_ptr(), agent);
    assert_eq!(metadata.title().unwrap().as_ptr(), title);
    assert_eq!(metadata.updated(), 99);
    assert!(metadata.pinned());
    assert_eq!(f.owner.rows().as_ptr(), rows);
}

#[test]
fn equal_tool_notice_keeps_original_rows_revision_and_admission() {
    let mut f = fixture();
    let (entry, allocation) = tool_row(&f.ledger, "call").into_parts();
    let Entry::Tool(record) = entry else {
        unreachable!()
    };
    let status = record.status;
    let update =
        crate::model::native::notice::tool_started(NativeOwned::from_parts(record, allocation));
    let admission = f.owner.admission(&f.epoch).unwrap();
    applied(f.owner.apply_owned(admission, update, 1).unwrap());
    let rows = f.owner.rows().as_ptr();
    let admission = f.owner.admission(&f.epoch).unwrap();
    let id = crate::model::native::tools::tool_id(text(&f.ledger, "call")).unwrap();
    let patch = crate::model::native::notice::tool_changed(
        crate::model::native::patch::tool_patch(id, None, Some(status)),
    );
    let result = applied(f.owner.apply_owned(admission.clone(), patch, 2).unwrap());
    assert_eq!(result.change(), ThreadChange::Transcript);
    assert!(!result.saved_changed());
    assert_eq!(result.revision(), admission.revision());
    assert_eq!(f.owner.rows().as_ptr(), rows);
    let id = crate::model::native::tools::tool_id(text(&f.ledger, "call")).unwrap();
    let patch = crate::model::native::notice::tool_changed(
        crate::model::native::patch::tool_patch(id, None, Some(ToolStatus::Succeeded)),
    );
    assert!(applied(f.owner.apply_owned(admission, patch, 3).unwrap()).saved_changed());
}

#[test]
fn plans_belong_to_live_and_survive_input_edits_but_are_omitted_from_candidates() {
    use crate::model::{
        native::NativeList,
        notice::{PlanPriority, PlanStatus},
    };
    let mut f = fixture();
    let plan = || {
        let mut steps = NativeList::new(&f.ledger, 1).unwrap();
        steps
            .push(crate::model::native::notice::plan_step(
                text(&f.ledger, "managed plan"),
                PlanPriority::High,
                PlanStatus::Pending,
            ))
            .unwrap();
        crate::model::native::notice::plan(steps.finish())
    };
    let admission = f.owner.admission(&f.epoch).unwrap();
    let result = applied(f.owner.apply_owned(admission, plan(), 1).unwrap());
    assert!(!result.saved_changed());
    assert_eq!(f.owner.plan().unwrap()[0].content, "managed plan");
    let payload = f.owner.plan().unwrap().as_ptr();
    let admission = f.owner.admission(&f.epoch).unwrap();
    let result = applied(f.owner.apply_owned(admission.clone(), plan(), 2).unwrap());
    assert_eq!(result.revision(), admission.revision());
    f.owner.set_draft(admission, Some("draft")).unwrap();
    assert_eq!(f.owner.plan().unwrap().as_ptr(), payload);
    f.owner
        .prepare_saved_candidate(f.owner.admission(&f.epoch).unwrap(), &mut f.candidate)
        .unwrap();
    assert!(f.candidate.snapshot().unwrap().presentation().is_none());
    assert_eq!(f.owner.plan().unwrap().as_ptr(), payload);
    assert_eq!(
        f.candidate
            .snapshot()
            .unwrap()
            .composer()
            .draft()
            .as_deref(),
        Some("draft")
    );
}

mod queue;

mod dispatch;

mod copies;
