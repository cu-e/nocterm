//! Native inner gestures cannot mutate either outer or sibling topology.
use super::*;

#[gpui_kit::test]
fn inner_native_reorder_split_and_close_leave_the_outer_connection_bar_intact(
    cx: &mut TestAppContext,
) {
    let NestedTabs {
        window,
        workspace,
        server,
        outer,
        surface,
        terminals,
    } = three_inner(cx);
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find(("tab-title", outer.entity_id().as_u64()))
                .bounds()
                .bottom()
                <= window
                    .find(("tab-title", terminals[0].entity_id().as_u64()))
                    .bounds()
                    .top()
        );
        drag_tab(
            window,
            terminals[0].entity_id(),
            terminals[2].entity_id(),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        assert_eq!(
            surface.panes(DockPlacement::Center, cx)[0].items(),
            [
                terminals[1].entity_id(),
                terminals[2].entity_id(),
                terminals[0].entity_id()
            ]
        );
        window.render_frame(cx);
        window.right_click(("tab-title", terminals[1].entity_id().as_u64()), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(6usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        assert_eq!(surface.panes(DockPlacement::Center, cx).len(), 2);
        assert_eq!(
            workspace
                .read(cx)
                .items(cx)
                .map(|item| item.item_id())
                .collect::<Vec<_>>(),
            [server.entity_id(), outer.entity_id()]
        );
        assert_eq!(
            workspace
                .read(cx)
                .tab_panes(DockPlacement::Center, cx)
                .len(),
            1
        );
        window.render_frame(cx);
        window.click(("close-tab", terminals[1].entity_id().as_u64()), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, _, cx| {
        assert_eq!(surface.tabs(cx).len(), 2);
        assert_eq!(workspace.read(cx).items(cx).count(), 2);
        assert_eq!(terminals[1].read(cx).closes.get(), 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn native_drag_between_inner_and_outer_bars_does_not_transfer_items(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let (server, outer, surface, terminal) = cx
        .update_window(window, |_, window, cx| {
            let server = probe(cx, Rc::default());
            workspace.update(cx, |workspace, cx| {
                workspace.add_item(server.clone(), window, cx)
            });
            let (outer, surface) = add_project(&workspace, 1, window, cx);
            let terminal = probe(cx, Rc::default());
            surface
                .add_item(surface.selection_receipt(cx), terminal.clone(), window, cx)
                .unwrap();
            (server, outer, surface, terminal)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        drag_tab(window, terminal.entity_id(), outer.entity_id(), cx);
        drag_tab(window, server.entity_id(), terminal.entity_id(), cx);
        drag_tab(window, outer.entity_id(), terminal.entity_id(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, _, cx| {
        assert_eq!(
            workspace
                .read(cx)
                .items(cx)
                .map(|item| item.item_id())
                .collect::<Vec<_>>(),
            [server.entity_id(), outer.entity_id()]
        );
        assert_eq!(
            surface
                .tabs(cx)
                .iter()
                .map(|item| item.id())
                .collect::<Vec<_>>(),
            [terminal.entity_id()]
        );
        assert_eq!(terminal.read(cx).closes.get(), 0);
        assert_eq!(server.read(cx).closes.get(), 0);
    })
    .unwrap();
}

#[gpui_kit::test]
fn inner_terminal_content_fills_the_pane_without_a_card_inset(cx: &mut TestAppContext) {
    let nested = three_inner(cx);
    cx.run_until_parked();
    cx.update_window(nested.window, |_, window, cx| {
        window.render_frame(cx);
        let pane = window.find("project-terminal-pane").bounds();
        let content = window
            .within("project-terminal-pane")
            .find("tab-group-content")
            .bounds();
        assert_eq!(content.left(), pane.left());
        assert_eq!(content.bottom(), pane.bottom());
        assert_eq!(content.size.width, pane.size.width);
    })
    .unwrap();
}

struct NestedTabs {
    window: AnyWindowHandle,
    workspace: Entity<Workspace>,
    server: Entity<Probe>,
    outer: Entity<project_fixtures::ProjectTab>,
    surface: crate::ProjectSurface,
    terminals: [Entity<Probe>; 3],
}
fn three_inner(cx: &mut TestAppContext) -> NestedTabs {
    let (window, workspace) = fixture(cx);
    let (server, outer, surface, terminals) = cx
        .update_window(window, |_, window, cx| {
            let server = probe(cx, Rc::default());
            workspace.update(cx, |workspace, cx| {
                workspace.add_item(server.clone(), window, cx)
            });
            let (outer, surface) = add_project(&workspace, 1, window, cx);
            let terminals = [
                probe(cx, Rc::default()),
                probe(cx, Rc::default()),
                probe(cx, Rc::default()),
            ];
            for terminal in &terminals {
                surface
                    .add_item(surface.selection_receipt(cx), terminal.clone(), window, cx)
                    .unwrap();
            }
            (server, outer, surface, terminals)
        })
        .unwrap();
    NestedTabs {
        window,
        workspace,
        server,
        outer,
        surface,
        terminals,
    }
}
