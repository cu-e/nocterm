//! One installation admits hydration across original contexts and windows.
use super::{contexts::scoped_fixture, *};
use nocterm_ai::{
    history::{ContextId, MAX_RESIDENT_BYTES, SavedChat},
    storage::AgentStorageError,
};
const MIB: usize = 1024 * 1024;
use crate::{
    HistoryUsage,
    history_admission::{HistoryAdmissionError, testing::HistoryProbe},
};

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

#[gpui::test]
fn transferring_a_unique_buffer_releases_only_its_unused_reservation(cx: &mut TestAppContext) {
    let admission = HistoryProbe::installation();
    let reserved = cx.update(|cx| admission.reserve_unowned_bytes(32 * MIB, cx).unwrap());
    let retained = HistoryProbe::transfer_buffer(&admission, reserved, MIB).unwrap();
    assert_eq!(retained.bytes(), MIB);
    cx.update(|cx| drop(admission.reserve_unowned_bytes(60 * MIB, cx).unwrap()));
}

#[gpui::test]
fn a_shared_buffer_keeps_its_original_charge_until_shared_owners_release_it(
    cx: &mut TestAppContext,
) {
    let admission = HistoryProbe::installation();
    let reserved = cx.update(|cx| admission.reserve_unowned_bytes(32 * MIB, cx).unwrap());
    let original = reserved.clone();
    let retained = HistoryProbe::transfer_buffer(&admission, reserved, MIB).unwrap();
    assert_eq!(retained.bytes(), 32 * MIB);
    cx.update(|cx| {
        assert!(matches!(
            admission.reserve_unowned_bytes(40 * MIB, cx),
            Err(HistoryAdmissionError::ResidentCapacity { .. })
        ))
    });
    drop(original);
    let retained = HistoryProbe::transfer_buffer(&admission, retained, MIB).unwrap();
    assert_eq!(retained.bytes(), MIB);
    cx.update(|cx| drop(admission.reserve_unowned_bytes(40 * MIB, cx).unwrap()));
}

#[gpui::test]
fn buffer_transfer_refuses_foreign_allocations_and_growth_without_releasing_other_owners(
    cx: &mut TestAppContext,
) {
    let admission = HistoryProbe::installation();
    let foreign_owner = HistoryProbe::installation();
    let foreign = cx.update(|cx| foreign_owner.reserve_unowned_bytes(MIB, cx).unwrap());
    let retained = foreign.clone();
    assert!(matches!(
        HistoryProbe::transfer_buffer(&admission, foreign, MIB / 2),
        Err(HistoryAdmissionError::ForeignAllocation)
    ));
    assert!(
        matches!(HistoryProbe::transfer_buffer(&foreign_owner, retained.clone(), MIB * 2), Err(HistoryAdmissionError::AllocationGrowth { reserved, requested }) if reserved==MIB && requested==MIB*2)
    );
    cx.update(|cx| {
        assert!(
            foreign_owner
                .reserve_unowned_bytes(crate::history_admission::MAX_NORMAL_HISTORY_BYTES, cx)
                .is_err()
        )
    });
    drop(retained);
    cx.update(|cx| {
        drop(
            foreign_owner
                .reserve_unowned_bytes(crate::history_admission::MAX_NORMAL_HISTORY_BYTES, cx)
                .unwrap(),
        )
    });
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
        let incoming = MAX_RESIDENT_BYTES - retained + 64 * 1024;
        assert!(matches!(
            general.admit_history(client.id(), incoming, cx),
            Err(AgentStorageError::ResidentCapacity { .. })
        ));
        let claim = ownership::owned_claim(
            general
                .claim_document(prepared.header.id, client.id(), cx)
                .unwrap(),
        );
        let slot = general.history_admission(cx).reserve_payload().unwrap();
        drop(general.take_prepared_chat(&claim, slot, cx).unwrap());
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
            project.admit_history(
                requester.id(),
                MAX_RESIDENT_BYTES - retained + 64 * 1024,
                cx
            ),
            Err(AgentStorageError::ResidentCapacity { .. })
        ));
    });
    drop(first);
    drop(writer_client);
    release.send(Ok(())).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        project
            .admit_history(
                requester.id(),
                MAX_RESIDENT_BYTES - retained + 64 * 1024,
                cx,
            )
            .unwrap()
    });
}

