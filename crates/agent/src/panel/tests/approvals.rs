use super::*;

#[gpui_kit::test]
fn independent_connection_approvals_recheck_detachment_and_stop_blocks_late_chunks(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    new_chat(&f, cx);
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 2);
    let (rx, session) = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        let (tx, rx) = oneshot::channel();
        let registration = thread.read(cx).registration().as_ref().unwrap().id;
        let id = thread.update(cx, |thread, cx| thread.resolved(cx)[0].0.clone());
        thread.update(cx, |thread, cx| {
            thread.handle_tool(
                BridgeCall {
                    registration_id: registration,
                    call: nocterm_ai::TerminalCall::SendInput(nocterm_ai::SendInput {
                        terminal_id: id,
                        text: "danger".into(),
                        press_enter: true,
                    }),
                    respond: tx,
                },
                cx,
            )
        });
        assert_eq!(thread.read(cx).tools.len(), 1);
        thread.update(cx, |thread, _| thread.composer.attachments.clear());
        thread.update(cx, |thread, cx| thread.approve_tool(0, true, true, cx));
        (rx, thread.read(cx).session().clone().unwrap())
    });
    assert!(f.access.sent.borrow().is_empty());
    assert!(futures::executor::block_on(rx).unwrap().is_err());
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| {
            thread.send("hello".into(), cx);
            thread.stop(cx);
            thread.send("blocked".into(), cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(f.commands.prompts.lock().unwrap().len(), 1);
    f.events
        .try_send(AgentEvent::Session(acp::SessionNotification::new(
            session,
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
                acp::TextContent::new("late"),
            ))),
        )))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        assert!(
            !thread.read(cx).state.entries.iter().any(
                |entry| matches!(entry,nocterm_ai::thread::Entry::Agent(value)if value=="late")
            )
        );
    });
}

#[gpui_kit::test]
fn live_group_membership_and_revoked_registration_reject_stale_terminal_ids(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    *f.access.profile.borrow_mut() = Some("p1".into());
    let summary: nocterm_workspace::ConnectionSummary = nocterm_workspace::ConnectionSummary {
        id: "p1".into(),
        name: "Server".into(),
        group: Some("prod".into()),
        description: "Production".into(),
        target: serde_json::from_value(
            serde_json::json!({"host":"example.test", "port":22, "user":"user"}),
        )
        .unwrap(),
        icon: None,
        flag: None,
    };
    let directory = Rc::new(Directory(std::cell::RefCell::new(vec![summary])));
    cx.update(|cx| {
        f.workspace.update(cx, |workspace, _| {
            workspace.set_connection_directory(directory.clone())
        })
    });
    new_chat(&f, cx);
    let (registration, id) = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| {
            thread.composer.attachments = vec![Attachment::Group("prod".into())];
            let terminals = thread.resolved(cx);
            assert_eq!(terminals.len(), 1);
            (
                thread.registration().as_ref().unwrap().id,
                terminals[0].0.clone(),
            )
        })
    });
    directory.0.borrow_mut()[0].group = Some("other".into());
    for registration_id in [registration, registration + 100] {
        let (tx, rx) = oneshot::channel();
        cx.update(|cx| {
            let thread = f.panel.read(cx).current().unwrap();
            thread.update(cx, |thread, cx| {
                thread.handle_tool(
                    BridgeCall {
                        registration_id,
                        call: nocterm_ai::TerminalCall::ReadTerminal(nocterm_ai::ReadTerminal {
                            terminal_id: id.clone(),
                            lines: None,
                            since: None,
                        }),
                        respond: tx,
                    },
                    cx,
                )
            });
        });
        assert!(futures::executor::block_on(rx).unwrap().is_err());
    }
    directory.0.borrow_mut()[0].group = Some("prod".into());
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| assert_eq!(thread.resolved(cx)[0].0, id));
    });
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn queued_long_terminal_approvals_fit_short_narrow_panel_and_actions_respond(
    cx: &mut TestAppContext,
) {
    let f = fixture_with_width(cx, 18.);
    new_chat(&f, cx);
    let responses = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        let registration_id = thread.read(cx).registration().as_ref().unwrap().id;
        let terminal_id = thread.update(cx, |thread, cx| thread.resolved(cx)[0].0.clone());
        (0..5)
            .map(|_| {
                let (respond, receive) = oneshot::channel();
                thread.update(cx, |thread, cx| {
                    thread.handle_tool(
                        BridgeCall {
                            registration_id,
                            call: nocterm_ai::TerminalCall::RunCommand(nocterm_ai::RunCommand {
                                terminal_id: terminal_id.clone(),
                                command: "long_unbroken_command_".repeat(200),
                                timeout_ms: None,
                                idle_ms: None,
                            }),
                            respond,
                        },
                        cx,
                    )
                });
                receive
            })
            .collect::<Vec<_>>()
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(400.)));
        window.render_frame(cx);
        let panel = window.find("agent-panel").bounds();
        let approvals = window.find("agent-approvals").bounds();
        let composer = window.find("agent-composer").bounds();
        assert!(approvals.bottom() <= composer.origin.y);
        assert!(composer.bottom() <= window.viewport_size().height);
        assert!(approvals.right() <= panel.right());
        assert!(window.find(("approval-command", 0usize)).bounds().right() <= panel.right());
        let deny = window.find(("tool-Deny", 0usize)).bounds();
        assert!(
            deny.bottom() <= window.find("agent-approval-list").bounds().bottom(),
            "first approval action must be visible"
        );
        window.click(("tool-Deny", 0usize), cx);
        assert_eq!(f.panel.read(cx).current().unwrap().read(cx).tools.len(), 4);
        window.click(("tool-Allow once", 0usize), cx);
        assert_eq!(f.panel.read(cx).current().unwrap().read(cx).tools.len(), 3);
    })
    .unwrap();
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(100));
    cx.run_until_parked();
    let mut responses = responses.into_iter();
    assert!(
        futures::executor::block_on(responses.next().unwrap())
            .unwrap()
            .unwrap_err()
            .contains("denied")
    );
    assert!(
        futures::executor::block_on(responses.next().unwrap())
            .unwrap()
            .is_ok()
    );
    assert_eq!(f.access.sent.borrow().len(), 1);
    cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .update(cx, |thread, cx| thread.stop(cx))
    });
    for response in responses {
        assert!(futures::executor::block_on(response).unwrap().is_err());
    }
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("agent-approvals").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn permission_approval_choices_are_clickable_and_cancel_removes_last_request(
    cx: &mut TestAppContext,
) {
    let f = fixture_with_width(cx, 18.);
    new_chat(&f, cx);
    let responses = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        let session = thread.read(cx).session().clone().unwrap();
        (0..2).map(|_| {
            let (send, receive) = nocterm_ai::PermissionResponder::channel();
            let request = serde_json::from_value(serde_json::json!({"sessionId":session,"toolCall":{"toolCallId":"call","title":"Long permission title"},"options":[{"optionId":"allow","name":"Allow once","kind":"allow_once"},{"optionId":"deny","name":"Reject","kind":"reject_once"}]})).unwrap();
            thread.update(cx, |thread, cx| thread.permission(request, send, cx));
            receive
        }).collect::<Vec<_>>()
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(400.)));
        window.render_frame(cx);
        window.click("permission-0-allow", cx);
        assert_eq!(
            f.panel
                .read(cx)
                .current()
                .unwrap()
                .read(cx)
                .permissions
                .len(),
            1
        );
        window.click(("permission-cancel", 0usize), cx);
        assert!(window.try_find("agent-approvals").is_none());
    })
    .unwrap();
    let mut responses = responses.into_iter();
    assert!(matches!(
        futures::executor::block_on(responses.next().unwrap()).unwrap(),
        acp::RequestPermissionOutcome::Selected(_)
    ));
    assert!(matches!(
        futures::executor::block_on(responses.next().unwrap()).unwrap(),
        acp::RequestPermissionOutcome::Cancelled
    ));
}

