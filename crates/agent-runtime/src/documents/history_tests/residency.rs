//! One installation admits hydration across original contexts and windows.
use super::{contexts::scoped_fixture, *};
use nocterm_ai::{
    history::{ContextId, MAX_RESIDENT_BYTES, SavedChat},
    storage::AgentStorageError,
};
const MIB: usize = 1024 * 1024;

fn client_in(scope: ContextId, cx: &mut TestAppContext) -> Rc<ProbeClient> {
    let mut metadata = snapshot().metadata.clone();
    metadata.version = 2;
    metadata.header.scope = scope;
    ProbeClient::new(
        cx,
        Arc::new(SharedChat {
            metadata,
            queue: Vec::new(),
        }),
    )
}
fn scope(index: u8) -> ContextId {
    serde_json::from_value(serde_json::json!({"kind":"project", "id": format!("{index:032x}")}))
        .unwrap()
}
#[gpui::test]
fn hydration_capacity_is_shared_by_contexts_and_shown_chats_in_other_windows_are_protected(
    cx: &mut TestAppContext,
) {
    let (general, _directory, _, _, _) = scoped_fixture(cx);
    let a = general.for_scope(scope(1));
    let b = general.for_scope(scope(2));
    let c = general.for_scope(scope(3));
    let first = client_in(a.scope(), cx);
    let second = client_in(b.scope(), cx);
    let requester = client_in(c.scope(), cx);
    first
        .history
        .set(Some(crate::HistoryUsage::new(24 * MIB, 1, true)));
    first.allow_history_eviction.set(true);
    second
        .history
        .set(Some(crate::HistoryUsage::new(24 * MIB, 2, false)));
    requester
        .history
        .set(Some(crate::HistoryUsage::new(0, 3, false)));
    cx.update(|cx| {
        ownership::register_bound(&a, first.clone(), cx).unwrap();
        ownership::register_bound(&b, second.clone(), cx).unwrap();
        ownership::register_bound(&c, requester.clone(), cx).unwrap();
        assert!(matches!(
            c.admit_history(requester.id(), 24 * MIB, cx),
            Err(AgentStorageError::ResidentCapacity { .. })
        ));
        assert_eq!(
            requester.history_reads.get(),
            0,
            "requester projection would reenter its borrowed entity"
        );
        assert!(first.history.get().is_some());
        assert!(second.history.get().is_some());
        second.allow_history_eviction.set(true);
        c.admit_history(requester.id(), 24 * MIB, cx).unwrap();
        assert!(first.history.get().is_some());
        assert!(second.history.get().is_none());
        assert!(matches!(
            a.admit_history(requester.id(), 1, cx),
            Err(AgentStorageError::ScopeMismatch { .. })
        ));
    });
}
#[gpui::test]
fn remaining_prepared_payloads_count_until_their_claimed_handoff(cx: &mut TestAppContext) {
    let mut prepared = snapshot().metadata.clone();
    prepared.draft = Some("p".repeat(MIB));
    let history = crate::PreparedHistory::new(vec![prepared.clone()]).unwrap();
    let retained = history.records()[0].resident_bytes();
    let (general, _directory, _, _, _) = contexts::scoped_fixture_with_history(cx, Some(history));
    let mut other = SavedChat::for_identity(
        "codex".into(),
        "00000000-0000-0000-0000-000000000002".into(),
        1,
    );
    other.header.scope = ContextId::General;
    let client = ProbeClient::new(
        cx,
        Arc::new(SharedChat {
            metadata: other,
            queue: Vec::new(),
        }),
    );
    cx.update(|cx| {
        ownership::register_bound(&general, client.clone(), cx).unwrap();
        let incoming = MAX_RESIDENT_BYTES - retained + 1;
        assert!(matches!(
            general.admit_history(client.id(), incoming, cx),
            Err(AgentStorageError::ResidentCapacity { .. })
        ));
        let claim = ownership::owned_claim(
            general
                .claim_document(prepared.header.id, client.id(), cx)
                .unwrap(),
        );
        drop(general.take_prepared_chat(&claim, cx).unwrap());
        general.admit_history(client.id(), incoming, cx).unwrap();
    });
}

#[gpui::test]
fn detached_active_and_latest_writer_payloads_remain_in_hydration_admission(
    cx: &mut TestAppContext,
) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    let mut first = snapshot().metadata.clone();
    first
        .entries
        .push(nocterm_ai::thread::Entry::Agent("a".repeat(MIB)));
    let first = Arc::new(SharedChat {
        metadata: first,
        queue: Vec::new(),
    });
    let mut second = first.metadata.clone();
    second
        .entries
        .push(nocterm_ai::thread::Entry::Agent("b".repeat(MIB)));
    let second = Arc::new(SharedChat {
        metadata: second,
        queue: Vec::new(),
    });
    let retained = first.retained_bytes().unwrap() + second.retained_bytes().unwrap();
    let writer_client = ProbeClient::new(cx, first.clone());
    let project = general.for_scope(scope(1));
    let requester = client_in(project.scope(), cx);
    let release = repository.block_next(StorageOperation::Save(first.header.id.clone()));
    cx.update(|cx| {
        ownership::register_bound(&general, writer_client.clone(), cx).unwrap();
        ownership::register_bound(&project, requester.clone(), cx).unwrap();
        general
            .save_chat(first.clone(), writer_client.id(), 1, cx)
            .unwrap();
    });
    cx.run_until_parked();
    cx.update(|cx| {
        *writer_client.chat.borrow_mut() = second.clone();
        writer_client.revision.set(2);
        general
            .save_chat(second, writer_client.id(), 2, cx)
            .unwrap();
        general.unregister_document(writer_client.id(), cx).unwrap();
        assert!(matches!(
            project.admit_history(requester.id(), MAX_RESIDENT_BYTES - retained + 1, cx),
            Err(AgentStorageError::ResidentCapacity { .. })
        ));
    });
    release.send(Ok(())).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        project
            .admit_history(requester.id(), MAX_RESIDENT_BYTES - retained + 1, cx)
            .unwrap()
    });
}
