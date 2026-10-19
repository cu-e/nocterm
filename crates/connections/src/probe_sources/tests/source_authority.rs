use super::*;
#[gpui_kit::test]
fn disconnect_with_retained_filesystem_rejects_completion_before_source_observer(
    cx: &mut TestAppContext,
) {
    let scene = Scene::new(false, cx);
    let fs: Arc<dyn RemoteFs> = Arc::new(InMemoryFs::default());
    let retained = fs.clone();
    let item = scene.add(scene.window, &scene.workspace, fs.clone(), cx);
    let gate = Gate::pending();
    scene.os.held_next(Ok(OsId::new("debian")), gate.clone());
    scene.scan(cx);
    until(cx, |_| scene.os.calls() == 1);
    // No Connections::register subscription exists: only the captured live receipt can reject.
    item.update(cx, |item, cx| item.set_connected(false, cx).unwrap());
    gate.release();
    until(cx, |cx| scene.slots(cx) == 32);
    assert_eq!(scene.os(cx), None);
    assert!(Arc::ptr_eq(&fs, &retained));
    scene.scan(cx);
    assert_eq!(scene.os.calls(), 1);
}
#[gpui_kit::test]
fn same_filesystem_new_session_epoch_admits_new_job_and_old_result_cannot_clear_it(
    cx: &mut TestAppContext,
) {
    let scene = Scene::new(false, cx);
    let fs: Arc<dyn RemoteFs> = Arc::new(InMemoryFs::default());
    let item = scene.add(scene.window, &scene.workspace, fs.clone(), cx);
    let old = Gate::pending();
    let new = Gate::pending();
    scene.os.held_next(Ok(OsId::new("debian")), old.clone());
    scene.os.held_next(Ok(OsId::new("arch")), new.clone());
    scene.scan(cx);
    until(cx, |_| scene.os.calls() == 1);
    item.update(cx, |item, cx| {
        item.set_connected(false, cx).unwrap();
        item.set_connected(true, cx).unwrap();
    });
    scene.scan(cx);
    until(cx, |_| scene.os.calls() == 2);
    assert_eq!(scene.slots(cx), 30);
    old.release();
    until(cx, |cx| scene.slots(cx) == 31);
    assert_eq!(scene.os(cx), None);
    new.release();
    until(cx, |cx| scene.slots(cx) == 32);
    assert_eq!(scene.os(cx).as_deref(), Some("arch"));
    scene.scan(cx);
    cx.run_until_parked();
    assert_eq!(scene.os.calls(), 2);
    let recorded = scene.os.sessions();
    assert!(
        recorded
            .iter()
            .all(|original| Weak::ptr_eq(original, &Arc::downgrade(&fs)))
    );
}
#[gpui_kit::test]
fn removing_original_item_does_not_borrow_another_windows_live_filesystem(cx: &mut TestAppContext) {
    let scene = Scene::new(false, cx);
    let fs: Arc<dyn RemoteFs> = Arc::new(InMemoryFs::default());
    let original = scene.add(scene.window, &scene.workspace, fs.clone(), cx);
    let gate = Gate::pending();
    scene.os.held_next(Ok(OsId::new("debian")), gate.clone());
    scene.scan(cx);
    until(cx, |_| scene.os.calls() == 1);
    let (other_window, other_workspace) = scene.second_window(cx);
    let other = scene.add(other_window, &other_workspace, fs.clone(), cx);
    cx.update_window(scene.window, |_, window, cx| {
        scene
            .workspace
            .update(cx, |workspace, cx| workspace.close_item(0, window, cx))
    })
    .unwrap();
    let new_gate = Gate::pending();
    scene.os.held_next(Ok(OsId::new("arch")), new_gate.clone());
    gate.release();
    until(cx, |cx| scene.os.calls() == 2 && scene.slots(cx) == 31);
    assert_eq!(scene.os(cx), None);
    assert!(!original.read_with(cx, |item, cx| {
        nocterm_workspace::Item::session(item, cx)
            .unwrap()
            .connected
    }));
    assert!(other.read_with(cx, |item, cx| {
        nocterm_workspace::Item::session(item, cx)
            .unwrap()
            .connected
    }));
    new_gate.release();
    scene.scan(cx);
    until(cx, |_| scene.os.calls() == 2);
    until(cx, |cx| scene.slots(cx) == 32);
    assert_eq!(scene.os(cx).as_deref(), Some("arch"));
}

