use super::*;
use nocterm_ai::ApprovalPolicy;

fn options() -> Vec<acp::PermissionOption> {
    vec![
        acp::PermissionOption::new(
            "reject",
            "Allow once",
            acp::PermissionOptionKind::RejectOnce,
        ),
        acp::PermissionOption::new("always", "Allow", acp::PermissionOptionKind::AllowAlways),
        acp::PermissionOption::new("once", "Continue", acp::PermissionOptionKind::AllowOnce),
    ]
}

fn request(
    session: acp::SessionId,
    options: Vec<acp::PermissionOption>,
) -> acp::RequestPermissionRequest {
    serde_json::from_value(serde_json::json!({
        "sessionId":session, "toolCall":{"toolCallId":"provider-tool", "title":"Use agent files"}, "options":options
    })).unwrap()
}

fn submit(
    f: &Fixture,
    cx: &mut TestAppContext,
    session: acp::SessionId,
    options: Vec<acp::PermissionOption>,
) -> oneshot::Receiver<acp::RequestPermissionOutcome> {
    let (respond, receive) = nocterm_ai::PermissionResponder::channel();
    f.events
        .try_send(AgentEvent::Permission {
            request: request(session, options),
            respond,
        })
        .unwrap();
    cx.run_until_parked();
    receive
}

fn set_policy(cx: &mut TestAppContext, policy: ApprovalPolicy) {
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(move |settings| {
            settings.approval.agent_permissions = policy;
        })
        .detach()
    });
    cx.run_until_parked();
}

fn selected_once(receive: &mut oneshot::Receiver<acp::RequestPermissionOutcome>) {
    assert!(
        matches!(receive.try_recv().unwrap(), Some(acp::RequestPermissionOutcome::Selected(selected)) if selected.option_id == acp::PermissionOptionId::from("once"))
    );
}

#[gpui_kit::test]
fn provider_automatic_approval_does_not_change_terminal_grants(cx: &mut TestAppContext) {
    let f = fixture(cx);
    set_policy(cx, ApprovalPolicy::Allow);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let session = cx.update(|cx| thread.read(cx).session().clone().unwrap());
    let mut receive = submit(&f, cx, session, options());
    selected_once(&mut receive);
    let mut tool_response = cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            assert!(thread.permissions.is_empty());
            assert_eq!(thread.approval_generation, 0);
            let (respond, receive) = oneshot::channel();
            let terminal_id = thread.resolved(cx)[0].0.clone();
            thread.handle_tool(
                BridgeCall {
                    arguments: None,
                    display_token: None,
                    registration_id: thread.registration().as_ref().unwrap().id,
                    call: nocterm_ai::TerminalCall::SendInput(nocterm_ai::SendInput {
                        terminal_id,
                        text: "echo reviewed".into(),
                        press_enter: true,
                    }),
                    respond,
                },
                cx,
            );
            assert_eq!(
                thread.tools.len(),
                1,
                "terminal_write Ask remains independent"
            );
            receive
        })
    });
    assert!(tool_response.try_recv().unwrap().is_none());
    assert!(f.access.sent.borrow().is_empty());
}

#[gpui_kit::test]
fn changes_handle_pending_requests_in_every_chat_and_ask_again_for_future_requests(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let first = cx.update(|cx| f.panel.read(cx).current().unwrap());
    new_chat(&f, cx);
    let second = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let sessions = cx.update(|cx| {
        [
            first.read(cx).session().clone().unwrap(),
            second.read(cx).session().clone().unwrap(),
        ]
    });
    let mut first_receive = submit(&f, cx, sessions[0].clone(), options());
    let mut second_receive = submit(&f, cx, sessions[1].clone(), options());
    assert!(first_receive.try_recv().unwrap().is_none());
    assert!(second_receive.try_recv().unwrap().is_none());
    set_policy(cx, ApprovalPolicy::Allow);
    selected_once(&mut first_receive);
    selected_once(&mut second_receive);
    cx.update(|cx| {
        for thread in [&first, &second] {
            assert!(thread.read(cx).permissions.is_empty());
            assert_eq!(
                thread.read(cx).approval_generation,
                1,
                "draining does not post a new approval notice"
            );
        }
    });
    set_policy(cx, ApprovalPolicy::Ask);
    let mut future = submit(&f, cx, sessions[1].clone(), options());
    assert!(future.try_recv().unwrap().is_none());
    cx.update(|cx| assert_eq!(second.read(cx).permissions.len(), 1));
}

#[gpui_kit::test]
fn no_one_time_choice_stays_visible_with_an_explanation_and_can_be_cancelled(
    cx: &mut TestAppContext,
) {
    let f = fixture_with_width(cx, 18.);
    set_policy(cx, ApprovalPolicy::Allow);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let session = cx.update(|cx| thread.read(cx).session().clone().unwrap());
    let mut receive = submit(
        &f,
        cx,
        session,
        vec![acp::PermissionOption::new(
            "always",
            "Allow once",
            acp::PermissionOptionKind::AllowAlways,
        )],
    );
    assert!(receive.try_recv().unwrap().is_none());
    cx.update_window(f.handle, |_, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(500.)));
        window.render_frame(cx);
        assert!(window.find(("permission-explanation", 0usize)).visible());
        window.click(("permission-cancel", 0usize), cx);
    })
    .unwrap();
    assert_eq!(
        receive.try_recv().unwrap(),
        Some(acp::RequestPermissionOutcome::Cancelled)
    );
}

