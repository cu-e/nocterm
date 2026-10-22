use super::{support::*, *};

#[gpui_kit::test]
fn creation_keeps_the_original_server_and_folder_after_another_connection_is_selected(
    cx: &mut TestAppContext,
) {
    let rig = Fixture::loaded(vec![], cx);
    let a = remote(Some("profile-a"), "a.example");
    let _parent_a = rig.parent(a.clone(), "Server A", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Original project", cx);
        assert!(window.try_find("project-form-server").is_some());
    })
    .unwrap();
    let _parent_b = rig.parent(remote(Some("profile-b"), "b.example"), "Server B", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = rig.installed.catalog(cx).iter().next().unwrap();
        assert_eq!(saved.name(), "Original project");
        assert_eq!(saved.host(), &a);
        assert_eq!(saved.root().as_str(), "/srv/original-folder");
        assert_eq!(rig.workspace.read(cx).items(cx).count(), 2);
    });
}

#[gpui_kit::test]
fn creation_uses_the_actual_direct_destination_and_only_name_and_folder_controls(
    cx: &mut TestAppContext,
) {
    let rig = Fixture::loaded(vec![], cx);
    let host = remote(None, "direct.example");
    let _parent = rig.parent(host.clone(), "Temporary connection", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Folder source", cx);
        fill(window, "project-path", "/srv/exact-folder", cx);
        assert!(window.try_find("project-writable").is_none());
        assert!(window.try_find("project-execution").is_none());
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = rig.installed.catalog(cx).iter().next().unwrap();
        assert_eq!(saved.host(), &host);
        assert_eq!(saved.root().as_str(), "/srv/exact-folder");
    });
}

#[gpui_kit::test]
fn a_captured_folder_draft_can_be_saved_after_its_original_parent_disconnects(
    cx: &mut TestAppContext,
) {
    let rig = Fixture::loaded(vec![], cx);
    let host = remote(Some("a"), "a.example");
    let parent = rig.parent(host.clone(), "Server A", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Saved folder", cx);
    })
    .unwrap();
    cx.update(|cx| {
        parent
            .item
            .update(cx, |item, cx| item.set_connected(false, cx).unwrap())
    });
    let _other = rig.parent(remote(Some("b"), "b.example"), "Server B", cx);
    cx.update_window(rig.window, |_, window, cx| window.click("project-save", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = rig.installed.catalog(cx).iter().next().unwrap();
        assert_eq!(saved.host(), &host);
        assert_eq!(saved.name(), "Saved folder");
    });
}

#[gpui_kit::test]
fn a_failed_save_keeps_the_literal_draft_for_a_successful_retry(cx: &mut TestAppContext) {
    let (writer, release, writes) = nocterm_store::test_support::gated_writer().into_parts();
    let rig = Fixture::new(vec![], writer, cx);
    let host = remote(Some("a"), "a.example");
    let _parent = rig.parent(host.clone(), "Server A", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Keep this draft", cx);
        fill(window, "project-path", "/srv/draft", cx);
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(writes.lock().unwrap().len(), 1);
    release
        .send(Err(nocterm_store::StoreError::backend(
            std::io::Error::other("disk full"),
        )))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("project-form-error").is_some());
        assert_eq!(rig.installed.catalog(cx).iter().count(), 0);
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = rig.installed.catalog(cx).iter().next().unwrap();
        assert_eq!(saved.name(), "Keep this draft");
        assert_eq!(saved.root().as_str(), "/srv/draft");
        assert_eq!(saved.host(), &host);
    });
}

#[gpui_kit::test]
fn the_filesystem_root_is_a_valid_folder_workspace(cx: &mut TestAppContext) {
    let rig = Fixture::loaded(vec![], cx);
    let host = remote(None, "server.example");
    let _parent = rig.parent(host.clone(), "Server", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Root folder", cx);
        fill(window, "project-path", "/", cx);
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = rig.installed.catalog(cx).iter().next().unwrap();
        assert_eq!(saved.root().as_str(), "/");
        assert_eq!(saved.host(), &host);
    });
}

#[gpui_kit::test]
fn invalid_relative_folder_does_not_publish_or_replace_the_draft(cx: &mut TestAppContext) {
    let rig = Fixture::loaded(vec![], cx);
    let _parent = rig.parent(remote(None, "a.example"), "Server A", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Valid name", cx);
        fill(window, "project-path", "relative/path", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.click("project-save", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-form-error").is_some());
        assert_eq!(rig.installed.catalog(cx).iter().count(), 0);
        fill(window, "project-path", "/srv/fixed", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("project-form-error").is_none());
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = rig.installed.catalog(cx).iter().next().unwrap();
        assert_eq!(saved.name(), "Valid name");
        assert_eq!(saved.root().as_str(), "/srv/fixed");
    });
}

#[gpui_kit::test]
fn editing_the_name_clears_its_error_before_the_draft_is_saved(cx: &mut TestAppContext) {
    let rig = Fixture::loaded(vec![], cx);
    let _parent = rig.parent(remote(None, "a.example"), "Server A", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.click("project-save", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-form-error").is_some());
        fill(window, "project-name", "Corrected name", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("project-form-error").is_none());
        assert_eq!(rig.installed.catalog(cx).iter().count(), 0);
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = rig.installed.catalog(cx).iter().next().unwrap();
        assert_eq!(saved.name(), "Corrected name");
        assert_eq!(saved.root().as_str(), "/srv/original-folder");
    });
}
