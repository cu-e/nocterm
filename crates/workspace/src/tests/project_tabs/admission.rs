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
        target
            .surface()
            .add_item(target.selection(), terminal.clone(), window, cx)
            .unwrap();
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
