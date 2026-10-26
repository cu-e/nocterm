//! Rejected completion closes only its new resource, never another surface's Item.
use super::*;

#[gpui_kit::test]
fn the_original_initial_terminal_intent_survives_hiding_but_cannot_be_issued_twice(
    cx: &mut TestAppContext,
) {
    let (window, workspace) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        let (_, a) = add_project(&workspace, 1, window, cx);
        let (_, b) = add_project(&workspace, 2, window, cx);
        let target = a.initial_terminal_target(window, cx).unwrap();
        target.validate(window, cx).unwrap();
        assert!(a.initial_terminal_target(window, cx).is_err());
        assert!(
            a.terminal_target(a.selection_receipt(cx), None, window, cx)
                .is_err()
        );
        let terminal = probe(cx, Rc::default());
        crate::admit_project_terminal(&target, terminal.clone(), window, cx).unwrap();
        assert_eq!(a.tabs(cx)[0].id(), terminal.entity_id());
        assert_eq!(
            workspace.read(cx).selected_context(cx),
            ContextId::Project(b.project())
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_new_target_is_refused_immediately_after_its_outer_tab_hides(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let (outer_a, a, outer_b) = cx
        .update_window(window, |_, window, cx| {
            let (outer_a, a) = add_project(&workspace, 1, window, cx);
            let (outer_b, _) = add_project(&workspace, 2, window, cx);
            workspace.update(cx, |workspace, cx| {
                workspace.activate_item_by_id(outer_a.entity_id(), window, cx);
            });
            (outer_a, a, outer_b)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        assert!(
            a.terminal_target(a.selection_receipt(cx), None, window, cx)
                .is_ok()
        );
        workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(outer_b.entity_id(), window, cx);
        });
        assert!(
            a.terminal_target(a.selection_receipt(cx), None, window, cx)
                .is_err()
        );
        workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(outer_a.entity_id(), window, cx);
        });
        assert!(
            a.terminal_target(a.selection_receipt(cx), None, window, cx)
                .is_ok()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn closing_and_reopening_the_same_project_never_revives_its_old_source(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        let (outer, old) = add_project(&workspace, 1, window, cx);
        let captured = old.selection_receipt(cx);
        workspace.update(cx, |workspace, cx| {
            workspace.close_tabs(outer.entity_id(), crate::TabCloseScope::Current, window, cx)
        });
        let (_, fresh) = add_project(&workspace, 1, window, cx);
        assert_ne!(old.source(cx), fresh.source(cx));
        let closes = Rc::new(Cell::new(0));
        let terminal = probe(cx, closes.clone());
        assert!(matches!(
            old.add_item(captured, terminal, window, cx),
            Err(crate::ContextSelectionError::StaleSurface)
        ));
        assert_eq!(closes.get(), 1);
        assert!(fresh.tabs(cx).is_empty());
        assert_eq!(
            workspace.read(cx).selected_context(cx),
            ContextId::Project(fresh.project())
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn delayed_general_completion_stays_in_outer_connections_while_a_workspace_is_active(
    cx: &mut TestAppContext,
) {
    let (window, workspace) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        let source = workspace.read(cx).general_surface_source(cx);
        let captured = workspace.read(cx).context_selection_receipt(cx);
        let (outer, surface) = add_project(&workspace, 1, window, cx);
        let terminal = probe(cx, Rc::default());
        workspace.update(cx, |workspace, cx| {
            assert!(
                workspace
                    .add_general_item_from(captured, terminal.clone(), window, cx)
                    .unwrap()
            )
        });
        assert_eq!(
            workspace.read(cx).active_item(cx).unwrap().item_id(),
            outer.entity_id()
        );
        assert_eq!(
            workspace.read(cx).selected_context(cx),
            ContextId::Project(surface.project())
        );
        assert_eq!(workspace.read(cx).general_surface_source(cx), source);
        assert!(surface.tabs(cx).is_empty());
        assert_eq!(workspace.read(cx).items(cx).count(), 2);
    })
    .unwrap();
}

#[gpui_kit::test]
fn another_native_window_cannot_admit_the_original_project_target(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let (other, _) = fixture(cx);
    let surface = cx
        .update_window(window, |_, window, cx| {
            add_project(&workspace, 1, window, cx).1
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(other, |_, window, cx| {
        assert!(matches!(
            surface.terminal_target(surface.selection_receipt(cx), None, window, cx),
            Err(crate::ContextSelectionError::StaleWindow)
        ));
    })
    .unwrap();
}

#[gpui_kit::test]
fn background_admission_rejects_closed_sources_without_closing_foreign_members(
    cx: &mut TestAppContext,
) {
    let (window, workspace) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        let (outer_a, a) = add_project(&workspace, 1, window, cx);
        let (_, b) = add_project(&workspace, 2, window, cx);
        let closes = Rc::new(Cell::new(0));
        let foreign = probe(cx, closes.clone());
        b.add_item(b.selection_receipt(cx), foreign.clone(), window, cx)
            .unwrap();
        let original = a.source(cx);
        workspace.update(cx, |workspace, cx| {
            workspace.close_tabs(
                outer_a.entity_id(),
                crate::TabCloseScope::Current,
                window,
                cx,
            );
            assert!(matches!(
                workspace.validate_context_source(original, window, cx),
                Err(crate::ContextSelectionError::StaleSurface)
            ));
            assert!(
                workspace
                    .add_context_background_from(original, foreign.clone(), window, cx)
                    .is_err()
            );
        });
        assert_eq!(closes.get(), 0);
        assert_eq!(b.tabs(cx)[0].id(), foreign.entity_id());
        let rejected_closes = Rc::new(Cell::new(0));
        let rejected = probe(cx, rejected_closes.clone());
        workspace.update(cx, |workspace, cx| {
            assert!(
                workspace
                    .add_context_background_from(original, rejected, window, cx)
                    .is_err()
            );
        });
        assert_eq!(rejected_closes.get(), 1);
        assert!(a.tabs(cx).is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn background_admission_checks_the_original_native_window_before_registration(
    cx: &mut TestAppContext,
) {
    let (window, workspace) = fixture(cx);
    let (other, _) = fixture(cx);
    let source = cx
        .update_window(window, |_, window, cx| {
            add_project(&workspace, 1, window, cx).1.source(cx)
        })
        .unwrap();
    cx.update_window(other, |_, window, cx| {
        let closes = Rc::new(Cell::new(0));
        let rejected = probe(cx, closes.clone());
        workspace.update(cx, |workspace, cx| {
            assert!(matches!(
                workspace.validate_context_source(source, window, cx),
                Err(crate::ContextSelectionError::StaleWindow)
            ));
            assert!(matches!(
                workspace.add_context_background_from(source, rejected, window, cx),
                Err(crate::ContextSelectionError::StaleWindow)
            ));
        });
        assert_eq!(closes.get(), 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn general_admission_keeps_central_local_and_background_members_separate(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        let closes = Rc::new(Cell::new(0));
        let central = probe(cx, closes.clone());
        let local = probe(cx, closes.clone());
        let background = probe(cx, closes.clone());
        workspace.update(cx, |workspace, cx| {
            workspace.add_item(central.clone(), window, cx);
            workspace.add_local_terminal(local.clone(), window, cx);
            workspace.add_background_item(background.clone(), window, cx);
        });
        assert_eq!(workspace.read(cx).tabs(cx).len(), 2);
        assert_eq!(
            workspace.read(cx).selected_local_terminal(cx),
            Some(local.entity_id())
        );
        assert!(workspace.read(cx).is_background(background.entity_id(), cx));
        workspace.update(cx, |workspace, cx| {
            workspace.close_tabs(
                central.entity_id(),
                crate::TabCloseScope::Current,
                window,
                cx,
            );
            workspace.close_local_terminal(window, cx);
            workspace.close_background_session(background.entity_id(), window, cx);
        });
        assert_eq!(closes.get(), 3);
        assert!(workspace.read(cx).tabs(cx).is_empty());
        assert!(!workspace.read(cx).is_background(background.entity_id(), cx));
    })
    .unwrap();
}

#[gpui_kit::test]
fn project_admission_closes_each_kind_once_without_closing_the_outer_item(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        let (_, surface) = add_project(&workspace, 1, window, cx);
        let closes = Rc::new(Cell::new(0));
        let central = probe(cx, closes.clone());
        let local = probe(cx, closes.clone());
        let background = probe(cx, closes.clone());
        surface
            .add_item(surface.selection_receipt(cx), central.clone(), window, cx)
            .unwrap();
        surface
            .add_local_terminal(
                surface.selection_receipt(cx),
                local.clone(),
                crate::LocalTerminalTarget::Bottom,
                window,
                cx,
            )
            .unwrap();
        surface
            .add_background_item(background.clone(), window, cx)
            .unwrap();
        assert_eq!(surface.tabs(cx).len(), 2);
        for item in [central.entity_id(), local.entity_id()] {
            surface
                .close_tabs(
                    surface.selection_receipt(cx),
                    item,
                    crate::TabCloseScope::Current,
                    window,
                    cx,
                )
                .unwrap();
        }
        workspace.update(cx, |workspace, cx| {
            workspace
                .close_context_background(surface.source(cx), background.entity_id(), window, cx)
                .unwrap();
        });
        assert_eq!(closes.get(), 3);
        assert!(surface.tabs(cx).is_empty());
        assert_eq!(workspace.read(cx).items(cx).count(), 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn duplicate_outer_project_admission_keeps_the_original_workspace(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        let (outer, surface) = add_project(&workspace, 1, window, cx);
        let factory = workspace.read(cx).project_surface_factory();
        let duplicate = cx.new(|cx| {
            super::project_fixtures::ProjectTab::new(factory, surface.project(), window, cx)
                .unwrap()
        });
        let other = duplicate.read(cx).surface();
        workspace.update(cx, |workspace, cx| {
            assert!(
                !workspace
                    .add_project_workspace(duplicate, other, window, cx)
                    .unwrap()
            );
            assert!(
                !workspace
                    .add_project_workspace(outer.clone(), surface.clone(), window, cx)
                    .unwrap()
            );
        });
        assert_eq!(workspace.read(cx).items(cx).count(), 1);
        assert_eq!(workspace.read(cx).context_source(cx), surface.source(cx));
        assert_eq!(
            workspace.read(cx).active_item(cx).unwrap().item_id(),
            outer.entity_id()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn foreign_terminal_destinations_preserve_item_ownership_until_the_caller_cleans_up(
    cx: &mut TestAppContext,
) {
    use nocterm_workspace_ports::{ContextSelectionError, ProjectTerminalTarget};
    let (window, workspace) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        let (_, surface) = add_project(&workspace, 1, window, cx);
        let owned_closes = Rc::new(Cell::new(0));
        let owned = probe(cx, owned_closes.clone());
        surface
            .add_item(surface.selection_receipt(cx), owned.clone(), window, cx)
            .unwrap();
        let foreign = ProjectTerminalTarget::new(
            cx.new(|_| crate::test_support::ProjectTerminalProbe::default()),
        );
        assert!(foreign.validate(window, cx).is_ok());
        assert!(matches!(
            crate::validate_project_terminal_target(&foreign, window, cx),
            Err(ContextSelectionError::StaleSurface)
        ));
        assert!(matches!(
            crate::admit_project_terminal(&foreign, owned, window, cx),
            Err(ContextSelectionError::StaleSurface)
        ));
        assert_eq!(owned_closes.get(), 0);
        let closes = Rc::new(Cell::new(0));
        let fresh = probe(cx, closes.clone());
        assert!(matches!(
            crate::admit_project_terminal(&foreign, fresh.clone(), window, cx),
            Err(ContextSelectionError::StaleSurface)
        ));
        assert_eq!(closes.get(), 0);
        assert_eq!(surface.tabs(cx).len(), 1);
        crate::ItemHandle::close(&fresh, window, cx);
        assert_eq!(closes.get(), 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn genuine_stale_terminal_destinations_close_only_new_unowned_items(cx: &mut TestAppContext) {
    use nocterm_workspace_ports::ContextSelectionError;
    let (window, workspace) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        let (_, a) = add_project(&workspace, 1, window, cx);
        let owned_closes = Rc::new(Cell::new(0));
        let owned = probe(cx, owned_closes.clone());
        a.add_item(a.selection_receipt(cx), owned.clone(), window, cx)
            .unwrap();
        let target = a
            .terminal_target(a.selection_receipt(cx), None, window, cx)
            .unwrap();
        let (_, b) = add_project(&workspace, 2, window, cx);
        assert!(matches!(
            crate::admit_project_terminal(&target, owned, window, cx),
            Err(ContextSelectionError::StaleSelection)
        ));
        assert_eq!(owned_closes.get(), 0);
        let foreign_closes = Rc::new(Cell::new(0));
        let foreign = probe(cx, foreign_closes.clone());
        b.add_item(b.selection_receipt(cx), foreign.clone(), window, cx)
            .unwrap();
        assert!(matches!(
            crate::admit_project_terminal(&target, foreign, window, cx),
            Err(ContextSelectionError::StaleSelection)
        ));
        assert_eq!(foreign_closes.get(), 0);
        let closes = Rc::new(Cell::new(0));
        let fresh = probe(cx, closes.clone());
        assert!(matches!(
            crate::admit_project_terminal(&target, fresh, window, cx),
            Err(ContextSelectionError::StaleSelection)
        ));
        assert_eq!(closes.get(), 1);
        assert_eq!(a.tabs(cx).len(), 1);
        assert_eq!(b.tabs(cx).len(), 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn captured_local_terminal_admission_uses_its_anchor_pane_instead_of_the_active_pane(
    cx: &mut TestAppContext,
) {
    let (window, workspace) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        let (_, surface) = add_project(&workspace, 1, window, cx);
        let anchor = probe(cx, Rc::default());
        let other = probe(cx, Rc::default());
        for item in [&anchor, &other] {
            surface
                .add_item(surface.selection_receipt(cx), item.clone(), window, cx)
                .unwrap();
        }
        surface
            .split_item(
                surface.selection_receipt(cx),
                anchor.entity_id(),
                Placement::Right,
                window,
                cx,
            )
            .unwrap();
        surface
            .activate_item(surface.selection_receipt(cx), other.entity_id(), window, cx)
            .unwrap();
        let target = surface
            .terminal_target(
                surface.selection_receipt(cx),
                Some(crate::LocalTerminalTarget::Beside(anchor.entity_id())),
                window,
                cx,
            )
            .unwrap();
        let terminal = probe(cx, Rc::default());
        assert!(crate::admit_project_terminal(&target, terminal.clone(), window, cx).unwrap());
        let panes = surface.panes(DockPlacement::Center, cx);
        assert_eq!(panes.len(), 2);
        let pane = panes
            .iter()
            .find(|pane| pane.items().contains(&anchor.entity_id()))
            .unwrap();
        assert_eq!(pane.items(), [anchor.entity_id(), terminal.entity_id()]);
        assert!(!pane.items().contains(&other.entity_id()));
        assert!(
            surface
                .tabs(cx)
                .iter()
                .find(|tab| tab.id() == terminal.entity_id())
                .unwrap()
                .is_local()
        );
        assert!(surface.panes(DockPlacement::Bottom, cx).is_empty());
    })
    .unwrap();
}
