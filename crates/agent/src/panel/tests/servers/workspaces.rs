//! Explicit saved-server attachments stay in the chat's original folder surface.
use super::*;
use nocterm_workspace::ProjectSurfaceReceipt;

fn project_chat(f: &Fixture, cx: &mut TestAppContext) -> ProjectSurfaceReceipt {
    let project = nocterm_project_model::ProjectId::from_bytes([1; 16]);
    let source = cx
        .update_window(f.handle, |_, window, cx| {
            let (_, surface, _) = super::super::seat_binding::project_seat(f, project, window, cx);
            surface.source(cx)
        })
        .unwrap();
    cx.run_until_parked();
    new_chat(f, cx);
    source
}
fn attach(f: &Fixture, row: &str, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-attach-context", cx);
        window.render_frame(cx);
        window.click(row.to_owned(), cx);
        if row.starts_with("attach-") {
            window.press("escape", cx);
            window.render_frame(cx);
            assert!(window.try_find("agent-picker").is_none());
        }
    })
    .unwrap();
}
fn replace_project(f: &Fixture, source: ProjectSurfaceReceipt, window: &mut Window, cx: &mut App) {
    f.workspace.update(cx, |workspace, cx| {
        let index = workspace
            .items(cx)
            .position(|item| Some(item.item_id()) == source.outer_item())
            .unwrap();
        workspace.close_item(index, window, cx);
    });
    let nocterm_ai::history::ContextId::Project(project) = source.context() else {
        panic!("project fixture")
    };
    super::super::seat_binding::project_seat(f, project, window, cx);
}

#[gpui_kit::test]
fn a_workspace_chat_can_attach_and_open_an_additional_saved_server(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let directory = install(&f, cx);
    let source = project_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    attach(&f, "attach-connection-web", cx);
    let server = cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            let servers = thread.offline_servers(cx);
            assert_eq!(servers.len(), 1);
            assert_eq!(servers[0].1.id.as_ref(), "web");
            servers[0].0.server_id.clone()
        })
    });
    let response = open_server(&f, &thread, &server, cx);
    let item = directory.background.borrow()[0];
    cx.update(|cx| {
        let workspace = f.workspace.read(cx);
        let entries = workspace.context_terminals(source, cx).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].item, item);
        assert!(entries[0].background);
        assert_eq!(
            workspace
                .context_terminals(workspace.general_surface_source(cx), cx)
                .unwrap()
                .len(),
            1
        );
        thread.update(cx, |thread, cx| {
            assert!(
                thread
                    .resolved(cx)
                    .iter()
                    .any(|(id, _, _)| id == response["terminal"]["id"].as_str().unwrap())
            );
            assert_eq!(thread.background_items(), vec![item]);
        });
    });
    assert_eq!(directory.opened.get(), 0);
}

#[gpui_kit::test]
fn a_workspace_group_attachment_exposes_its_saved_servers(cx: &mut TestAppContext) {
    let f = fixture(cx);
    install(&f, cx);
    project_chat(&f, cx);
    attach(&f, "attach-group-prod", cx);
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        let ids = thread.update(cx, |thread, cx| {
            thread
                .offline_servers(cx)
                .into_iter()
                .map(|(_, summary)| summary.id.to_string())
                .collect::<Vec<_>>()
        });
        assert_eq!(ids, ["web", "db"]);
    });
}

#[gpui_kit::test]
fn the_saved_server_arrow_opens_a_tab_inside_its_original_workspace(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let directory = install(&f, cx);
    let source = project_chat(&f, cx);
    attach(&f, "open-connection-web", cx);
    cx.run_until_parked();
    let item = directory.background.borrow()[0];
    cx.update(|cx| {
        let workspace = f.workspace.read(cx);
        let entries = workspace.context_terminals(source, cx).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].item, item);
        assert!(!entries[0].background);
        assert_eq!(workspace.items(cx).count(), 2);
        assert_eq!(workspace.context_source(cx), source);
    });
    assert_eq!(directory.opened.get(), 0);
}

#[gpui_kit::test]
fn an_approved_saved_server_open_cannot_target_a_reopened_workspace(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let directory = install(&f, cx);
    let source = project_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    attach(&f, "attach-connection-web", cx);
    let server = cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.offline_servers(cx)[0].0.server_id.clone()
        })
    });
    let mut response = queue_server_open(&f, &thread, &server, cx);
    let card = cx.update(|cx| thread.read(cx).tool_cards().remove(0));
    cx.update_window(f.handle, |_, window, cx| {
        thread.update(cx, |thread, cx| {
            thread.approve_tool(card.token(), true, false, cx)
        });
        replace_project(&f, source, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(response.try_recv().unwrap().expect("answered").is_err());
    assert!(directory.background.borrow().is_empty());
    assert!(directory.accesses.borrow().is_empty());
    cx.update(|cx| {
        let workspace = f.workspace.read(cx);
        assert_ne!(workspace.context_source(cx), source);
        assert_eq!(workspace.selected_context(cx), source.context());
        assert!(workspace.terminals(cx).is_empty());
        assert_eq!(
            workspace
                .context_terminals(workspace.general_surface_source(cx), cx)
                .unwrap()
                .len(),
            1
        );
    });
}

#[gpui_kit::test]
fn a_saved_server_arrow_cannot_target_a_reopened_workspace(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let directory = install(&f, cx);
    let source = project_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-attach-context", cx);
        window.render_frame(cx);
        window.click("open-connection-web", cx);
        replace_project(&f, source, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(directory.background.borrow().is_empty());
    assert!(directory.accesses.borrow().is_empty());
    cx.update(|cx| {
        let workspace = f.workspace.read(cx);
        assert_ne!(workspace.context_source(cx), source);
        assert!(workspace.terminals(cx).is_empty());
    });
}

#[gpui_kit::test]
fn failed_chat_handoff_keeps_a_workspace_session_the_user_already_promoted(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let directory = install(&f, cx);
    let source = project_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    attach(&f, "attach-connection-web", cx);
    let server = cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.offline_servers(cx)[0].0.server_id.clone()
        })
    });
    let weak = Rc::downgrade(&directory);
    let owner = thread.clone();
    let seat = cx.update(|cx| thread.read(cx).workspace_seat().clone());
    *directory.after_background_open.borrow_mut() = Some(Box::new(move |window, cx| {
        let item = weak.upgrade().unwrap().background.borrow()[0];
        seat.show_background(item, window, cx);
        owner.update(cx, |thread, cx| {
            thread.attach(Attachment::Connection("web".into()), cx)
        });
    }));
    let mut response = queue_server_open(&f, &thread, &server, cx);
    let card = cx.update(|cx| thread.read(cx).tool_cards().remove(0));
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.approve_tool(card.token(), true, false, cx)
        })
    });
    cx.run_until_parked();
    assert!(response.try_recv().unwrap().expect("answered").is_err());
    cx.update(|cx| {
        let entries = f.workspace.read(cx).context_terminals(source, cx).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].background);
        assert_eq!(entries[0].item, directory.background.borrow()[0]);
    });
}
