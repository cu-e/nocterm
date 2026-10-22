use super::{support::*, *};

#[gpui_kit::test]
fn closing_a_workspace_preserves_the_saved_definition_and_reopening_gets_a_new_surface(
    cx: &mut TestAppContext,
) {
    let rig = Fixture::loaded(vec![definition(1)], cx);
    let original = cx
        .update_window(rig.window, |_, window, cx| {
            rig.workspace.update(cx, |workspace, cx| {
                rig.installed
                    .open(definition(1).id(), workspace, window, cx)
                    .unwrap();
                let source = workspace.context_source(cx);
                workspace.close_item(0, window, cx);
                assert_eq!(workspace.items(cx).count(), 0);
                assert_eq!(rig.installed.catalog(cx).iter().count(), 1);
                source
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        rig.workspace.update(cx, |workspace, cx| {
            rig.installed
                .open(definition(1).id(), workspace, window, cx)
                .unwrap();
            assert_eq!(workspace.items(cx).count(), 1);
            assert_eq!(
                workspace.selected_context(cx),
                ContextId::Project(definition(1).id())
            );
            assert_ne!(workspace.context_source(cx), original);
            assert!(workspace.context_terminals(original, cx).is_err());
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn opening_an_already_owned_workspace_in_another_window_focuses_the_original_item(
    cx: &mut TestAppContext,
) {
    let rig = Fixture::loaded(vec![definition(1), definition(2)], cx);
    cx.update_window(rig.window, |_, window, cx| {
        rig.workspace.update(cx, |workspace, cx| {
            rig.installed
                .open(definition(1).id(), workspace, window, cx)
                .unwrap();
            rig.installed
                .open(definition(2).id(), workspace, window, cx)
                .unwrap();
        });
    })
    .unwrap();
    let (second, workspace_b) = open_window(&rig.installed, cx);
    cx.update_window(second, |_, window, cx| {
        workspace_b.update(cx, |workspace, cx| {
            rig.installed
                .open(definition(1).id(), workspace, window, cx)
                .unwrap();
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(workspace_b.read(cx).items(cx).count(), 0);
        assert_eq!(rig.workspace.read(cx).items(cx).count(), 2);
        assert_eq!(
            rig.workspace.read(cx).selected_context(cx),
            ContextId::Project(definition(1).id())
        );
    });
    cx.update_window(rig.window, |_, window, _| {
        assert!(window.is_window_active())
    })
    .unwrap();
}

#[gpui_kit::test]
fn leaving_an_unavailable_workspace_focuses_its_original_server_without_closing_it(
    cx: &mut TestAppContext,
) {
    let host = remote(Some("a"), "a.example");
    let rig = Fixture::loaded(vec![on_host(1, host.clone())], cx);
    let parent = rig.parent(host, "Server A", cx);
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
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("project-unavailable").is_some());
        window.click("project-leave", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let workspace = rig.workspace.read(cx);
        assert_eq!(
            workspace.active_item(cx).unwrap().item_id(),
            parent.item.entity_id()
        );
        assert_eq!(workspace.selected_context(cx), ContextId::General);
        assert_eq!(workspace.items(cx).count(), 2);
        assert!(workspace.context_terminals(source, cx).unwrap().is_empty());
        assert!(
            parent
                .item
                .terminal_capabilities(cx)
                .unwrap()
                .state()
                .info(cx)
                .unwrap()
                .status
                == nocterm_workspace::TerminalStatus::Connected
        );
    });
}

#[gpui_kit::test]
fn cancelling_removal_preserves_the_definition_and_original_open_item(cx: &mut TestAppContext) {
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
        window.click("project-remove-01010101010101010101010101010101", cx);
        window.render_frame(cx);
        window.click("cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(rig.installed.catalog(cx).iter().count(), 1);
        assert_eq!(rig.workspace.read(cx).items(cx).count(), 1);
        assert_eq!(rig.workspace.read(cx).context_source(cx), source);
    });
}

#[gpui_kit::test]
fn removing_saved_settings_keeps_an_open_orphan_scoped_to_its_project(cx: &mut TestAppContext) {
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
        window.click("project-remove-01010101010101010101010101010101", cx);
        window.render_frame(cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .try_find("project-open-01010101010101010101010101010101")
                .is_none()
        );
        let workspace = rig.workspace.read(cx);
        assert_eq!(rig.installed.catalog(cx).iter().count(), 0);
        assert_eq!(workspace.items(cx).count(), 1);
        assert_eq!(workspace.context_source(cx), source);
        assert_eq!(
            workspace.selected_context(cx),
            ContextId::Project(definition(1).id())
        );
        assert!(window.try_find("project-unavailable").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_pending_cross_window_focus_cannot_focus_a_reopened_replacement(cx: &mut TestAppContext) {
    let rig = Fixture::loaded(vec![definition(1), definition(2)], cx);
    let old = cx
        .update_window(rig.window, |_, window, cx| {
            rig.workspace.update(cx, |workspace, cx| {
                rig.installed
                    .open(definition(1).id(), workspace, window, cx)
                    .unwrap();
                let old = workspace.active_item(cx).unwrap().item_id();
                rig.installed
                    .open(definition(2).id(), workspace, window, cx)
                    .unwrap();
                old
            })
        })
        .unwrap();
    let (second, workspace_b) = open_window(&rig.installed, cx);
    cx.update(|cx| {
        second
            .update(cx, |_, window, cx| {
                workspace_b.update(cx, |workspace, cx| {
                    rig.installed
                        .open(definition(1).id(), workspace, window, cx)
                        .unwrap();
                })
            })
            .unwrap();
        rig.window
            .update(cx, |_, window, cx| {
                rig.workspace.update(cx, |workspace, cx| {
                    let ordinal = workspace
                        .items(cx)
                        .position(|item| item.item_id() == old)
                        .unwrap();
                    workspace.close_item(ordinal, window, cx);
                    rig.installed
                        .open(definition(1).id(), workspace, window, cx)
                        .unwrap();
                    assert_ne!(workspace.active_item(cx).unwrap().item_id(), old);
                    workspace.activate_item(0, window, cx);
                })
            })
            .unwrap();
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(workspace_b.read(cx).items(cx).count(), 0);
        assert_eq!(
            rig.workspace.read(cx).selected_context(cx),
            ContextId::Project(definition(2).id())
        );
    });
}

#[gpui_kit::test]
fn removing_an_imported_definition_never_mutates_the_server_files(cx: &mut TestAppContext) {
    let host = remote(Some("a"), "a.example");
    let mut draft = on_host(1, host.clone()).to_draft();
    draft.source = ProjectSource::Imported {
        provider: "external-provider".into(),
        deployment: "deployment-a".into(),
        account: "account-a".into(),
        external_id: "external-workspace".into(),
    };
    let rig = Fixture::loaded(vec![draft.try_into().unwrap()], cx);
    let files = Arc::new(nocterm_session::testing::InMemoryFs::default());
    files
        .seed_file("/srv/project-1/main.rs", b"keep project files".to_vec())
        .unwrap();
    let _parent = rig.parent_with_fs(host, "Server A", Some(files.clone()), cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-remove-01010101010101010101010101010101", cx);
        window.render_frame(cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(rig.installed.catalog(cx).iter().count(), 0));
    assert_eq!(
        files.bytes("/srv/project-1/main.rs").unwrap(),
        b"keep project files"
    );
    assert_eq!(files.activity().requests, 0);
}

#[gpui_kit::test]
fn failed_definition_removal_preserves_the_open_item_and_saved_row(cx: &mut TestAppContext) {
    let (writer, release, _) = nocterm_store::test_support::gated_writer().into_parts();
    let rig = Fixture::new(vec![definition(1)], writer, cx);
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
        window.click("project-remove-01010101010101010101010101010101", cx);
        window.render_frame(cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.run_until_parked();
    release
        .send(Err(nocterm_store::StoreError::backend(
            std::io::Error::other("disk full"),
        )))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .try_find("project-open-01010101010101010101010101010101")
                .is_some()
        );
        assert_eq!(rig.installed.catalog(cx).iter().count(), 1);
        assert_eq!(rig.workspace.read(cx).context_source(cx), source);
        assert_eq!(rig.workspace.read(cx).items(cx).count(), 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn closing_the_original_owner_allows_another_window_to_open_its_own_item(cx: &mut TestAppContext) {
    let rig = Fixture::loaded(vec![definition(1)], cx);
    cx.update_window(rig.window, |_, window, cx| {
        rig.workspace.update(cx, |workspace, cx| {
            rig.installed
                .open(definition(1).id(), workspace, window, cx)
                .unwrap();
        });
    })
    .unwrap();
    let (second, workspace_b) = open_window(&rig.installed, cx);
    cx.update_window(rig.window, |_, window, cx| {
        rig.workspace
            .update(cx, |workspace, cx| workspace.close_item(0, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(second, |_, window, cx| {
        workspace_b.update(cx, |workspace, cx| {
            rig.installed
                .open(definition(1).id(), workspace, window, cx)
                .unwrap();
            assert_eq!(workspace.items(cx).count(), 1);
            assert_eq!(
                workspace.selected_context(cx),
                ContextId::Project(definition(1).id())
            );
        });
    })
    .unwrap();
    cx.update(|cx| assert_eq!(rig.workspace.read(cx).items(cx).count(), 0));
}