#[gpui::test]
fn remote_admission_evicts_hidden_local_history_and_local_admission_evicts_remote_history(
    cx: &mut TestAppContext,
) {
    let (general, _directory, _, _, _) = scoped_fixture(cx);
    cx.run_until_parked();
    let protected = client_in(general.scope(), cx);
    let hidden_scope = general.for_scope(scope(1));
    let hidden = client_in(hidden_scope.scope(), cx);
    protected
        .history
        .set(Some(HistoryUsage::new(24 * MIB, 1, true)));
    hidden
        .history
        .set(Some(HistoryUsage::new(24 * MIB, 2, false)));
    hidden.allow_history_eviction.set(true);
    let remote = HistoryProbe::default();
    cx.update(|cx| {
        ownership::register_bound(&general, protected.clone(), cx).unwrap();
        ownership::register_bound(&hidden_scope, hidden.clone(), cx).unwrap();
        let admission = general.history_admission(cx);
        let registration = admission.register(remote.participant()).unwrap();
        let buffer = admission
            .reserve_bytes(&registration, 24 * MIB, cx)
            .unwrap();
        assert!(hidden.history.get().is_none());
        assert!(protected.history.get().is_some());
        remote.set_history(HistoryUsage::new(24 * MIB, 3, false), true);
        drop(buffer);
        hidden_scope
            .admit_history(hidden.id(), 24 * MIB, cx)
            .unwrap();
        assert!(remote.history().is_none());
        assert_eq!(remote.evictions(), 1);
        assert!(protected.history.get().is_some());
    });
}

#[gpui::test]
fn pending_remote_buffer_clones_stay_charged_until_the_last_owner_drops(cx: &mut TestAppContext) {
    let (general, _directory, _, _, _) = scoped_fixture(cx);
    let client = client_in(general.scope(), cx);
    cx.update(|cx| {
        ownership::register_bound(&general, client.clone(), cx).unwrap();
        let admission = general.history_admission(cx);
        let buffer = admission.reserve_unowned_bytes(40 * MIB, cx).unwrap();
        let pending = buffer.clone();
        drop(buffer);
        assert!(matches!(
            general.admit_history(client.id(), 32 * MIB, cx),
            Err(AgentStorageError::ResidentCapacity { .. })
        ));
        drop(pending);
        general.admit_history(client.id(), 32 * MIB, cx).unwrap();
    });
}

#[gpui::test]
fn local_and_remote_payloads_reclaim_each_other_through_one_installation_cap(
    cx: &mut TestAppContext,
) {
    let (general, _directory, _, _, _) = scoped_fixture(cx);
    cx.run_until_parked();
    let remote = HistoryProbe::default();
    remote.set_history(HistoryUsage::new(0, 1, false), true);
    cx.update(|cx| {
        let admission = general.history_admission(cx);
        let _registration = admission.register(remote.participant()).unwrap();
        for _ in 1..crate::history_admission::MAX_NORMAL_HISTORY_PAYLOADS {
            remote.retain_payload(admission.reserve_payload().unwrap());
        }
        let payload = admission.reserve_payload_reclaiming(None, cx).unwrap();
        assert!(
            general.catalog_snapshot(cx).is_none(),
            "oldest unpinned Local catalog was reclaimed"
        );
        remote.retain_payload(payload);
        assert_eq!(
            remote.payload_count(),
            crate::history_admission::MAX_NORMAL_HISTORY_PAYLOADS
        );
        general.try_reload_history(cx).unwrap();
        assert_eq!(
            remote.payload_count(),
            crate::history_admission::MAX_NORMAL_HISTORY_PAYLOADS - 1,
            "Local load reclaimed a remote payload"
        );
    });
    cx.run_until_parked();
    assert!(cx.update(|cx| general.catalog_snapshot(cx).is_some()));
}

#[gpui::test]
fn history_eviction_callbacks_run_without_a_budget_borrow_and_nested_admission_refuses(
    cx: &mut TestAppContext,
) {
    let (general, _directory, _, _, _) = scoped_fixture(cx);
    let remote = HistoryProbe::default();
    remote.set_history(HistoryUsage::new(40 * MIB, 1, false), true);
    cx.update(|cx| {
        let admission = general.history_admission(cx);
        let _registration = admission.register(remote.participant()).unwrap();
        let nested = admission.clone();
        let called = Rc::new(Cell::new(false));
        let observed = called.clone();
        remote.on_evict(Rc::new(move |cx| {
            // Registration may mutate budget state, while recursive eviction
            // decisions must refuse instead of admitting uncounted work.
            let probe = HistoryProbe::default();
            let _registration = nested.register(probe.participant()).unwrap();
            assert!(matches!(
                nested.reserve_unowned_bytes(1, cx),
                Err(HistoryAdmissionError::Reentrant)
            ));
            observed.set(true);
        }));
        let _buffer = admission.reserve_unowned_bytes(40 * MIB, cx).unwrap();
        assert!(called.get());
    });
}

#[gpui::test]
fn unchanged_payload_offers_stop_reclamation_even_when_the_callback_reports_success(
    cx: &mut TestAppContext,
) {
    use crate::history_admission::{HistoryAdmissionError, testing::HistoryProbe};
    let admission = HistoryProbe::installation();
    let neighbor = HistoryProbe::default();
    neighbor.set_history(crate::HistoryUsage::new(0, 0, false), true);
    neighbor.report_payload_success_without_release();
    let _registration = admission.register(neighbor.participant()).unwrap();
    for _ in 0..crate::history_admission::MAX_NORMAL_HISTORY_PAYLOADS {
        neighbor.retain_payload(admission.reserve_payload().unwrap());
    }
    cx.update(|cx| {
        assert!(matches!(
            admission.reserve_payload_reclaiming(None, cx),
            Err(HistoryAdmissionError::PayloadCapacity)
        ))
    });
    assert_eq!(neighbor.evictions(), 1);
    assert_eq!(
        neighbor.payload_count(),
        crate::history_admission::MAX_NORMAL_HISTORY_PAYLOADS
    );
}

