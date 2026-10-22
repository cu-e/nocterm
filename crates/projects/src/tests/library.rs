use super::{support::*, *};

#[gpui_kit::test]
fn current_group_matches_saved_profile_identity_and_not_equal_server_titles(
    cx: &mut TestAppContext,
) {
    let current = remote(Some("stable-profile"), "old-address.example");
    let other = remote(Some("another-profile"), "new-address.example");
    let rig = Fixture::loaded(vec![on_host(1, current), on_host(2, other)], cx);
    let _parent = rig.parent(
        remote(Some("stable-profile"), "new-address.example"),
        "Same name",
        cx,
    );
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("project-context").label(), Some("Same name"));
        window.click("projects-other", cx);
        window.render_frame(cx);
        assert!(
            window
                .try_find("project-open-01010101010101010101010101010101")
                .is_some()
        );
        assert!(
            window
                .try_find("project-open-02020202020202020202020202020202")
                .is_none()
        );
        assert_eq!(rig.installed.catalog(cx).iter().count(), 2);
    })
    .unwrap();
}

#[gpui_kit::test]
fn direct_connections_with_equal_titles_keep_distinct_current_groups(cx: &mut TestAppContext) {
    let a = remote(None, "a.example");
    let b = remote(None, "b.example");
    let rig = Fixture::loaded(vec![on_host(1, a.clone()), on_host(2, b.clone())], cx);
    let _first = rig.parent(a, "Identical title", cx);
    let _second = rig.parent(b, "Identical title", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("projects-other", cx);
        window.render_frame(cx);
        assert!(
            window
                .try_find("project-open-01010101010101010101010101010101")
                .is_none()
        );
        assert!(
            window
                .try_find("project-open-02020202020202020202020202020202")
                .is_some()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn disconnect_keeps_current_saved_rows_visible_but_disables_creation(cx: &mut TestAppContext) {
    let host = remote(Some("a"), "a.example");
    let rig = Fixture::loaded(vec![on_host(1, host.clone()), definition(2)], cx);
    let parent = rig.parent(host, "Server A", cx);
    cx.update(|cx| {
        parent
            .item
            .update(cx, |item, cx| item.set_connected(false, cx).unwrap())
    });
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("projects-other", cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        assert_eq!(window.find("project-context").label(), Some("Server A"));
        assert!(
            window
                .try_find("project-open-01010101010101010101010101010101")
                .is_some()
        );
        assert!(window.try_find("project-name").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn saved_metadata_can_be_edited_offline_without_opening_a_terminal(cx: &mut TestAppContext) {
    let original = on_host(1, remote(Some("offline"), "offline.example"));
    let rig = Fixture::loaded(vec![original.clone()], cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-edit-01010101010101010101010101010101", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Renamed offline", cx);
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = rig.installed.catalog(cx).get(original.id()).unwrap();
        assert_eq!(saved.name(), "Renamed offline");
        assert_eq!(saved.host(), original.host());
        assert_eq!(saved.root(), original.root());
        assert_eq!(rig.workspace.read(cx).items(cx).count(), 0);
    });
}

#[gpui_kit::test]
fn an_offline_editor_refuses_to_overwrite_a_newer_saved_definition(cx: &mut TestAppContext) {
    let original = definition(1);
    let rig = Fixture::loaded(vec![original.clone()], cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-edit-01010101010101010101010101010101", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Older editor draft", cx);
    })
    .unwrap();
    let mut newer = original.to_draft();
    newer.name = "Published elsewhere".into();
    cx.update(|cx| {
        rig.model
            .save(newer.try_into().unwrap(), Some(original), cx)
    })
    .detach();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("project-form-error").is_some());
        assert!(window.try_find("project-name").is_some());
        assert_eq!(
            rig.installed
                .catalog(cx)
                .get(definition(1).id())
                .unwrap()
                .name(),
            "Published elsewhere"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_cancelled_pending_save_cannot_close_a_later_editor(cx: &mut TestAppContext) {
    let (writer, release, _) = nocterm_store::test_support::gated_writer().into_parts();
    let rig = Fixture::new(vec![definition(1), definition(2)], writer, cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-edit-01010101010101010101010101010101", cx);
        window.render_frame(cx);
        fill(window, "project-name", "First accepted save", cx);
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-cancel", cx);
        window.render_frame(cx);
        window.click("project-edit-02020202020202020202020202020202", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Second editor draft", cx);
    })
    .unwrap();
    release.send(Ok(())).unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("project-name").is_some());
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            rig.installed
                .catalog(cx)
                .get(definition(1).id())
                .unwrap()
                .name(),
            "First accepted save"
        );
        assert_eq!(
            rig.installed
                .catalog(cx)
                .get(definition(2).id())
                .unwrap()
                .name(),
            "Second editor draft"
        );
    });
}

#[gpui_kit::test]
fn renaming_saved_metadata_updates_the_open_tab_without_replacing_its_scope(
    cx: &mut TestAppContext,
) {
    let rig = Fixture::loaded(vec![definition(1)], cx);
    let source = cx
        .update_window(rig.window, |_, window, cx| {
            rig.workspace.update(cx, |workspace, cx| {
                rig.installed
                    .open(definition(1).id(), workspace, window, cx)
                    .unwrap();
                workspace.context_source(cx)
            })
        })
        .unwrap();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-edit-01010101010101010101010101010101", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Renamed live tab", cx);
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let workspace = rig.workspace.read(cx);
        assert_eq!(
            workspace.active_item(cx).unwrap().tab_title(cx).as_ref(),
            "Renamed live tab"
        );
        assert_eq!(workspace.context_source(cx), source);
        assert_eq!(
            workspace.selected_context(cx),
            ContextId::Project(definition(1).id())
        );
    });
}
