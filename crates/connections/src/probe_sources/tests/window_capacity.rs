use super::*;
#[gpui_kit::test]
fn two_windows_reproject_original_sources_when_physical_capacity_is_released(
    cx: &mut TestAppContext,
) {
    let scene = Scene::new(false, cx);
    let mut gates = Vec::new();
    for _ in 0..32 {
        let gate = Gate::pending();
        scene.os.held_next(Ok(None), gate.clone());
        scene.add(
            scene.window,
            &scene.workspace,
            Arc::new(InMemoryFs::default()),
            cx,
        );
        gates.push(gate);
    }
    // Fair retries use the actual Installed service capacity subscription.
    for _ in 0..32 {
        scene.scan(cx);
    }
    until(cx, |_| scene.os.calls() == 32);
    assert_eq!(scene.slots(cx), 0);
    let (window, workspace) = scene.second_window(cx);
    let item = scene.add(window, &workspace, Arc::new(InMemoryFs::default()), cx);
    scene.os.answer_next(Ok(OsId::new("arch")));
    scene.scan(cx);
    assert_eq!(scene.os.calls(), 32);
    gates[0].release();
    until(cx, |_| scene.os.calls() == 33);
    assert_eq!(scene.os.sessions().len(), 33);
    for gate in gates {
        gate.release();
    }
    until(cx, |cx| scene.slots(cx) == 32);
    assert_eq!(scene.os(cx).as_deref(), Some("arch"));
    assert!(item.read_with(cx, |item, cx| {
        nocterm_workspace::Item::session(item, cx)
            .unwrap()
            .connected
    }));
}

#[gpui_kit::test]
fn fully_registered_items_changes_scan_after_workspace_update_has_returned(
    cx: &mut TestAppContext,
) {
    let scene = Scene::new(false, cx);
    cx.update_window(scene.window, |_, window, cx| {
        scene.workspace.update(cx, |workspace, cx| {
            crate::register(&scene.installed, workspace, window, cx)
        })
    })
    .unwrap();
    scene.os.answer_next(Ok(OsId::new("debian")));
    let item = scene.add(
        scene.window,
        &scene.workspace,
        Arc::new(InMemoryFs::default()),
        cx,
    );
    until(cx, |cx| scene.os(cx).as_deref() == Some("debian"));
    let mut profile = scene.profile.clone();
    profile.name = "Profile changed".into();
    let receipt = scene
        .installed
        .connections()
        .update(cx, |connections, cx| connections.save_profile(profile, cx));
    cx.run_until_parked();
    futures::executor::block_on(receipt).unwrap();
    cx.update_window(scene.window, |_, window, cx| {
        scene
            .workspace
            .update(cx, |workspace, cx| workspace.close_item(0, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(scene.os.calls(), 1);
    assert!(!item.read_with(cx, |item, cx| {
        nocterm_workspace::Item::session(item, cx)
            .unwrap()
            .connected
    }));
    cx.update_window(scene.window, |_, window, _| window.remove_window())
        .unwrap();
    drop((item, scene));
    cx.run_until_parked();
}
