//! One attachment choice per saved server, and a permanent local shell choice.
use super::*;

#[gpui_kit::test]
fn several_tabs_of_one_server_have_one_attachment_choice_covering_every_session(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let directory = install(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        for profile in ["web", "web", "db"] {
            assert!(directory.open(profile, &f.workspace.downgrade(), window, cx));
        }
    })
    .unwrap();
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-attach-context", cx);
        let local = window.find("attach-local-terminal").bounds();
        let web = window.find("attach-connection-web").bounds();
        assert!(local.origin.y < window.find("Saved servers").bounds().origin.y);
        assert!(local.origin.y < web.origin.y);
        assert!(window.try_find("Active sessions").is_none());
        for entry in f.workspace.read(cx).terminals(cx) {
            assert!(
                window
                    .try_find(format!("attach-{:?}", entry.item))
                    .is_none()
            );
        }
        window.click("attach-connection-web", cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(
            thread
                .read(cx)
                .composer()
                .attachments()
                .contains(&Attachment::Connection("web".into()))
        );
        let resolved = thread.update(cx, |thread, cx| thread.resolved(cx));
        assert_eq!(
            resolved
                .iter()
                .filter(|(_, _, descriptor)| descriptor
                    .connection
                    .as_ref()
                    .is_some_and(|connection| connection.id == "web"))
                .count(),
            2
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn servers_with_the_same_display_name_keep_independent_attachment_choices(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let mut web = summary("web", None);
    let mut db = summary("db", None);
    web.name = "Same name".into();
    db.name = "Same name".into();
    cx.update(|cx| {
        f.workspace.update(cx, |workspace, _| {
            workspace.set_connection_directory(Rc::new(Directory(RefCell::new(vec![web, db]))))
        })
    });
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-attach-context", cx);
        assert!(window.find("attach-connection-web").visible());
        assert!(window.find("attach-connection-db").visible());
        window.click("attach-connection-db", cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(
            thread
                .read(cx)
                .composer()
                .attachments()
                .contains(&Attachment::Connection("db".into()))
        );
        assert!(
            !thread
                .read(cx)
                .composer()
                .attachments()
                .contains(&Attachment::Connection("web".into()))
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_closed_local_terminal_can_be_created_and_attached_from_the_first_row(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.workspace.update(cx, |workspace, cx| {
            workspace.close_item(0, window, cx);
            workspace.set_local_terminal_opener(|workspace, target, window, cx| {
                let terminal = cx
                    .new(|cx| FakeTerminal::new(Access::new(None, Rc::new(SignIn::default())), cx));
                workspace.add_local_terminal_at(terminal, target, window, cx);
            });
        });
    })
    .unwrap();
    new_chat(&f, cx);
    assert!(cx.update(|cx| f.workspace.read(cx).terminals(cx).is_empty()));
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-attach-context", cx);
        assert_eq!(
            window.find("attach-local-terminal").label(),
            Some("Local terminal")
        );
        assert!(
            f.workspace.read(cx).terminals(cx).is_empty(),
            "opening the menu must not create a session"
        );
        window.click("attach-local-terminal", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let entries = f.workspace.read(cx).terminals(cx);
        assert_eq!(entries.len(), 1);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(
            thread
                .read(cx)
                .composer()
                .attachments()
                .contains(&Attachment::Terminal(entries[0].item))
        );
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.click("agent-attach-context", cx);
        window.click("attach-local-terminal", cx);
        assert_eq!(
            f.workspace.read(cx).terminals(cx).len(),
            1,
            "existing shell is reused"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_server_row_can_detach_an_individually_attached_tab_then_attach_the_whole_server(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let directory = install(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        assert!(directory.open("web", &f.workspace.downgrade(), window, cx));
    })
    .unwrap();
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click("agent-attach-context", cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(
            thread
                .read(cx)
                .composer()
                .attachments()
                .iter()
                .any(|attachment| matches!(attachment, Attachment::Terminal(_)))
        );
        window.click("attach-connection-web", cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(thread.read(cx).composer().attachments().is_empty());
        window.click("attach-connection-web", cx);
        assert_eq!(
            thread.read(cx).composer().attachments(),
            &[Attachment::Connection("web".into())]
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_project_chat_keeps_the_local_choice_without_falling_back_to_a_general_shell(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.workspace.update(cx, |workspace, _| {
            workspace
                .set_local_terminal_opener(|_, _, _, _| panic!("must not open a General shell"));
        });
        super::super::seat_binding::project_seat(
            &f,
            nocterm_project_model::ProjectId::from_bytes([1; 16]),
            window,
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.click("agent-attach-context", cx);
        assert!(window.find("attach-local-terminal").visible());
        window.click("attach-local-terminal", cx);
        assert!(
            f.panel
                .read(cx)
                .current()
                .unwrap()
                .read(cx)
                .workspace_seat()
                .terminals(cx)
                .unwrap()
                .is_empty()
        );
    })
    .unwrap();
    cx.run_until_parked();
}
