//! Live original events survive hiding while deferred UI selections expire.
use super::*;

#[gpui_kit::test]
fn promoting_a_hidden_general_chat_background_keeps_the_selected_workspace(
    cx: &mut TestAppContext,
) {
    let (window, workspace) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        let source = workspace.read(cx).general_surface_source(cx);
        let background = probe(cx, Rc::default());
        workspace.update(cx, |workspace, cx| {
            workspace.add_background_item(background.clone(), window, cx);
        });
        let (outer, surface) = add_project(&workspace, 1, window, cx);
        workspace.update(cx, |workspace, cx| {
            assert!(
                workspace
                    .show_context_background(source, background.entity_id(), window, cx)
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
        assert_eq!(workspace.read(cx).items(cx).count(), 2);
    })
    .unwrap();
}

#[gpui_kit::test]
fn hidden_completion_registers_only_original_surface_and_never_steals_focus(
    cx: &mut TestAppContext,
) {
    let (window, workspace) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        let (outer_a, a) = add_project(&workspace, 1, window, cx);
        window.render_frame(cx);
        let captured = a.selection_receipt(cx);
        let (outer_b, b) = add_project(&workspace, 2, window, cx);
        window.render_frame(cx);
        let focus = b.focus_handle(cx);
        assert!(focus.is_focused(window));
        let terminal = probe(cx, Rc::default());
        a.add_item(captured, terminal.clone(), window, cx).unwrap();
        assert!(focus.is_focused(window));
        assert_eq!(
            workspace.read(cx).selected_context(cx),
            ContextId::Project(b.project())
        );
        assert_eq!(a.tabs(cx)[0].id(), terminal.entity_id());
        workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(outer_a.entity_id(), window, cx);
        });
        window.render_frame(cx);
        assert!(!a.accepts_selection(captured, cx));
        assert!(
            a.activate_item(captured, terminal.entity_id(), window, cx)
                .is_err()
        );
        assert_eq!(
            workspace
                .read(cx)
                .items(cx)
                .map(|item| item.item_id())
                .collect::<Vec<_>>(),
            [outer_a.entity_id(), outer_b.entity_id()]
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_fresh_visible_inactive_workspace_can_focus_without_a_global_mode_switch(
    cx: &mut TestAppContext,
) {
    let (window, workspace) = fixture(cx);
    let (outer_a, a, b, terminal) = cx
        .update_window(window, |_, window, cx| {
            let (outer_a, a) = add_project(&workspace, 1, window, cx);
            let terminal = probe(cx, Rc::default());
            a.add_item(a.selection_receipt(cx), terminal.clone(), window, cx)
                .unwrap();
            let (_, b) = add_project(&workspace, 2, window, cx);
            workspace.update(cx, |workspace, cx| {
                workspace.split_item(outer_a.entity_id(), Placement::Right, window, cx)
            });
            window.render_frame(cx);
            assert_eq!(
                workspace.read(cx).selected_context(cx),
                ContextId::Project(b.project())
            );
            (outer_a, a, b, terminal)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            workspace.read(cx).selected_context(cx),
            ContextId::Project(b.project())
        );
        assert!(a.accepts_selection(a.selection_receipt(cx), cx));
        assert!(
            a.activate_item(a.selection_receipt(cx), terminal.entity_id(), window, cx)
                .unwrap()
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            workspace.read(cx).active_item(cx).unwrap().item_id(),
            outer_a.entity_id()
        );
        assert_eq!(
            workspace.read(cx).selected_context(cx),
            ContextId::Project(a.project())
        );
        assert!(terminal.focus_handle(cx).is_focused(window));
        assert_eq!(
            workspace
                .read(cx)
                .tab_panes(DockPlacement::Center, cx)
                .len(),
            2
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn hidden_background_events_and_close_stay_on_the_original_surface(cx: &mut TestAppContext) {
    let (window, workspace) = fixture(cx);
    let (a, b, background) = cx
        .update_window(window, |_, window, cx| {
            let (_, a) = add_project(&workspace, 1, window, cx);

            let background = probe(cx, Rc::default());
            a.add_background_item(background.clone(), window, cx)
                .unwrap();
            let (_, b) = add_project(&workspace, 2, window, cx);
            (a, b, background)
        })
        .unwrap();
    cx.run_until_parked();
    let changes = Rc::new(Cell::new(0));
    let observed = changes.clone();
    let original = cx.update(|cx| a.source(cx));
    let _events = cx.update(|cx| {
        cx.subscribe(&workspace, move |_, event, _| {
            if *event == crate::WorkspaceEvent::SurfaceItemsChanged(original) {
                observed.set(observed.get() + 1)
            }
        })
    });
    cx.update_window(window, |_, _, cx| {
        background.update(cx, |_, cx| cx.emit(ItemEvent::Changed))
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        assert!(changes.get() > 0);
        assert_eq!(
            workspace.read(cx).selected_context(cx),
            ContextId::Project(b.project())
        );
        workspace.update(cx, |workspace, cx| {
            workspace
                .close_context_background(b.source(cx), background.entity_id(), window, cx)
                .unwrap();
        });
        assert_eq!(background.read(cx).closes.get(), 0);
        workspace.update(cx, |workspace, cx| {
            workspace
                .close_context_background(a.source(cx), background.entity_id(), window, cx)
                .unwrap()
        });
        assert_eq!(background.read(cx).closes.get(), 1);
        assert_eq!(
            workspace.read(cx).selected_context(cx),
            ContextId::Project(b.project())
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn source_bound_background_admission_and_promotion_keep_hidden_workspace_focus(
    cx: &mut TestAppContext,
) {
    let (window, workspace) = fixture(cx);
    let closes = Rc::new(Cell::new(0));
    let (original, b, background) = cx
        .update_window(window, |_, window, cx| {
            let (_, a) = add_project(&workspace, 1, window, cx);
            let (_, b) = add_project(&workspace, 2, window, cx);
            window.render_frame(cx);
            let focus = b.focus_handle(cx);
            let background = probe(cx, closes.clone());
            let original = a.source(cx);
            workspace.update(cx, |workspace, cx| {
                assert_eq!(
                    workspace
                        .add_context_background_from(original, background.clone(), window, cx)
                        .unwrap(),
                    background.entity_id()
                );
                assert_eq!(
                    workspace
                        .add_context_background_from(original, background.clone(), window, cx)
                        .unwrap(),
                    background.entity_id()
                );
                assert!(
                    workspace
                        .show_context_background(original, background.entity_id(), window, cx)
                        .unwrap()
                );
            });
            assert!(focus.is_focused(window));
            assert_eq!(
                a.tabs(cx)
                    .iter()
                    .map(crate::TabSnapshot::id)
                    .collect::<Vec<_>>(),
                [background.entity_id()]
            );
            assert_eq!(
                workspace.read(cx).selected_context(cx),
                ContextId::Project(b.project())
            );
            (original, b, background)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, _, cx| {
        background.update(cx, |_, cx| cx.emit(ItemEvent::CloseRequested));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, _, cx| {
        assert_eq!(closes.get(), 1);
        assert!(
            workspace
                .read(cx)
                .context_terminals(original, cx)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            workspace.read(cx).selected_context(cx),
            ContextId::Project(b.project())
        );
    })
    .unwrap();
}
