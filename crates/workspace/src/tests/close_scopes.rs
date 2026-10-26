//! Close scopes follow the clicked native pane and visual order.
use super::*;

#[gpui_kit::test]
fn close_scopes_follow_reordered_pane_tabs_and_do_not_close_other_panes(cx: &mut TestAppContext) {
    use crate::TabCloseScope;
    let (window, workspace) = fixture(cx);
    let closes = Rc::new(Cell::new(0));
    cx.update_window(window, |_, window, cx| {
        let a = probe(cx, closes.clone());
        let b = probe(cx, closes.clone());
        let c = probe(cx, closes.clone());
        let isolated = probe(cx, closes.clone());
        workspace.update(cx, |workspace, cx| {
            for item in [&a, &b, &c, &isolated] {
                workspace.add_item(item.clone(), window, cx);
            }
            workspace.split_active(Placement::Right, window, cx);
            workspace.activate_item_by_id(c.entity_id(), window, cx);
            workspace.move_active_tab(-1, window, cx); // a, c, b
            workspace.activate_item_by_id(isolated.entity_id(), window, cx);
            assert_eq!(
                workspace.tabs_to_close(c.entity_id(), TabCloseScope::Left, cx),
                [a.entity_id()]
            );
            assert_eq!(
                workspace.tabs_to_close(c.entity_id(), TabCloseScope::Right, cx),
                [b.entity_id()]
            );
            assert_eq!(
                workspace.tabs_to_close(c.entity_id(), TabCloseScope::Others, cx),
                [a.entity_id(), b.entity_id()]
            );
            workspace.close_tabs(c.entity_id(), TabCloseScope::Others, window, cx);
            assert_eq!(
                workspace.active_item(cx).unwrap().item_id(),
                isolated.entity_id()
            );
            assert_eq!(workspace.tabs(cx).len(), 2);
            assert_eq!(groups(workspace, cx), 2);
            assert_eq!(closes.get(), 2);
            let local = probe(cx, Rc::new(Cell::new(0)));
            workspace.set_local_terminal(local, window, cx);
            workspace.close_tabs(c.entity_id(), TabCloseScope::All, window, cx);
            assert!(workspace.items(cx).next().is_none());
            assert!(workspace.selected_local_terminal(cx).is_some());
            assert!(workspace.dock_region_present(DockPlacement::Bottom, cx));
            assert_eq!(closes.get(), 4);
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        closes.get(),
        4,
        "native deferred removal must not close twice"
    );
}
