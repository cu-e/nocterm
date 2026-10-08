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
    let (send, _receive) = oneshot::channel();
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        let session = thread.read(cx).session().clone().unwrap();
        let request = serde_json::from_value(serde_json::json!({"sessionId":session,"toolCall":{"toolCallId":"call","title":"Permission"},"options":[{"optionId":"allow","name":"Allow","kind":"allow_once"}]})).unwrap();
        thread.update(cx, |thread, cx| thread.permission(request, send, cx));
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
        let (send, _receive) = oneshot::channel();
        cx.update(|cx| {
            let session = owner.read(cx).session().clone().unwrap();
            let request = serde_json::from_value(serde_json::json!({"sessionId":session,"toolCall":{"toolCallId":id,"title":"Permission"},"options":[{"optionId":"allow","name":"Allow","kind":"allow_once"}]})).unwrap();
            owner.update(cx, |t, cx| t.permission(request, send, cx));
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
                t.status = "unchanged approval, more output".into();
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update_window(f.handle, |_, window, cx| {
            assert_eq!(nocterm_ui::notice::count(window, cx), 0)
        })
        .unwrap();
    }
    cx.update(|cx| assert_eq!(owner.read(cx).permissions.len(), 2));
}