#[gpui::test]
fn normal_allocations_preserve_urgent_control_capacity_inside_the_hard_installation_bounds(
    cx: &mut TestAppContext,
) {
    use crate::history_admission::{
        MAX_NORMAL_HISTORY_BYTES, MAX_NORMAL_HISTORY_PAYLOADS, URGENT_HISTORY_BYTES,
        testing::HistoryProbe,
    };
    let admission = HistoryProbe::installation();
    let neighbor = HistoryProbe::default();
    let registration = admission.register(neighbor.participant()).unwrap();
    let normal = (0..MAX_NORMAL_HISTORY_PAYLOADS)
        .map(|_| admission.reserve_payload().unwrap())
        .collect::<Vec<_>>();
    let bytes = cx.update(|cx| {
        admission
            .reserve_unowned_bytes(MAX_NORMAL_HISTORY_BYTES, cx)
            .unwrap()
    });
    assert!(admission.reserve_payload().is_err());
    let urgent = cx.update(|cx| {
        admission
            .reserve_urgent_control(&registration, URGENT_HISTORY_BYTES, 2, cx)
            .unwrap()
    });
    cx.update(|cx| {
        assert!(
            admission
                .reserve_urgent_control(&registration, 1, 1, cx)
                .is_err()
        )
    });
    let clone = urgent.clone();
    drop(urgent);
    cx.update(|cx| {
        assert!(
            admission
                .reserve_urgent_control(&registration, 1, 1, cx)
                .is_err()
        )
    });
    drop(clone);
    cx.update(|cx| {
        drop(
            admission
                .reserve_urgent_control(&registration, URGENT_HISTORY_BYTES, 2, cx)
                .unwrap(),
        )
    });
    drop((normal, bytes));
}

#[gpui::test]
fn snapshot_arc_clones_share_one_generation_and_release_only_after_the_actual_last_drop(
    cx: &mut TestAppContext,
) {
    let (general, _directory, _, _, _) = scoped_fixture(cx);
    cx.run_until_parked();
    let requester = client_in(general.scope(), cx);
    let value = snapshot();
    let source = cx.update(|cx| general.local_history_allocations(cx));
    let allocation = cx.update(|cx| {
        source
            .reserve_snapshot(requester.id(), 0, value.retained_bytes().unwrap(), cx)
            .unwrap()
    });
    let value = allocation.retain(value).unwrap();
    let clone = value.clone();
    let admission = cx.update(|cx| general.history_admission(cx));
    let others = (0..crate::history_admission::MAX_NORMAL_HISTORY_PAYLOADS - 2)
        .map(|_| admission.reserve_payload().unwrap())
        .collect::<Vec<_>>();
    assert!(admission.reserve_payload().is_err()); // catalog + snapshot +28normal
    drop(value);
    assert!(admission.reserve_payload().is_err());
    drop(clone);
    let freed = admission.reserve_payload().unwrap();
    drop((freed, others));
}

#[gpui::test]
fn active_and_latest_writer_generations_use_distinct_shared_slots_until_native_completion(
    cx: &mut TestAppContext,
) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    cx.run_until_parked();
    let first = snapshot();
    let client = ProbeClient::new(cx, first.clone());
    let release = repository.block_next(StorageOperation::Save(first.header.id.clone()));
    cx.update(|cx| {
        ownership::register_bound(&general, client.clone(), cx).unwrap();
        general
            .save_chat(first.clone(), client.id(), 1, cx)
            .unwrap();
    });
    cx.run_until_parked();
    let second = snapshot();
    cx.update(|cx| {
        *client.chat.borrow_mut() = second.clone();
        client.revision.set(2);
        general
            .save_chat(second.clone(), client.id(), 2, cx)
            .unwrap();
        general.unregister_document(client.id(), cx).unwrap();
    });
    drop((first, second, client));
    let admission = cx.update(|cx| general.history_admission(cx));
    // The catalog is cold and invalidated by retirement; both actual writer
    // generations still exist in the common store/native worker.
    let retained = (0..crate::history_admission::MAX_NORMAL_HISTORY_PAYLOADS - 2)
        .map(|_| admission.reserve_payload().unwrap())
        .collect::<Vec<_>>();
    assert!(admission.reserve_payload().is_err());
    release.send(Ok(())).unwrap();
    cx.run_until_parked();
    let freed = admission.reserve_payload().unwrap();
    drop((freed, retained));
}