#[gpui_kit::test]
fn same_actual_item_and_epoch_projected_in_two_windows_keeps_one_capability_decision(
    cx: &mut TestAppContext,
) {
    let scene = Scene::new(false, cx);
    let fs: Arc<dyn RemoteFs> = Arc::new(InMemoryFs::default());
    let item = scene.add(scene.window, &scene.workspace, fs.clone(), cx);
    let gate = Gate::pending();
    scene.os.held_next(Ok(OsId::new("debian")), gate.clone());
    scene.scan(cx);
    until(cx, |_| scene.os.calls() == 1);
    let (window, workspace) = scene.second_window(cx);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.add_item(item.clone(), window, cx)
        })
    })
    .unwrap();
    for _ in 0..4 {
        scene.scan(cx);
    }
    cx.run_until_parked();
    assert_eq!(scene.os.calls(), 1);
    assert_eq!(scene.slots(cx), 31);
    gate.release();
    until(cx, |cx| scene.slots(cx) == 32);
    assert_eq!(scene.os(cx).as_deref(), Some("debian"));
    scene.scan(cx);
    cx.run_until_parked();
    assert_eq!(scene.os.calls(), 1);
}

#[gpui_kit::test]
fn dropping_projection_window_preserves_original_item_transferred_to_another_window(
    cx: &mut TestAppContext,
) {
    let scene = Scene::new(false, cx);
    let item = scene.add(
        scene.window,
        &scene.workspace,
        Arc::new(InMemoryFs::default()),
        cx,
    );
    let gate = Gate::pending();
    scene.os.held_next(Ok(OsId::new("debian")), gate.clone());
    scene.scan(cx);
    until(cx, |_| scene.os.calls() == 1);
    let (other_window, other_workspace) = scene.second_window(cx);
    cx.update_window(other_window, |_, window, cx| {
        other_workspace.update(cx, |workspace, cx| {
            workspace.add_item(item.clone(), window, cx)
        })
    })
    .unwrap();
    let installed = scene.installed.clone();
    let calls = scene.os.clone();
    cx.update_window(scene.window, |_, window, _| window.remove_window())
        .unwrap();
    drop(scene);
    // Reprojection removes the weak originating window but cannot revoke a live original Item.
    cx.update(|cx| {
        installed
            .sources()
            .scan(installed.connections(), installed.facts(), cx)
    });
    assert_eq!(calls.calls(), 1);
    gate.release();
    until(cx, |cx| {
        installed
            .facts()
            .read_with(cx, |facts, _| facts.available_jobs() == 32)
    });
    assert_eq!(
        installed.facts().read_with(cx, |facts, cx| facts
            .os(
                nocterm_connections_model::test_support::profile_id_for("source"),
                cx
            )
            .map(str::to_owned)),
        Some("debian".into())
    );
    assert!(item.read_with(cx, |item, cx| {
        nocterm_workspace::Item::session(item, cx)
            .unwrap()
            .connected
    }));
}

#[gpui_kit::test]
fn reassociating_original_item_without_reconnect_rejects_old_profile_completion(
    cx: &mut TestAppContext,
) {
    let scene = Scene::new(false, cx);
    let item = scene.add(
        scene.window,
        &scene.workspace,
        Arc::new(InMemoryFs::default()),
        cx,
    );
    let gate = Gate::pending();
    scene.os.held_next(Ok(OsId::new("debian")), gate.clone());
    scene.scan(cx);
    until(cx, |_| scene.os.calls() == 1);
    let mut second = scene.profile.clone();
    second.id = nocterm_connections_model::test_support::profile_id_for("second");
    let receipt = scene.installed.connections().update(cx, |connections, cx| {
        connections.save_profile(second.clone(), cx)
    });
    cx.run_until_parked();
    futures::executor::block_on(receipt).unwrap();
    item.update(cx, |item, cx| {
        item.associate_profile(Some(second.id.to_string().into()), cx)
    });
    gate.release();
    until(cx, |cx| scene.slots(cx) == 32);
    assert_eq!(scene.os(cx), None);
    assert!(
        scene
            .installed
            .facts()
            .read_with(cx, |facts, cx| facts.os(second.id, cx).is_none())
    );
    scene.scan(cx);
    cx.run_until_parked();
    assert_eq!(scene.os.calls(), 1);
}