#[gpui_kit::test]
fn review_notice_selects_pending_thread_and_leaves_history(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let owner = cx.update(|cx| f.panel.read(cx).current().unwrap());
    cx.update(|cx| owner.update(cx, |thread, cx| thread.send("check the disk".into(), cx)));
    new_chat(&f, cx);
    let (send, receive) = nocterm_ai::PermissionResponder::channel();
    cx.update(|cx| {
        let session = owner.read(cx).session().clone().unwrap();
        let request = serde_json::from_value(serde_json::json!({"sessionId":session,"toolCall":{"toolCallId":"call","title":"Review pending request"},"options":[{"optionId":"allow","name":"Allow","kind":"allow_once"}]})).unwrap();
        f.panel.update(cx, |panel, cx| { panel.history = true; cx.notify(); });
        owner.update(cx, |thread, cx| thread.permission(request, send, cx));
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("notice-action", cx);
        assert_eq!(
            f.panel.read(cx).current().unwrap().entity_id(),
            owner.entity_id()
        );
        window.click("permission-0-allow", cx);
    })
    .unwrap();
    assert!(matches!(
        futures::executor::block_on(receive).unwrap(),
        acp::RequestPermissionOutcome::Selected(_)
    ));
}

#[gpui_kit::test]
fn obsolete_approval_notice_is_removed_on_stop_resolve_and_delete_without_clearing_other_notices(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    for disposition in ["stop", "resolve", "delete"] {
        new_chat(&f, cx);
        let (send, receive) = nocterm_ai::PermissionResponder::channel();
        let owner = cx.update(|cx| {
            let owner = f.panel.read(cx).current().unwrap();
            let session = owner.read(cx).session().clone().unwrap();
            let request = serde_json::from_value(serde_json::json!({"sessionId":session,"toolCall":{"toolCallId":"call","title":"Hidden permission"},"options":[{"optionId":"allow","name":"Allow","kind":"allow_once"}]})).unwrap();
            owner.update(cx, |thread, cx| thread.permission(request, send, cx));
            owner
        });
        // Another chat in front hides the request.
        new_chat(&f, cx);
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        cx.update_window(f.handle, |_, window, cx| {
            nocterm_ui::notice::info(
                window,
                cx,
                "unrelated",
                "Other operation",
                "Keep this notice",
            );
            assert_eq!(nocterm_ui::notice::count(window, cx), 2);
            match disposition {
                "stop" => owner.update(cx, |thread, cx| thread.stop(cx)),
                "resolve" => owner.update(cx, |thread, cx| {
                    thread.choose_permission(0, Some("allow".into()), cx)
                }),
                "delete" => {
                    f.panel.update(cx, |panel, cx| panel.set_history(true, cx));
                    window.render_frame(cx);
                    window.click(("delete-thread", owner.entity_id().as_u64()), cx);
                }
                _ => unreachable!(),
            }
        })
        .unwrap();
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        cx.update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(nocterm_ui::notice::count(window, cx), 1, "obsolete {disposition} approval notice must disappear while unrelated notice remains");
            assert!(window.try_find("notice-action").is_none());
        }).unwrap();
        if disposition == "delete" {
            cx.update(|_| drop(owner));
            cx.run_until_parked();
        }
        let result = receive
            .now_or_never()
            .unwrap_or_else(|| panic!("{disposition} response was not released"))
            .unwrap();
        assert_eq!(
            matches!(result, acp::RequestPermissionOutcome::Selected(_)),
            disposition == "resolve"
        );
    }
}
