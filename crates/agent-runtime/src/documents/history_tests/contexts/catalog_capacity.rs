//! Catalog generations, shared slots and retained metadata allocation.
use super::*;
use crate::history_admission::HistoryAdmissionError;

#[gpui::test]
fn retained_catalog_generations_consume_the_installation_limit_until_dropped(
    cx: &mut TestAppContext,
) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    cx.run_until_parked();
    let mut retained = Vec::new();
    for _ in 0..crate::history_admission::MAX_NORMAL_HISTORY_PAYLOADS {
        retained.push(cx.update(|cx| general.catalog_snapshot(cx).unwrap()));
        if retained.len() < crate::history_admission::MAX_NORMAL_HISTORY_PAYLOADS {
            cx.update(|cx| general.try_reload_history(cx).unwrap());
            cx.run_until_parked();
        }
    }
    let generation = retained.last().unwrap().generation();
    let attempts = repository.operations().len();
    cx.update(|cx| {
        assert!(matches!(
            general.try_reload_history(cx),
            Err(crate::CatalogLoadError::Capacity)
        ));
        assert_eq!(
            general.catalog_snapshot(cx).unwrap().generation(),
            generation
        );
        assert!(matches!(
            general.for_scope(project_scope(1)).try_reload_history(cx),
            Err(crate::CatalogLoadError::Capacity)
        ));
    });
    cx.run_until_parked();
    assert_eq!(repository.operations().len(), attempts);
    drop(retained.remove(0));
    cx.update(|cx| general.try_reload_history(cx).unwrap());
    cx.run_until_parked();
    assert!(cx.update(|cx| general.catalog_snapshot(cx).unwrap().generation()) > generation);
}
#[gpui::test]
fn pending_catalog_requests_share_the_same_installation_byte_capacity(cx: &mut TestAppContext) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    // General's initial scan already owns its worst-case metadata allocation,
    // including before the background work's first poll.
    let mut admitted = 1;
    let mut refused = None;
    cx.update(|cx| {
        for index in 1..32 {
            match general
                .for_scope(project_scope(index))
                .try_reload_history(cx)
            {
                Ok(()) => admitted += 1,
                Err(crate::CatalogLoadError::ResidentCapacity { .. }) => {
                    refused = Some(index);
                    break;
                }
                Err(error) => panic!("unexpected catalog refusal: {error}"),
            }
        }
    });
    assert!(admitted > 1 && admitted < 32);
    let refused = refused.expect("bounded pending metadata exhausts bytes before32slots");
    cx.run_until_parked();
    assert_eq!(
        repository
            .operations()
            .iter()
            .filter(|operation| **operation == StorageOperation::Catalog)
            .count(),
        admitted
    );
    // Each completed result transfers to a smaller immutable retained allocation.
    cx.update(|cx| {
        general
            .for_scope(project_scope(refused))
            .try_reload_history(cx)
            .unwrap()
    });
    cx.run_until_parked();
    assert!(
        cx.update(|cx| general
            .for_scope(project_scope(refused))
            .catalog_snapshot(cx))
            .is_some()
    );
}
#[gpui::test]
fn an_old_catalog_load_cannot_replace_a_newer_scope_generation(cx: &mut TestAppContext) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    cx.run_until_parked();
    let project = general.for_scope(project_chat().header.scope);
    let release = repository.block_next(StorageOperation::Catalog);
    cx.update(|cx| project.reload_history(cx));
    cx.run_until_parked();
    let mut chat = project_chat();
    chat.header.title = Some("Current generation".into());
    repository.seed_chat(&chat).unwrap();
    cx.update(|cx| project.reload_history(cx));
    cx.run_until_parked();
    let latest = cx.update(|cx| project.catalog_snapshot(cx).unwrap());
    chat.header.title = Some("Late stale generation".into());
    repository.seed_chat(&chat).unwrap();
    let _ = release.send(Ok(()));
    cx.run_until_parked();
    cx.update(|cx| {
        let actual = project.catalog_snapshot(cx).unwrap();
        assert_eq!(actual.generation(), latest.generation());
        assert_eq!(actual.rows()[0].title(), "Current generation");
    });
}

#[gpui::test]
fn a_retained_catalog_row_keeps_metadata_charged_until_its_last_reference_drops(
    cx: &mut TestAppContext,
) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    cx.run_until_parked();
    let mut saved = snapshot().metadata.clone();
    saved.header.title = Some("界".repeat(512));
    saved.header.name = saved.header.title.clone();
    saved.model = saved.header.title.clone();
    repository.seed_chat(&saved).unwrap();
    cx.update(|cx| general.try_reload_history(cx).unwrap());
    cx.run_until_parked();
    let snapshot = cx.update(|cx| general.catalog_snapshot(cx).unwrap());
    let row = snapshot.row(0).unwrap();
    let second = row.clone();
    drop(snapshot);
    cx.update(|cx| general.delete_chat(saved.header.id, cx));
    cx.run_until_parked();
    let admission = cx.update(|cx| general.history_admission(cx));
    let incoming = crate::history_admission::MAX_NORMAL_HISTORY_BYTES - 8 * 1024;
    cx.update(|cx| {
        assert!(matches!(
            admission.reserve_unowned_bytes(incoming, cx),
            Err(HistoryAdmissionError::ResidentCapacity { .. })
        ))
    });
    drop(row);
    cx.update(|cx| assert!(admission.reserve_unowned_bytes(incoming, cx).is_err()));
    assert_eq!(second.summary().name.as_ref().unwrap(), &"界".repeat(512));
    drop(second);
    cx.update(|cx| drop(admission.reserve_unowned_bytes(incoming, cx).unwrap()));
}

