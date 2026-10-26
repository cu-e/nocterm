//! Workspaces are actual outer Items; their inner native docks retain identity.
use super::*;
use nocterm_project_model::ContextId;
#[path = "project_fixtures.rs"]
mod project_fixtures;
use project_fixtures::add_project;
#[path = "project_tabs/actions.rs"]
mod actions;
#[path = "project_tabs/admission.rs"]
mod admission;
#[path = "project_tabs/filesystem.rs"]
mod filesystem;
#[path = "project_tabs/gestures.rs"]
mod gestures;
#[path = "project_tabs/lifecycle.rs"]
mod lifecycle;

#[gpui_kit::test]
fn workspace_tabs_preserve_outer_connections_and_original_inner_membership(
    cx: &mut TestAppContext,
) {
    let (window, workspace) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        let server1 = probe(cx, Rc::new(Cell::new(0)));
        let server2 = probe(cx, Rc::new(Cell::new(0)));
        workspace.update(cx, |workspace, cx| {
            workspace.add_item(server1.clone(), window, cx);
            workspace.add_item(server2.clone(), window, cx);
        });
        let (a, a_surface) = add_project(&workspace, 1, window, cx);
        let (b, b_surface) = add_project(&workspace, 2, window, cx);
        let terminal = probe(cx, Rc::new(Cell::new(0)));
        let original = a_surface.source(cx);
        a_surface
            .add_item(
                a_surface.selection_receipt(cx),
                terminal.clone(),
                window,
                cx,
            )
            .unwrap();
        assert_eq!(
            workspace
                .read(cx)
                .items(cx)
                .map(|item| item.item_id())
                .collect::<Vec<_>>(),
            [
                server1.entity_id(),
                server2.entity_id(),
                a.entity_id(),
                b.entity_id()
            ]
        );
        assert_eq!(
            workspace.read(cx).selected_context(cx),
            ContextId::Project(b_surface.project())
        );
        workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(a.entity_id(), window, cx);
        });
        window.render_frame(cx);
        assert_eq!(
            workspace.read(cx).selected_context(cx),
            ContextId::Project(a_surface.project())
        );
        assert_eq!(a_surface.source(cx), original);
        assert_eq!(
            a_surface
                .tabs(cx)
                .iter()
                .map(|tab| tab.id())
                .collect::<Vec<_>>(),
            [terminal.entity_id()]
        );
        assert!(b_surface.tabs(cx).is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_foreign_registered_item_is_neither_closed_nor_retagged_by_another_surface(
    cx: &mut TestAppContext,
) {
    let (window, workspace) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        let (_, a) = add_project(&workspace, 1, window, cx);
        let (_, b) = add_project(&workspace, 2, window, cx);
        let closes = Rc::new(Cell::new(0));
        let terminal = probe(cx, closes.clone());

        a.add_item(a.selection_receipt(cx), terminal.clone(), window, cx)
            .unwrap();
        assert!(
            !b.add_item(b.selection_receipt(cx), terminal.clone(), window, cx)
                .unwrap()
        );
        assert_eq!(closes.get(), 0);
        assert_eq!(a.tabs(cx).len(), 1);
        assert!(b.tabs(cx).is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn closing_workspace_closes_its_original_inner_items_once_and_leaves_servers(
    cx: &mut TestAppContext,
) {
    let (window, workspace) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        let server = probe(cx, Rc::new(Cell::new(0)));
        workspace.update(cx, |workspace, cx| {
            workspace.add_item(server.clone(), window, cx)
        });
        let (outer, surface) = add_project(&workspace, 1, window, cx);
        let closes = Rc::new(Cell::new(0));
        let terminal = probe(cx, closes.clone());
        surface
            .add_item(surface.selection_receipt(cx), terminal, window, cx)
            .unwrap();
        let original = surface.source(cx);
        workspace.update(cx, |workspace, cx| {
            workspace.close_tabs(outer.entity_id(), crate::TabCloseScope::Current, window, cx);
        });
        assert_eq!(closes.get(), 1);
        assert_eq!(
            workspace
                .read(cx)
                .items(cx)
                .map(|item| item.item_id())
                .collect::<Vec<_>>(),
            [server.entity_id()]
        );
        assert_eq!(workspace.read(cx).selected_context(cx), ContextId::General);
        surface.close(window, cx);
        assert_eq!(closes.get(), 1);
        assert!(workspace.read(cx).context_terminals(original, cx).is_err());
    })
    .unwrap();
}
