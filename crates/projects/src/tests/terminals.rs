//! Ordinary folder terminals belong to their retained outer workspace Item.
use super::{support::*, *};
use nocterm_workspace::test_support::ProjectTerminalProbe;

struct FolderFixture {
    rig: Fixture,
    terminals: ProjectTerminalProbe,
    parent: ParentConnection,
}
impl FolderFixture {
    fn new(cx: &mut TestAppContext) -> Self {
        let host = remote(Some("server-a"), "a.example");
        let terminals = ProjectTerminalProbe::with_sessions(nocterm_workspace::SessionSpec {
            profile: Some("server-a".into()),
            options: Default::default(),
            title: "Folder terminal".into(),
            target: nocterm_session::Target::new("alice", "a.example", 22),
            auth: Default::default(),
            launch: None,
            credential: None,
            jumps: vec![],
        });
        let rig = Fixture::with_ports(
            vec![on_host(1, host.clone()), on_host(2, host.clone())],
            Arc::new(|_| Box::pin(async { Ok(()) })),
            Rc::new(terminals.clone()),
            Some(Rc::new(terminals.clone())),
            cx,
        );
        let parent = rig.parent(host, "Server A", cx);
        Self {
            rig,
            terminals,
            parent,
        }
    }
    fn open(&self, value: u8, cx: &mut TestAppContext) {
        cx.update_window(self.rig.window, |_, window, cx| {
            self.rig.workspace.update(cx, |workspace, cx| {
                self.rig
                    .installed
                    .open(definition(value).id(), workspace, window, cx)
                    .unwrap();
            })
        })
        .unwrap();
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn a_saved_folder_opens_one_ordinary_terminal_and_repeated_open_reuses_it(cx: &mut TestAppContext) {
    let f = FolderFixture::new(cx);
    f.open(1, cx);
    f.open(1, cx);
    cx.update_window(f.rig.window, |_, window, cx| {
        window.render_frame(cx);
        let workspace = f.rig.workspace.read(cx);
        assert_eq!(workspace.items(cx).count(), 2);
        assert_eq!(
            workspace.selected_context(cx),
            ContextId::Project(definition(1).id())
        );
        assert_eq!(workspace.terminals(cx).len(), 1);
        assert!(window.try_find("project-unavailable").is_none());
        assert_eq!(f.terminals.attempts().len(), 1);
        assert_eq!(f.terminals.attempts()[0].1, *definition(1).root());
        assert!(f.terminals.resolutions().is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn opening_a_workspace_prefers_the_focused_matching_connection_over_an_earlier_tab(
    cx: &mut TestAppContext,
) {
    let f = FolderFixture::new(cx);
    let selected = f.rig.parent(
        remote(Some("server-a"), "a.example"),
        "Selected session",
        cx,
    );
    cx.update(|cx| {
        for (parent, marker) in [(&f.parent, "earlier"), (&selected, "selected")] {
            let mut spec = parent.item.session_spec(cx).unwrap();
            spec.launch = Some(nocterm_session::ShellLaunch {
                program: Some("/bin/sh".into()),
                args: vec!["-l".into()],
                env: [("SESSION_MARKER".into(), marker.into())]
                    .into_iter()
                    .collect(),
                ..Default::default()
            });
            parent.item.update(cx, |item, cx| {
                item.replace_session(spec, None, true, cx).unwrap()
            });
        }
    });
    cx.update_window(f.rig.window, |_, window, cx| {
        f.rig.workspace.update(cx, |workspace, cx| {
            assert!(workspace.focus_terminal(selected.item.entity_id(), window, cx));
        });
    })
    .unwrap();
    f.open(1, cx);
    cx.update_window(f.rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("project-server").label(),
            Some("Selected session")
        );
        window.click("project-leave", cx);
        assert_eq!(
            f.rig.workspace.read(cx).active_item(cx).unwrap().item_id(),
            selected.item.entity_id()
        );
    })
    .unwrap();
    assert_eq!(f.terminals.attempts().len(), 1);
    assert!(f.terminals.resolutions().is_empty());
}

#[gpui_kit::test]
fn opening_a_saved_workspace_without_a_parent_opens_its_routed_terminal_inside_the_workspace(
    cx: &mut TestAppContext,
) {
    let target = nocterm_session::Target::new("alice", "a.example", 22);
    let via = vec![
        nocterm_session::Target::new("jump", "first.example", 2201),
        nocterm_session::Target::new("jump", "second.example", 2202),
    ];
    let host = HostLocator::Ssh {
        profile: "server-a".into(),
        target: target.clone(),
        via: via.clone(),
    };
    let terminals = ProjectTerminalProbe::with_sessions(nocterm_workspace::SessionSpec {
        profile: Some("server-a".into()),
        options: Default::default(),
        title: "Terminal".into(),
        target,
        auth: Default::default(),
        launch: None,
        credential: None,
        jumps: via
            .iter()
            .cloned()
            .map(|target| nocterm_session::ResolvedJump {
                host: nocterm_session::JumpHost {
                    target,
                    auth: Default::default(),
                },
                credential: None,
            })
            .collect(),
    });
    let rig = Fixture::with_ports(
        vec![on_host(1, host.clone())],
        Arc::new(|_| Box::pin(async { Ok(()) })),
        Rc::new(terminals.clone()),
        Some(Rc::new(terminals.clone())),
        cx,
    );
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-open-01010101010101010101010101010101", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        let workspace = rig.workspace.read(cx);
        assert_eq!(workspace.items(cx).count(), 1);
        assert_eq!(workspace.terminals(cx).len(), 1);
        assert!(
            workspace
                .context_terminals(workspace.general_surface_source(cx), cx)
                .unwrap()
                .is_empty()
        );
        assert!(window.try_find("project-connect").is_none());
        assert!(window.try_find("project-unavailable").is_none());
    })
    .unwrap();
    assert_eq!(terminals.resolutions(), vec![host]);
    assert_eq!(terminals.attempts().len(), 1);
    assert_eq!(terminals.attempts()[0].1, *definition(1).root());
}

#[gpui_kit::test]
fn the_header_and_sidebar_keep_the_server_caption_when_the_inner_terminal_is_focused(
    cx: &mut TestAppContext,
) {
    let f = FolderFixture::new(cx);
    f.open(1, cx);
    cx.update_window(f.rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("project-server").label(), Some("Server A"));
        assert_eq!(window.find("project-context").label(), Some("Server A"));
        let name = window.find("project-row-name-01010101010101010101010101010101");
        let server = window.find("project-row-server-01010101010101010101010101010101");
        let row = window.find("project-row-01010101010101010101010101010101");
        assert_eq!(name.label(), Some("Workspace 1"));
        assert_eq!(server.label(), Some("Server A"));
        assert_eq!(name.bounds().left(), server.bounds().left());
        assert!(name.bounds().left() < row.bounds().center().x);
        assert!(server.bounds().top() >= name.bounds().bottom());
        assert!(f.terminals.resolutions().is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_unavailable_definition_has_one_compact_retry_action_without_a_separate_server_tab_action(
    cx: &mut TestAppContext,
) {
    let rig = Fixture::loaded(vec![definition(1)], cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-open-01010101010101010101010101010101", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-unavailable").is_some());
        assert!(window.try_find("project-connect").is_none());
        let retry = window.find("project-retry");
        assert_eq!(retry.label(), Some("Retry"));
        assert!(retry.bounds().size.width < gpui_kit::px(160.0));
        assert_eq!(rig.workspace.read(cx).items(cx).count(), 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn retry_uses_a_newly_available_original_connection_from_the_active_workspace(
    cx: &mut TestAppContext,
) {
    let host = remote(Some("server-a"), "a.example");
    let terminals = ProjectTerminalProbe::default();
    let rig = Fixture::with_ports(
        vec![on_host(1, host.clone())],
        Arc::new(|_| Box::pin(async { Ok(()) })),
        Rc::new(terminals.clone()),
        None,
        cx,
    );
    let outer = cx
        .update_window(rig.window, |_, window, cx| {
            window.render_frame(cx);
            window.click("project-open-01010101010101010101010101010101", cx);
            rig.workspace.read(cx).active_item(cx).unwrap().item_id()
        })
        .unwrap();
    assert!(terminals.attempts().is_empty());
    let _parent = rig.parent(host, "Newly available", cx);
    cx.update_window(rig.window, |_, window, cx| {
        rig.workspace.update(cx, |workspace, cx| {
            assert!(workspace.activate_item_by_id(outer, window, cx));
        });
        window.render_frame(cx);
        window.click("project-retry", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(terminals.attempts().len(), 1);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("project-unavailable").is_none());
        assert_eq!(
            rig.workspace.read(cx).active_item(cx).unwrap().item_id(),
            outer
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn separate_workspaces_keep_their_original_inner_terminals_when_switching_outer_tabs(
    cx: &mut TestAppContext,
) {
    let f = FolderFixture::new(cx);
    f.open(1, cx);
    let (first, terminal) = cx.update(|cx| {
        let workspace = f.rig.workspace.read(cx);
        (
            workspace.context_source(cx),
            workspace.terminals(cx)[0].item,
        )
    });
    f.open(2, cx);
    cx.update(|cx| {
        let workspace = f.rig.workspace.read(cx);
        assert_eq!(workspace.items(cx).count(), 3);
        assert_ne!(workspace.terminals(cx)[0].item, terminal);
        assert_eq!(
            workspace.context_terminals(first, cx).unwrap()[0].item,
            terminal
        );
    });
    f.open(1, cx);
    cx.update(|cx| assert_eq!(f.rig.workspace.read(cx).terminals(cx)[0].item, terminal));
    assert_eq!(f.terminals.attempts().len(), 2);
}

#[gpui_kit::test]
fn closing_the_parent_connection_does_not_prevent_new_terminals_in_the_open_workspace(
    cx: &mut TestAppContext,
) {
    let f = FolderFixture::new(cx);
    f.open(1, cx);
    let source = cx.update(|cx| f.rig.workspace.read(cx).context_source(cx));
    cx.update_window(f.rig.window, |_, window, cx| {
        f.rig
            .workspace
            .update(cx, |workspace, cx| workspace.close_item(0, window, cx));
        window.render_frame(cx);
        window.click("project-terminal-new", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let workspace = f.rig.workspace.read(cx);
        assert_eq!(workspace.items(cx).count(), 1);
        assert_eq!(workspace.context_source(cx), source);
        assert_eq!(workspace.terminals(cx).len(), 2);
    });
    assert_eq!(f.terminals.attempts().len(), 2);
}

#[gpui_kit::test]
fn closing_the_last_inner_terminal_keeps_the_folder_tab_and_scope(cx: &mut TestAppContext) {
    let f = FolderFixture::new(cx);
    f.open(1, cx);
    let source = cx.update(|cx| f.rig.workspace.read(cx).context_source(cx));
    cx.update_window(f.rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(nocterm_workspace::CloseTab), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let workspace = f.rig.workspace.read(cx);
        assert_eq!(workspace.items(cx).count(), 2);
        assert_eq!(workspace.context_source(cx), source);
        assert_eq!(
            workspace.selected_context(cx),
            ContextId::Project(definition(1).id())
        );
        assert!(workspace.terminals(cx).is_empty());
    });
}

#[gpui_kit::test]
fn closing_and_immediately_reopening_replaces_the_surface_and_starts_one_terminal(
    cx: &mut TestAppContext,
) {
    let f = FolderFixture::new(cx);
    f.open(1, cx);
    let source = cx.update(|cx| f.rig.workspace.read(cx).context_source(cx));
    cx.update_window(f.rig.window, |_, window, cx| {
        f.rig.workspace.update(cx, |workspace, cx| {
            workspace.close_item(1, window, cx);
            f.rig
                .installed
                .open(definition(1).id(), workspace, window, cx)
                .unwrap();
            assert_eq!(workspace.items(cx).count(), 2);
            assert!(workspace.context_terminals(source, cx).is_err());
            assert_eq!(workspace.terminals(cx).len(), 1);
        });
        window.render_frame(cx);
        assert!(window.try_find("project-unavailable").is_none());
    })
    .unwrap();
    assert_eq!(f.terminals.attempts().len(), 2);
    cx.update(|cx| assert!(f.parent.item.terminal_capabilities(cx).is_some()));
}

#[gpui_kit::test]
async fn changing_the_saved_server_prevents_new_terminals_from_using_the_old_connection(
    cx: &mut TestAppContext,
) {
    let f = FolderFixture::new(cx);
    f.open(1, cx);
    let terminal = cx.update(|cx| f.rig.workspace.read(cx).terminals(cx)[0].item);
    let original = cx.update(|cx| {
        f.rig
            .model
            .catalog(cx)
            .get(definition(1).id())
            .unwrap()
            .clone()
    });
    let mut changed = ProjectDraft::from(original.clone());
    changed.host = remote(Some("server-b"), "b.example");
    cx.update(|cx| {
        f.rig
            .model
            .save(changed.try_into().unwrap(), Some(original), cx)
    })
    .await
    .unwrap();
    cx.run_until_parked();

    cx.update_window(f.rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("project-unavailable").is_some());
        window.click("project-terminal-new", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(f.terminals.attempts().len(), 1);
    cx.update(|cx| {
        let terminals = f.rig.workspace.read(cx).terminals(cx);
        assert_eq!(terminals.len(), 1);
        assert_eq!(terminals[0].item, terminal);
    });
}