#[gpui::test]
fn replacing_a_loading_generation_keeps_its_native_read_budget_until_actual_completion(
    cx: &mut TestAppContext,
) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    cx.run_until_parked();
    let admission = cx.update(|cx| general.history_admission(cx));
    let release = repository.block_next(StorageOperation::Catalog);
    cx.update(|cx| general.try_reload_history(cx).unwrap());
    cx.run_until_parked();
    cx.update(|cx| general.try_reload_history(cx).unwrap());
    cx.run_until_parked();
    let latest = cx.update(|cx| general.catalog_snapshot(cx).unwrap());
    let generation = latest.generation();
    cx.update(|cx| {
        assert!(matches!(
            admission.reserve_unowned_bytes(56 * 1024 * 1024, cx),
            Err(HistoryAdmissionError::ResidentCapacity { .. })
        ))
    });
    release.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| general.catalog_snapshot(cx).unwrap().generation()),
        generation
    );
    cx.update(|cx| {
        drop(
            admission
                .reserve_unowned_bytes(56 * 1024 * 1024, cx)
                .unwrap(),
        )
    });
}

#[gpui::test]
fn catalog_work_observes_the_same_retained_remote_bytes_before_native_start(
    cx: &mut TestAppContext,
) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    cx.run_until_parked();
    let admission = cx.update(|cx| general.history_admission(cx));
    let pending = cx.update(|cx| {
        admission
            .reserve_unowned_bytes(56 * 1024 * 1024, cx)
            .unwrap()
    });
    let attempts = repository.operations().len();
    cx.update(|cx| {
        assert!(matches!(
            general.try_reload_history(cx),
            Err(crate::CatalogLoadError::ResidentCapacity { .. })
        ))
    });
    cx.run_until_parked();
    assert_eq!(repository.operations().len(), attempts);
    drop(pending);
    cx.update(|cx| general.try_reload_history(cx).unwrap());
    cx.run_until_parked();
    assert_eq!(repository.operations().len(), attempts + 1);
}

#[gpui::test]
fn byte_pressure_reclaims_an_unretained_catalog_payload_before_refusing_shared_history(
    cx: &mut TestAppContext,
) {
    let (general, _directory, _, _, _) = scoped_fixture(cx);
    cx.run_until_parked();
    let admission = cx.update(|cx| general.history_admission(cx));
    assert!(cx.update(|cx| general.catalog_snapshot(cx)).is_some());
    let incoming = cx.update(|cx| {
        admission
            .reserve_unowned_bytes(crate::history_admission::MAX_NORMAL_HISTORY_BYTES, cx)
            .unwrap()
    });
    assert!(cx.update(|cx| general.catalog_snapshot(cx)).is_none());
    drop(incoming);
    cx.update(|cx| general.ensure_history(cx).unwrap());
    cx.run_until_parked();
    assert!(cx.update(|cx| general.catalog_snapshot(cx)).is_some());
}

#[gpui::test]
fn a_catalog_reload_reclaims_its_unpinned_cache_without_reentering_runtime(
    cx: &mut TestAppContext,
) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    cx.run_until_parked();
    let old = general.for_scope(project_scope(1));
    for index in 0..nocterm_ai::history::MAX_RESTORED {
        let mut chat = project_chat();
        chat.header.scope = old.scope();
        chat.header.id = format!("00000000-0000-0000-0000-{index:012x}");
        chat.header.title = Some("界".repeat(512));
        chat.header.name = chat.header.title.clone();
        chat.model = chat.header.title.clone();
        repository.seed_chat(&chat).unwrap();
    }
    cx.update(|cx| old.try_reload_history(cx).unwrap());
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| old.catalog_snapshot(cx).unwrap().rows().len()),
        nocterm_ai::history::MAX_RESTORED
    );
    let admission = cx.update(|cx| general.history_admission(cx));
    let remote = cx.update(|cx| {
        admission
            // Leaves room for exactly one worst-case catalog reload, so the
            // second scope fits only if the unpinned first scope is reclaimed.
            .reserve_unowned_bytes(
                crate::history_admission::MAX_NORMAL_HISTORY_BYTES
                    - crate::catalog::CATALOG_WORK_BYTES,
                cx,
            )
            .unwrap()
    });
    let next = general.for_scope(project_scope(2));
    cx.update(|cx| next.try_reload_history(cx).unwrap());
    assert!(cx.update(|cx| old.catalog_snapshot(cx)).is_none());
    cx.run_until_parked();
    assert!(cx.update(|cx| next.catalog_snapshot(cx)).is_some());
    drop(remote);
}

#[gpui::test]
fn byte_pressure_reclaims_every_cold_scope_offered_by_one_catalog_owner(cx: &mut TestAppContext) {
    let (general, _directory, _, _, _) = scoped_fixture(cx);
    cx.run_until_parked();
    let project = general.for_scope(project_scope(1));
    cx.update(|cx| project.ensure_history(cx).unwrap());
    cx.run_until_parked();
    assert!(cx.update(|cx| general.catalog_snapshot(cx)).is_some());
    assert!(cx.update(|cx| project.catalog_snapshot(cx)).is_some());
    let admission = cx.update(|cx| general.history_admission(cx));
    let incoming = cx.update(|cx| {
        admission
            .reserve_unowned_bytes(crate::history_admission::MAX_NORMAL_HISTORY_BYTES, cx)
            .unwrap()
    });
    assert!(cx.update(|cx| general.catalog_snapshot(cx)).is_none());
    assert!(cx.update(|cx| project.catalog_snapshot(cx)).is_none());
    drop(incoming);
}
