// Modified by Nocterm contributors; see NOCTERM.md. Licensed under Apache-2.0.
//! Native edits settle queued zoom intent and retain explicit constraints.
use super::*;
use crate::dock::InsertTarget;

#[gpui::test]
fn same_group_reorder_keeps_zoom_and_cross_group_move_reveals_both_groups(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let (a_node, b_node, panel) = cx.read(|cx| {
        (
            f.a.read(cx).node(),
            f.b.read(cx).node(),
            PanelId::from(f.a_panel.entity_id()),
        )
    });
    cx.update(|window, cx| {
        f.area.update(cx, |area, cx| {
            let extra = TestPanel::new("extra", cx);
            area.add_panel_view(
                std::sync::Arc::new(extra),
                DockPlacement::Center,
                None,
                window,
                cx,
            );
            area.set_zoomed_in(a_node, window, cx);
            area.move_panel(
                panel,
                InsertTarget::Tabs {
                    node: a_node,
                    ix: Some(1),
                    activate: true,
                },
                window,
                cx,
            );
            assert_eq!(area.zoomed_group(), Some(a_node));
            area.move_panel(
                panel,
                InsertTarget::Tabs {
                    node: b_node,
                    ix: None,
                    activate: true,
                },
                window,
                cx,
            );
            assert!(!area.is_zoomed());
        });
    });
    settle(cx);
    cx.read(|cx| assert!(!f.area.read(cx).is_zoomed()));
}

#[gpui::test]
fn accepted_split_cancels_queued_zoom_while_noop_and_locked_moves_keep_zoom(
    cx: &mut TestAppContext,
) {
    let (f, cx) = fixture(cx);
    let (node, panel) = cx.read(|cx| (f.a.read(cx).node(), PanelId::from(f.a_panel.entity_id())));
    cx.update(|window, cx| {
        f.a.update(cx, |group, cx| group.toggle_zoom(window, cx));
        f.area.update(cx, |area, cx| {
            area.move_panel(
                panel,
                InsertTarget::Split {
                    node,
                    placement: Placement::Right,
                    size: None,
                },
                window,
                cx,
            );
        });
    });
    settle(cx);
    cx.read(|cx| assert!(!f.area.read(cx).is_zoomed()));
    cx.update(|window, cx| {
        f.area.update(cx, |area, cx| {
            let node = area
                .layout(DockPlacement::Center)
                .unwrap()
                .find_panel_node(panel)
                .unwrap();
            area.set_zoomed_in(node, window, cx);
            area.move_panel(
                PanelId::from_u64(u64::MAX),
                InsertTarget::Tabs {
                    node,
                    ix: None,
                    activate: true,
                },
                window,
                cx,
            );
            assert_eq!(area.zoomed_group(), Some(node));
            area.set_locked(true, window, cx);
            area.move_panel(
                panel,
                InsertTarget::Split {
                    node,
                    placement: Placement::Right,
                    size: None,
                },
                window,
                cx,
            );
            assert_eq!(area.zoomed_group(), Some(node));
        });
    });
    settle(cx);
}

#[gpui::test]
fn bottom_hide_retains_topology_and_only_clears_zoom_of_that_placement(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let a_node = cx.read(|cx| f.a.read(cx).node());
    cx.update(|window, cx| {
        f.area.update(cx, |area, cx| {
            let bottom = TestPanel::new("bottom", cx);
            let id = PanelId::from(bottom.entity_id());
            area.add_panel_view(
                std::sync::Arc::new(bottom),
                DockPlacement::Bottom,
                Some(gpui::px(260.)),
                window,
                cx,
            );
            let node = area
                .layout(DockPlacement::Bottom)
                .unwrap()
                .find_panel_node(id)
                .unwrap();
            area.set_zoomed_in(a_node, window, cx);
            area.toggle_dock(DockPlacement::Bottom, window, cx);
            assert_eq!(area.zoomed_group(), Some(a_node));
            assert_eq!(
                area.layout(DockPlacement::Bottom)
                    .unwrap()
                    .find_panel_node(id),
                Some(node)
            );
            area.toggle_dock(DockPlacement::Bottom, window, cx);
            assert_eq!(area.dock_size(DockPlacement::Bottom), Some(gpui::px(260.)));
            area.set_zoomed_in(node, window, cx);
            area.toggle_dock(DockPlacement::Bottom, window, cx);
            assert!(!area.is_zoomed());
            assert_eq!(
                area.layout(DockPlacement::Bottom)
                    .unwrap()
                    .find_panel_node(id),
                Some(node)
            );
        });
    });
    settle(cx);
    cx.read(|cx| assert!(!f.area.read(cx).is_zoomed()));
}

#[gpui::test]
fn inter_region_last_tab_drag_is_opt_in_and_does_not_relax_close_or_lock_guards(
    cx: &mut TestAppContext,
) {
    let (f, cx) = fixture(cx);
    cx.update(|window, cx| {
        f.area.update(cx, |area, cx| {
            let bottom = TestPanel::new("last-bottom", cx);
            let id = PanelId::from(bottom.entity_id());
            area.add_panel(bottom, DockPlacement::Bottom, None, window, cx);
            let node = area
                .layout(DockPlacement::Bottom)
                .unwrap()
                .find_panel_node(id)
                .unwrap();
            let group = area.groups[&node].entity.clone();
            assert!(!group.read(cx).context(cx).is_draggable());
            area.set_last_panel_drag_between_regions(true, window, cx);
            assert!(group.read(cx).context(cx).is_draggable());
            assert!(!group.read(cx).context(cx).is_panel_closable(id, cx));
            area.set_locked(true, window, cx);
            assert!(!group.read(cx).context(cx).is_draggable());
            area.set_locked(false, window, cx);
            assert!(group.read(cx).context(cx).is_draggable());
            area.set_center(DockLayout::h_split(), window, cx);
            assert!(!group.read(cx).context(cx).is_draggable());
        });
    });
    settle(cx);
}