#[gpui_kit::test]
fn stale_card_choice_cannot_grant_a_different_request_after_queue_compaction(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let session = cx.update(|cx| thread.read(cx).session().clone().unwrap());
    let mut once = submit(
        &f,
        cx,
        session.clone(),
        vec![acp::PermissionOption::new(
            "shared",
            "Allow once",
            acp::PermissionOptionKind::AllowOnce,
        )],
    );
    let old_generation = cx.update(|cx| thread.read(cx).permissions[0].generation);
    let mut always = submit(
        &f,
        cx,
        session,
        vec![acp::PermissionOption::new(
            "shared",
            "Remember",
            acp::PermissionOptionKind::AllowAlways,
        )],
    );
    set_policy(cx, ApprovalPolicy::Allow);
    assert!(matches!(
        once.try_recv().unwrap(),
        Some(acp::RequestPermissionOutcome::Selected(_))
    ));
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.choose_permission_request(old_generation, Some("shared".into()), cx)
        })
    });
    assert!(always.try_recv().unwrap().is_none());
    cx.update(|cx| {
        assert_eq!(thread.read(cx).permissions.len(), 1);
        let current_generation = thread.read(cx).permissions[0].generation;
        thread.update(cx, |thread, cx| {
            thread.choose_permission_request(current_generation, None, cx)
        });
    });
    assert_eq!(
        always.try_recv().unwrap(),
        Some(acp::RequestPermissionOutcome::Cancelled)
    );
}

#[gpui_kit::test]
fn invalid_manual_ids_and_foreign_or_replaced_sessions_cancel_even_in_automatic_mode(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let session = cx.update(|cx| thread.read(cx).session().clone().unwrap());
    let mut invalid = submit(&f, cx, session.clone(), options());
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.choose_permission(0, Some("foreign-id".into()), cx)
        })
    });
    assert_eq!(
        invalid.try_recv().unwrap(),
        Some(acp::RequestPermissionOutcome::Cancelled)
    );
    let mut obsolete = submit(&f, cx, session.clone(), options());
    cx.update(|cx| {
        thread.update(cx, |thread, _| {
            thread.lease.as_mut().unwrap().session = Some("replacement-session".into())
        })
    });
    set_policy(cx, ApprovalPolicy::Allow);
    assert_eq!(
        obsolete.try_recv().unwrap(),
        Some(acp::RequestPermissionOutcome::Cancelled)
    );
    let mut foreign = submit(&f, cx, session.clone(), options());
    assert_eq!(
        foreign.try_recv().unwrap(),
        Some(acp::RequestPermissionOutcome::Cancelled)
    );
    let (respond, mut direct_foreign) = nocterm_ai::PermissionResponder::channel();
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.permission(request(session, options()), respond, cx)
        })
    });
    assert_eq!(
        direct_foreign.try_recv().unwrap(),
        Some(acp::RequestPermissionOutcome::Cancelled)
    );
}

#[gpui_kit::test]
fn stop_blocks_late_provider_requests_in_idle_and_active_chats(cx: &mut TestAppContext) {
    for generating in [false, true] {
        let f = fixture(cx);
        new_chat(&f, cx);
        let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
        let session = cx.update(|cx| {
            if generating {
                thread.update(cx, |thread, cx| thread.send("hello".into(), cx));
            }
            thread.read(cx).session().clone().unwrap()
        });
        cx.run_until_parked();
        let mut queued = submit(&f, cx, session.clone(), options());
        cx.update(|cx| thread.update(cx, |thread, cx| thread.stop(cx)));
        cx.update(|cx| {
            assert!(thread.read(cx).stopped);
            assert_eq!(thread.read(cx).accept_updates, !generating);
        });
        assert_eq!(
            queued.try_recv().unwrap(),
            Some(acp::RequestPermissionOutcome::Cancelled)
        );
        set_policy(cx, ApprovalPolicy::Allow);
        let mut late = submit(&f, cx, session, options());
        assert_eq!(
            late.try_recv().unwrap(),
            Some(acp::RequestPermissionOutcome::Cancelled)
        );
    }
}

#[gpui_kit::test]
async fn disabling_ai_before_enabling_automatic_permissions_cancels_queued_requests(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let session = cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .read(cx)
            .session()
            .clone()
            .unwrap()
    });
    let mut queued = submit(&f, cx, session, options());
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| {
            settings.enabled = false;
            settings.approval.agent_permissions = ApprovalPolicy::Allow;
        })
    })
    .await
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        queued.try_recv().unwrap(),
        Some(acp::RequestPermissionOutcome::Cancelled)
    );
    assert_eq!(f.commands.shutdowns.load(Ordering::SeqCst), 1);
}
