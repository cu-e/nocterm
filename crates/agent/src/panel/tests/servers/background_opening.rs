//! Background session creation retains original ownership through async handoff.
use super::*;

fn queued_background_open(
    f: &Fixture,
    cx: &mut TestAppContext,
) -> (
    Rc<OpeningDirectory>,
    gpui_kit::WeakEntity<crate::thread::AgentThread>,
    crate::thread::ToolApprovalCard,
) {
    let directory = install(f, cx);
    new_chat(f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let server = cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.attach(Attachment::Group("prod".into()), cx);
            thread.offline_servers(cx)[0].0.server_id.clone()
        })
    });
    let (respond, _response) = oneshot::channel();
    cx.update(|cx| {
        thread.update(cx, |thread, _| {
            f.calls
                .try_send(BridgeCall {
                    arguments: None,
                    display_token: None,
                    registration_id: thread.registration().as_ref().unwrap().id,
                    call: nocterm_ai::TerminalCall::OpenTerminal(nocterm_ai::OpenTerminal {
                        server_id: server,
                    }),
                    respond,
                })
                .unwrap();
        })
    });
    cx.run_until_parked();
    let card = cx.update(|cx| thread.read(cx).tool_cards().remove(0));
    (directory, thread.downgrade(), card)
}

#[gpui_kit::test]
fn deleting_chat_before_background_open_first_poll_never_creates_a_session(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let (directory, owner, card) = queued_background_open(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.set_history(true, cx));
        window.render_frame(cx);
        owner.upgrade().unwrap().update(cx, |thread, cx| {
            thread.approve_tool(card.token(), true, false, cx)
        });
        window.click(("delete-thread", owner.entity_id().as_u64()), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(owner.upgrade().is_none());
    assert!(
        directory.background.borrow().is_empty(),
        "destroyed chat cannot start native opening"
    );
    cx.update(|cx| assert_eq!(f.workspace.read(cx).terminals(cx).len(), 1));
}

#[gpui_kit::test]
fn detaching_server_before_background_open_first_poll_never_creates_a_session(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let (directory, owner, card) = queued_background_open(&f, cx);
    cx.update(|cx| {
        owner.upgrade().unwrap().update(cx, |thread, cx| {
            thread.approve_tool(card.token(), true, false, cx);
            thread.attach(Attachment::Group("prod".into()), cx);
        });
    });
    cx.run_until_parked();
    assert!(
        directory.background.borrow().is_empty(),
        "detached server cannot start native opening"
    );
    cx.update(|cx| assert_eq!(f.workspace.read(cx).terminals(cx).len(), 1));
}

#[gpui_kit::test]
fn deleting_chat_during_background_creation_closes_its_original_session(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let (directory, owner, card) = queued_background_open(&f, cx);
    let id = owner.entity_id().as_u64();
    *directory.after_background_open.borrow_mut() = Some(Box::new(move |window, cx| {
        window.click(("delete-thread", id), cx);
    }));
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| panel.set_history(true, cx));
        window.render_frame(cx);
        owner.upgrade().unwrap().update(cx, |thread, cx| {
            thread.approve_tool(card.token(), true, false, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert!(owner.upgrade().is_none());
    let opened = directory.background.borrow();
    assert_eq!(
        opened.len(),
        1,
        "opening started before its owner was destroyed"
    );
    cx.update(|cx| {
        let workspace = f.workspace.read(cx);
        assert!(!workspace.is_background(opened[0]));
        assert!(
            workspace
                .terminals(cx)
                .iter()
                .all(|entry| entry.item != opened[0])
        );
    });
}

#[gpui_kit::test]
fn failed_opening_never_closes_a_session_the_user_promoted_to_a_tab(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let (directory, owner, card) = queued_background_open(&f, cx);
    let workspace = f.workspace.downgrade();
    let detached = owner.clone();
    *directory.after_background_open.borrow_mut() = Some(Box::new(move |window, cx| {
        let item = workspace
            .upgrade()
            .unwrap()
            .read(cx)
            .terminals(cx)
            .into_iter()
            .find(|entry| entry.background)
            .unwrap()
            .item;
        workspace
            .update(cx, |workspace, cx| {
                assert!(workspace.show_background_session(item, window, cx));
            })
            .unwrap();
        detached.upgrade().unwrap().update(cx, |thread, cx| {
            thread.attach(Attachment::Group("prod".into()), cx);
        });
    }));
    cx.update(|cx| {
        owner.upgrade().unwrap().update(cx, |thread, cx| {
            thread.approve_tool(card.token(), true, false, cx)
        });
    });
    cx.run_until_parked();
    let item = directory.background.borrow()[0];
    cx.update(|cx| {
        let workspace = f.workspace.read(cx);
        assert!(!workspace.is_background(item));
        assert!(
            workspace
                .terminals(cx)
                .iter()
                .any(|entry| entry.item == item && !entry.background)
        );
    });
}
