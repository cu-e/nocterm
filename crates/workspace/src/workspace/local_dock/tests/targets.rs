//! Explicit opener destinations and registration ownership.
use super::*;
use crate::LocalTerminalTarget;
use gpui_kit::component::dock::InsertTarget;

#[gpui_kit::test]
fn explicit_destination_resolves_moved_anchor_and_reveals_closed_region(cx: &mut TestAppContext) {
    let (window, workspace, closes) = fixture(cx);
    window
        .update(cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                let central = probe(cx, "/central", closes.clone());
                workspace.add_item(central.clone(), window, cx);
                let anchor = probe(cx, "/anchor", closes.clone());
                workspace.add_local_terminal(anchor.clone(), window, cx);
                let request = LocalTerminalTarget::Beside(anchor.entity_id());
                let destination = workspace.item_location(central.entity_id(), cx).unwrap().1;
                let panel = workspace.panel_of(anchor.entity_id()).unwrap();
                workspace.dock.update(cx, |dock, cx| {
                    dock.move_panel(
                        panel,
                        InsertTarget::Tabs {
                            node: destination,
                            ix: None,
                            activate: true,
                        },
                        window,
                        cx,
                    );
                });
                let added = probe(cx, "/added", closes.clone());
                assert!(workspace.add_local_terminal_at(added.clone(), request, window, cx));
                assert_eq!(
                    workspace.item_location(added.entity_id(), cx),
                    workspace.item_location(anchor.entity_id(), cx)
                );
                assert_eq!(
                    workspace.item_location(added.entity_id(), cx).unwrap().0,
                    DockPlacement::Center
                );
                let bottom = probe(cx, "/bottom", closes.clone());
                workspace.add_local_terminal(bottom.clone(), window, cx);
                workspace.toggle_local_terminal(window, cx);
                assert!(!workspace.dock.read(cx).is_dock_open(DockPlacement::Bottom));
                let added = probe(cx, "/revealed", closes.clone());
                assert!(workspace.add_local_terminal_at(
                    added.clone(),
                    LocalTerminalTarget::Beside(bottom.entity_id()),
                    window,
                    cx
                ));
                assert!(workspace.dock.read(cx).is_dock_open(DockPlacement::Bottom));
                assert_eq!(
                    workspace.item_location(added.entity_id(), cx),
                    workspace.item_location(bottom.entity_id(), cx)
                );
                assert_eq!(closes.get(), 0);
            });
        })
        .unwrap();
}

#[gpui_kit::test]
fn rejected_destination_closes_only_new_item_once(cx: &mut TestAppContext) {
    let (window, workspace, _) = fixture(cx);
    let registered_closes = Rc::new(Cell::new(0));
    let rejected_closes = Rc::new(Cell::new(0));
    window
        .update(cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                let stale = probe(cx, "/stale", Rc::default());
                let registered = probe(cx, "/registered", registered_closes.clone());
                workspace.add_local_terminal(registered.clone(), window, cx);
                let background = probe(cx, "/background", registered_closes.clone());
                workspace.add_background_item(background.clone(), window, cx);
                let rejected = probe(cx, "/rejected", rejected_closes.clone());
                let target = LocalTerminalTarget::Beside(stale.entity_id());
                assert!(!workspace.add_local_terminal_at(rejected.clone(), target, window, cx));
                assert!(!workspace.add_local_terminal_at(registered.clone(), target, window, cx));
                assert!(!workspace.add_local_terminal_at(background.clone(), target, window, cx));
                assert!(workspace.is_background(background.entity_id()));
                assert_eq!(rejected_closes.get(), 1);
                assert_eq!(registered_closes.get(), 0);
                assert_eq!(workspace.items.len(), 1);
                assert!(workspace.local_entry(rejected.entity_id()).is_none());
                assert!(workspace.local_entry(registered.entity_id()).is_some());
            });
        })
        .unwrap();
}
