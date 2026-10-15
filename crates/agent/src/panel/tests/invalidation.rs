use super::*;
use nocterm_workspace::WorkspaceEvent;
use std::cell::Cell;

#[gpui_kit::test]
fn workspace_chrome_notifications_do_not_invalidate_chat_but_semantic_events_do(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    cx.run_until_parked();
    let notifications = Rc::new(Cell::new(0));
    let seen = notifications.clone();
    let _observer = cx.update(|cx| cx.observe(&f.panel, move |_, _| seen.set(seen.get() + 1)));
    cx.update(|cx| f.workspace.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    assert_eq!(notifications.get(), 0);
    for event in [
        WorkspaceEvent::ItemsChanged,
        WorkspaceEvent::ActiveItemChanged,
        WorkspaceEvent::ActiveSessionChanged,
        WorkspaceEvent::LocalDirectoryChanged,
        WorkspaceEvent::ConnectionsChanged,
    ] {
        let before = notifications.get();
        cx.update(|cx| f.workspace.update(cx, |_, cx| cx.emit(event)));
        cx.run_until_parked();
        assert!(notifications.get() > before, "missing dependency {event:?}");
    }
}

#[gpui_kit::test]
fn closing_and_reopening_pending_chat_updates_attention_without_stream_updates(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let (send, _receive) = nocterm_ai::PermissionResponder::channel();
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        let session = thread.read(cx).session().clone().unwrap();
        let request = serde_json::from_value(serde_json::json!({"sessionId":session,"toolCall":{"toolCallId":"call","title":"Permission"},"options":[{"optionId":"allow","name":"Allow","kind":"allow_once"}]})).unwrap();
        f.events.try_send(AgentEvent::Permission { request, respond: send }).unwrap();
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        assert_eq!(
            nocterm_ui::notice::count(window, cx),
            0,
            "visible current approval needs no notice"
        );
        f.workspace
            .update(cx, |w, cx| w.toggle_right_panel(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        assert_eq!(
            nocterm_ui::notice::count(window, cx),
            1,
            "closing posts the hidden approval"
        );
        nocterm_ui::notice::remove(window, cx, "agent-approval");
        f.panel
            .update(cx, |panel, cx| panel.sync_approval_attention(window, cx));
        assert_eq!(
            nocterm_ui::notice::count(window, cx),
            0,
            "streaming must not repost a dismissed notice"
        );
        f.workspace
            .update(cx, |w, cx| w.toggle_right_panel(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        assert_eq!(nocterm_ui::notice::count(window, cx), 0);
        f.workspace
            .update(cx, |w, cx| w.toggle_right_panel(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        assert_eq!(nocterm_ui::notice::count(window, cx), 1)
    })
    .unwrap();
}

#[gpui_kit::test]
fn new_hidden_approval_reposts_after_dismissal_but_unchanged_stream_does_not(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let owner = cx.update(|cx| f.panel.read(cx).current().unwrap());
    cx.update_window(f.handle, |_, window, cx| {
        f.workspace
            .update(cx, |w, cx| w.toggle_right_panel(window, cx))
    })
    .unwrap();
    for id in ["first", "second"] {
        let (send, _receive) = nocterm_ai::PermissionResponder::channel();
        cx.update(|cx| {
            let session = owner.read(cx).session().clone().unwrap();
            let request = serde_json::from_value(serde_json::json!({"sessionId":session,"toolCall":{"toolCallId":id,"title":"Permission"},"options":[{"optionId":"allow","name":"Allow","kind":"allow_once"}]})).unwrap();
            f.events.try_send(AgentEvent::Permission { request, respond: send }).unwrap();
        });
        cx.run_until_parked();
        cx.update_window(f.handle, |_, window, cx| {
            assert_eq!(
                nocterm_ui::notice::count(window, cx),
                1,
                "new request {id} must announce itself"
            );
            nocterm_ui::notice::remove(window, cx, "agent-approval");
        })
        .unwrap();
        cx.update(|cx| {
            owner.update(cx, |t, cx| {
                report_control(
                    t,
                    acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                        acp::ContentBlock::Text(acp::TextContent::new(
                            "unchanged approval, more output",
                        )),
                    )),
                    cx,
                );
            })
        });
        cx.run_until_parked();
        cx.update_window(f.handle, |_, window, cx| {
            assert_eq!(nocterm_ui::notice::count(window, cx), 0)
        })
        .unwrap();
    }
    cx.update(|cx| assert_eq!(owner.read(cx).permission_cards().len(), 2));
}

#[gpui_kit::test]
fn separate_installations_keep_original_runtime_and_policy_after_later_composition(
    cx: &mut TestAppContext,
) {
    let first = fixture(cx);
    let second = fixture(cx);
    cx.update(|cx| {
        second
            .installation
            .installed
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |ai| ai.enabled = false)
            .detach();
        first
            .installation
            .installed
            .runtime()
            .set_last_agent("original", cx);
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(first.workspace.read(cx).right_panel_is_available());
        assert!(!second.workspace.read(cx).right_panel_is_available());
        assert_eq!(
            first
                .installation
                .installed
                .runtime()
                .favorites(cx)
                .last_agent
                .as_deref(),
            Some("original")
        );
        assert!(
            second
                .installation
                .installed
                .runtime()
                .favorites(cx)
                .last_agent
                .is_none()
        );
    });
    new_chat(&first, cx);
    assert_eq!(first.connector.connects.load(Ordering::SeqCst), 1);
    assert_eq!(second.connector.connects.load(Ordering::SeqCst), 0);
}

#[gpui_kit::test]
fn refused_shutdown_registration_returns_a_usable_original_installation(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let refused = nocterm_store::ShutdownCoordinator::default();
    cx.update(|cx| {
        drop(refused.capture_all(cx));
        let (installed, registration) = crate::init(
            fixture_services(&f),
            image_deps(),
            f.installation.layout.clone(),
            &f.installation.ready,
            &refused,
            cx,
        );
        assert!(matches!(
            registration,
            Err(nocterm_store::StoreError::Closing)
        ));
        installed.runtime().set_last_agent("still-usable", cx);
        assert_eq!(
            installed.runtime().favorites(cx).last_agent.as_deref(),
            Some("still-usable")
        );
        assert!(
            f.installation
                .installed
                .runtime()
                .favorites(cx)
                .last_agent
                .is_none()
        );
    });
}
