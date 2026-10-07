//! Controlled factory delivery keeps native header intent across layout changes.
use super::*;
use crate::LocalTerminalTarget;
use gpui_kit::component::WindowExt as _;
use std::cell::RefCell;

fn queue_factory(
    cx: &mut TestAppContext,
    workspace: &Entity<Workspace>,
) -> Rc<RefCell<Vec<LocalTerminalTarget>>> {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let captured = requests.clone();
    workspace.update(cx, |ws, _| {
        ws.set_local_terminal_opener(move |_, target, _, _| captured.borrow_mut().push(target));
    });
    requests
}

fn click_plus(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("local-terminal-new", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn native_header_request_follows_moved_anchor_when_factory_delivers_later(cx: &mut TestAppContext) {
    let (window, ws, locals) = open_three(cx, true);
    let anchor = locals[2].entity_id();
    let central = cx
        .update_window(window, |_, window, cx| {
            let central = tab(cx, "/central");
            ws.update(cx, |ws, cx| {
                ws.add_item(central.clone(), window, cx);
                ws.activate_item_by_id(anchor, window, cx);
            });
            central
        })
        .unwrap();
    cx.run_until_parked();
    let requests = queue_factory(cx, &ws);
    click_plus(cx, window);
    assert_eq!(*requests.borrow(), [LocalTerminalTarget::Beside(anchor)]);
    ws.read_with(cx, |ws, _| assert_eq!(ws.items.len(), 4));

    with(cx, window, &ws, |ws, window, cx| {
        let node = ws.item_location(central.entity_id(), cx).unwrap().1;
        let panel = ws.panel_of(anchor).unwrap();
        ws.dock.update(cx, |dock, cx| {
            dock.move_panel(
                panel,
                InsertTarget::Tabs {
                    node,
                    ix: None,
                    activate: true,
                },
                window,
                cx,
            );
            dock.set_zoomed_in(node, window, cx);
        });
    });
    let live = ws.read_with(cx, |ws, cx| ws.item_location(anchor, cx).unwrap());
    assert_eq!(live.0, DockPlacement::Center);
    let request = requests.borrow_mut().pop().unwrap();
    let added = cx
        .update_window(window, |_, window, cx| {
            let added = tab(cx, "/delayed");
            ws.update(cx, |ws, cx| {
                assert!(ws.add_local_terminal_at(added.clone(), request, window, cx));
            });
            added
        })
        .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.items.len(), 5);
        assert_eq!(ws.item_location(added.entity_id(), cx), Some(live));
        assert_eq!(ws.dock.read(cx).zoomed_group(), Some(live.1));
        assert_eq!(ws.local_terminal_cwd(cx), Some("/delayed".into()));
        assert!(bar(ws, anchor, cx).contains(&added.entity_id()));
    });
    for tab in locals.into_iter().chain([central, added]) {
        assert_eq!(tab.read_with(cx, |tab, _| tab.closed.get()), 0);
    }
}

#[gpui_kit::test]
fn native_header_request_rejects_closed_anchor_without_ghost_or_existing_item_close(
    cx: &mut TestAppContext,
) {
    let (window, ws, locals) = open_three(cx, true);
    let anchor = locals[2].entity_id();
    let requests = queue_factory(cx, &ws);
    click_plus(cx, window);
    assert_eq!(*requests.borrow(), [LocalTerminalTarget::Beside(anchor)]);
    with(cx, window, &ws, |ws, window, cx| {
        ws.close_item_by_id(anchor, window, cx)
    });

    let target = requests.borrow_mut().pop().unwrap();
    let rejected = cx
        .update_window(window, |_, window, cx| {
            let rejected = tab(cx, "/rejected");
            ws.update(cx, |ws, cx| {
                assert!(!ws.add_local_terminal_at(rejected.clone(), target, window, cx));
                assert!(!ws.add_local_terminal_at(locals[0].clone(), target, window, cx));
            });
            rejected
        })
        .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.items.len(), 2);
        assert!(ws.item_location(rejected.entity_id(), cx).is_none());
        assert!(ws.local_entry(rejected.entity_id()).is_none());
        assert!(ws.item_location(anchor, cx).is_none());
        assert_eq!(bar(ws, locals[0].entity_id(), cx).len(), 2);
    });
    assert_eq!(rejected.read_with(cx, |tab, _| tab.closed.get()), 1);
    assert_eq!(locals[2].read_with(cx, |tab, _| tab.closed.get()), 1);
    for tab in &locals[..2] {
        assert_eq!(tab.read_with(cx, |tab, _| tab.closed.get()), 0);
    }
}

#[gpui_kit::test]
fn native_inactive_alias_updates_authoritative_central_context_after_local_focus(
    cx: &mut TestAppContext,
) {
    let (window, ws) = super::super::tests::fixture(cx);
    let (first, second, local) = cx
        .update_window(window, |_, window, cx| {
            let first = tab(cx, "/first");
            let second = tab(cx, "/second");
            let local = tab(cx, "/local");
            first.update(cx, |tab, _| {
                tab.target = Some(Target::parse("user@first", None).unwrap())
            });
            second.update(cx, |tab, _| {
                tab.target = Some(Target::parse("user@second", None).unwrap())
            });
            ws.update(cx, |ws, cx| {
                ws.add_item(first.clone(), window, cx);
                ws.add_item(second.clone(), window, cx);
                ws.add_local_terminal(local.clone(), window, cx);
            });
            (first, second, local)
        })
        .unwrap();
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.active_session(cx).unwrap().target.host, "second")
    });
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.double_click(("tab-title", first.entity_id().as_u64()), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.focused_input(cx).is_some());
        assert_eq!(
            ws.read(cx).active_item().unwrap().item_id(),
            first.entity_id()
        );
        assert_eq!(ws.read(cx).active_session(cx).unwrap().target.host, "first");
        window.press("ctrl-a", cx);
        window.input("Renamed", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            ws.read(cx).command_title(window, cx).as_deref(),
            Some("Renamed")
        );
        assert!(first.read(cx).focus_handle(cx).is_focused(window));
        ws.update(cx, |ws, cx| {
            ws.activate_item_by_id(local.entity_id(), window, cx);
            assert_eq!(ws.active_session(cx).unwrap().target.host, "first");
            assert_eq!(ws.local_terminal_cwd(cx), Some("/local".into()));
        });
    })
    .unwrap();
    cx.run_until_parked();
    for tab in [first, second, local] {
        assert_eq!(tab.read_with(cx, |tab, _| tab.closed.get()), 0);
    }
}
