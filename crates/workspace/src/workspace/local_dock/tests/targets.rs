//! Explicit opener destinations and registration ownership.
use super::*;
use crate::LocalTerminalTarget;

#[gpui_kit::test]
fn explicit_destination_resolves_moved_anchor_and_reveals_closed_region(cx: &mut TestAppContext) {
    let (window, workspace, closes) = fixture(cx);
    let (central, anchor, request) = window
        .update(cx, |_, window, cx| {
            let central = probe(cx, "/central", closes.clone());
            let anchor = probe(cx, "/anchor", closes.clone());
            let request = workspace.update(cx, |workspace, cx| {
                workspace.add_item(central.clone(), window, cx);
                workspace.add_local_terminal(anchor.clone(), window, cx);
                LocalTerminalTarget::Beside(anchor.entity_id())
            });
            (central, anchor, request)
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |_, window, cx| {
            crate::workspace::tests::drag_tab(window, anchor.entity_id(), central.entity_id(), cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                let added = probe(cx, "/added", closes.clone());
                assert!(workspace.add_local_terminal_at(added.clone(), request, window, cx));
                assert_eq!(
                    workspace.tab_location(added.entity_id(), cx),
                    workspace.tab_location(anchor.entity_id(), cx)
                );
                assert_eq!(
                    workspace.tab_location(added.entity_id(), cx).unwrap().0,
                    DockPlacement::Center
                );
                let bottom = probe(cx, "/bottom", closes.clone());
                workspace.add_local_terminal(bottom.clone(), window, cx);
                workspace.toggle_local_terminal(window, cx);
                assert!(!workspace.dock_region_open(DockPlacement::Bottom, cx));
                let added = probe(cx, "/revealed", closes.clone());
                assert!(workspace.add_local_terminal_at(
                    added.clone(),
                    LocalTerminalTarget::Beside(bottom.entity_id()),
                    window,
                    cx
                ));
                assert!(workspace.dock_region_open(DockPlacement::Bottom, cx));
                assert_eq!(
                    workspace.tab_location(added.entity_id(), cx),
                    workspace.tab_location(bottom.entity_id(), cx)
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
                assert!(workspace.is_background(background.entity_id(), cx));
                assert_eq!(rejected_closes.get(), 1);
                assert_eq!(registered_closes.get(), 0);
                assert_eq!(workspace.tabs(cx).len(), 1);
                assert!(
                    !workspace
                        .tabs(cx)
                        .iter()
                        .any(|tab| tab.id() == rejected.entity_id() && tab.is_local())
                );
                assert!(
                    workspace
                        .tabs(cx)
                        .iter()
                        .any(|tab| tab.id() == registered.entity_id() && tab.is_local())
                );
            });
        })
        .unwrap();
}
