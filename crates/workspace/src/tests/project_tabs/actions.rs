//! Keyboard tab commands belong to the focused level of the nested dock.
use super::*;

#[gpui_kit::test]
fn an_inner_menu_captured_before_outer_aba_cannot_close_after_return(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let (outer_a, a, outer_b, terminal) = cx
        .update_window(window, |_, window, cx| {
            let (outer_a, a) = add_project(&workspace, 1, window, cx);
            let terminal = probe(cx, Rc::default());
            a.add_item(a.selection_receipt(cx), terminal.clone(), window, cx)
                .unwrap();
            let (outer_b, _) = add_project(&workspace, 2, window, cx);
            workspace.update(cx, |workspace, cx| {
                workspace.activate_item_by_id(outer_a.entity_id(), window, cx);
            });
            (outer_a, a, outer_b, terminal)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(("tab-title", terminal.entity_id().as_u64()), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(outer_b.entity_id(), window, cx);
            workspace.activate_item_by_id(outer_a.entity_id(), window, cx);
        });
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, _, cx| {
        assert_eq!(a.tabs(cx).len(), 1);
        assert_eq!(terminal.read(cx).closes.get(), 0);
        assert_eq!(
            workspace.read(cx).selected_context(cx),
            ContextId::Project(a.project())
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn inner_zoom_and_local_dock_actions_do_not_change_the_window_dock(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let surface = cx
        .update_window(window, |_, window, cx| {
            let (_, surface) = add_project(&workspace, 1, window, cx);
            for _ in 0..2 {
                let terminal = probe(cx, Rc::default());
                surface
                    .add_item(surface.selection_receipt(cx), terminal, window, cx)
                    .unwrap();
            }
            let local = probe(cx, Rc::default());
            surface
                .add_local_terminal(
                    surface.selection_receipt(cx),
                    local,
                    crate::LocalTerminalTarget::Bottom,
                    window,
                    cx,
                )
                .unwrap();
            surface
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        let terminal = surface
            .tabs(cx)
            .into_iter()
            .find(|tab| !tab.is_local())
            .unwrap()
            .id();
        surface
            .activate_item(surface.selection_receipt(cx), terminal, window, cx)
            .unwrap();
        window.render_frame(cx);
        assert!(surface.local_terminal_is_visible(cx));
        assert!(!workspace.read(cx).local_terminal_is_visible(cx));
        window.dispatch_action(Box::new(gpui_kit::component::dock::ToggleZoom), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(surface.zoomed_pane(cx).is_some());
        assert!(workspace.read(cx).zoomed_pane(cx).is_none());
        window.dispatch_action(Box::new(gpui_kit::component::dock::ToggleZoom), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(surface.zoomed_pane(cx).is_none());
        window.dispatch_action(Box::new(crate::ToggleLocalTerminal), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(!surface.local_terminal_is_visible(cx));
        assert!(!workspace.read(cx).local_terminal_is_visible(cx));
    })
    .unwrap();
}

#[gpui_kit::test]
fn deferred_find_cannot_retarget_another_inner_terminal_in_the_same_workspace(
    cx: &mut TestAppContext,
) {
    let (window, workspace) = fixture(cx);
    let (surface, first, second) = cx
        .update_window(window, |_, window, cx| {
            let (_, surface) = add_project(&workspace, 1, window, cx);
            let first = probe(cx, Rc::default());
            let second = probe(cx, Rc::default());
            for item in [&first, &second] {
                surface
                    .add_item(surface.selection_receipt(cx), item.clone(), window, cx)
                    .unwrap();
            }
            (surface, first, second)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        surface
            .activate_item(surface.selection_receipt(cx), first.entity_id(), window, cx)
            .unwrap();
        window.render_frame(cx);
        workspace.update(cx, |workspace, cx| {
            assert!(workspace.command_available(&crate::Find, window, cx));
            workspace.dispatch_command(Box::new(crate::Find), window, cx);
        });
        surface
            .activate_item(
                surface.selection_receipt(cx),
                second.entity_id(),
                window,
                cx,
            )
            .unwrap();
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, _, cx| {
        assert!(first.read(cx).executed.borrow().is_empty());
        assert!(second.read(cx).executed.borrow().is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn closing_the_last_inner_tab_keeps_its_workspace_and_connection_tabs(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let (outer, surface) = cx
        .update_window(window, |_, window, cx| {
            let server = probe(cx, Rc::default());
            workspace.update(cx, |workspace, cx| workspace.add_item(server, window, cx));
            let (outer, surface) = add_project(&workspace, 1, window, cx);
            let terminal = probe(cx, Rc::default());
            surface
                .add_item(surface.selection_receipt(cx), terminal, window, cx)
                .unwrap();
            window.render_frame(cx);
            (outer, surface)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(crate::CloseTab), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, _, cx| {
        assert_eq!(workspace.read(cx).items(cx).count(), 2);
        assert_eq!(
            workspace.read(cx).active_item(cx).unwrap().item_id(),
            outer.entity_id()
        );
        assert!(surface.tabs(cx).is_empty());
        assert_eq!(
            workspace.read(cx).selected_context(cx),
            ContextId::Project(surface.project())
        );
    })
    .unwrap();
}