#[gpui_kit::test]
fn saved_profile_name_change_preserves_original_id_association_and_ticket(cx: &mut TestAppContext) {
    let scene = Scene::new(false, cx);
    let item = scene.add(
        scene.window,
        &scene.workspace,
        Arc::new(InMemoryFs::default()),
        cx,
    );
    let gate = Gate::pending();
    scene.os.held_next(Ok(OsId::new("debian")), gate.clone());
    scene.scan(cx);
    until(cx, |_| scene.os.calls() == 1);
    let mut renamed = scene.profile.clone();
    renamed.name = "Renamed server".into();
    let receipt = scene
        .installed
        .connections()
        .update(cx, |connections, cx| connections.save_profile(renamed, cx));
    cx.run_until_parked();
    futures::executor::block_on(receipt).unwrap();
    scene.scan(cx);
    gate.release();
    until(cx, |cx| scene.slots(cx) == 32);
    assert_eq!(scene.os(cx).as_deref(), Some("debian"));
    assert_eq!(scene.os.calls(), 1);
    assert_eq!(
        item.read_with(cx, |item, cx| nocterm_workspace::Item::session_spec(
            item, cx
        )
        .unwrap()
        .profile),
        Some(scene.profile.id.to_string().into())
    );
}

#[gpui_kit::test]
fn disconnected_original_country_source_cannot_publish_or_start_flag_after_native_return(
    cx: &mut TestAppContext,
) {
    use nocterm_connections_model::{
        observation::{CountryCode, PublicAddress},
        test_support::{BlockingGate, ObservationCall},
    };
    let scene = Scene::new(true, cx);
    let item = scene.add(
        scene.window,
        &scene.workspace,
        Arc::new(InMemoryFs::default()),
        cx,
    );
    let country = Arc::new(BlockingGate::default());
    let _release = country.release_on_drop();
    scene
        .script
        .resolve_next(Ok(PublicAddress::new("8.8.8.8".parse().unwrap())));
    scene
        .script
        .country_next(Ok(Some(CountryCode::new("de").unwrap())));
    scene.script.suspend_next_country(country.clone());
    scene.scan(cx);
    until(cx, |_| country.entered() == 1);
    // The Item retains FS and no source observer is installed; live original receipt must refuse.
    item.update(cx, |item, cx| item.set_connected(false, cx).unwrap());
    country.release();
    until(cx, |cx| scene.slots(cx) == 32);
    assert!(scene.installed.facts().read_with(cx, |facts, cx| {
        facts.country(scene.profile.id, cx).is_none()
    }));
    assert!(
        !scene
            .script
            .calls()
            .iter()
            .any(|call| matches!(call, ObservationCall::Flag(_)))
    );
}

#[gpui_kit::test]
fn closed_original_items_with_retained_filesystems_leave_no_admission_metadata(
    cx: &mut TestAppContext,
) {
    let scene = Scene::new(false, cx);
    let retained: Arc<dyn RemoteFs> = Arc::new(InMemoryFs::default());
    for expected_calls in 1..=48 {
        let item = scene.add(scene.window, &scene.workspace, retained.clone(), cx);
        scene.os.answer_next(Ok(None));
        scene.scan(cx);
        until(cx, |cx| scene.slots(cx) == 32);
        assert_eq!(scene.os.calls(), expected_calls);
        assert_eq!(
            scene
                .installed
                .facts()
                .read_with(cx, |facts, _| facts.workload().retained_sources()),
            1
        );
        cx.update_window(scene.window, |_, window, cx| {
            scene
                .workspace
                .update(cx, |workspace, cx| workspace.close_item(0, window, cx))
        })
        .unwrap();
        // Keep the original Entity and FS alive while actual close marks its session closed.
        assert!(!item.read_with(cx, |item, cx| {
            nocterm_workspace::Item::session(item, cx)
                .unwrap()
                .connected
        }));
        scene.scan(cx);
        assert_eq!(
            scene
                .installed
                .facts()
                .read_with(cx, |facts, _| facts.workload().retained_sources()),
            0
        );
        assert_eq!(
            scene
                .installed
                .facts()
                .read_with(cx, |facts, _| facts.workload().physical_jobs()),
            0
        );
        drop(item);
    }
}

#[gpui_kit::test]
fn profile_association_changes_preserve_same_capability_once_marker(cx: &mut TestAppContext) {
    let scene = Scene::new(false, cx);
    let item = scene.add(
        scene.window,
        &scene.workspace,
        Arc::new(InMemoryFs::default()),
        cx,
    );
    scene.os.answer_next(Ok(None));
    scene.scan(cx);
    until(cx, |cx| scene.slots(cx) == 32);
    item.update(cx, |item, cx| item.associate_profile(None, cx));
    scene.scan(cx);
    assert_eq!(
        scene
            .installed
            .facts()
            .read_with(cx, |facts, _| facts.workload().retained_sources()),
        1
    );
    item.update(cx, |item, cx| {
        item.associate_profile(Some(scene.profile.id.to_string().into()), cx)
    });
    scene.scan(cx);
    cx.run_until_parked();
    assert_eq!(scene.os.calls(), 1);
}
