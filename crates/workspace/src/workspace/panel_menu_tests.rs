//! The panels the window menu lists: the registered ones the selected layout places.
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};

use crate::{
    PanelRegistration,
    test_support::{PANEL_A, PANEL_B, TRAY_PANEL, TestPanel},
    workspace::tests::test_plugin::fixture_with_test_plugin,
};

#[gpui_kit::test]
fn the_menu_lists_registered_panels_of_the_layout_and_leaves_out_the_tray(cx: &mut TestAppContext) {
    let (handle, workspace, _) = fixture_with_test_plugin(cx);
    cx.update(|cx| assert!(workspace.read(cx).panel_menu().is_empty()));
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            for contribution in [PANEL_A, TRAY_PANEL] {
                let panel = TestPanel::builder(contribution.title).build(cx);
                workspace
                    .register_panel(
                        PanelRegistration::new(contribution, move |_, _, _| panel.clone()),
                        window,
                        cx,
                    )
                    .expect("the panel is in the test catalog");
            }
            // PANEL_B is in the catalog but never registered.
            assert!(!workspace.is_registered(&PANEL_B.id));
        });
    })
    .unwrap();
    cx.update(|cx| {
        let menu = workspace.read(cx).panel_menu();
        let listed: Vec<Vec<_>> = menu
            .iter()
            .map(|group| group.iter().map(|entry| entry.title()).collect())
            .collect();
        assert_eq!(listed, [[PANEL_A.title]]);
        assert_eq!(
            menu[0][0].is_open(),
            workspace.read(cx).panel_is_open(&PANEL_A.id)
        );
    });
}

#[gpui_kit::test]
fn toggling_a_named_panel_opens_it_and_a_name_outside_the_catalog_does_nothing(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, _) = fixture_with_test_plugin(cx);
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            let panel = TestPanel::builder("Panel B").build(cx);
            workspace
                .register_panel(
                    PanelRegistration::new(PANEL_B, move |_, _, _| panel.clone()),
                    window,
                    cx,
                )
                .expect("the panel is in the test catalog");
        });
    })
    .unwrap();
    let toggle = |cx: &mut TestAppContext, id: &str| {
        cx.update_window(handle, |_, window, cx| {
            let focus = workspace.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            window.dispatch_action(
                Box::new(crate::TogglePanel {
                    id: id.to_owned().into(),
                }),
                cx,
            )
        })
        .unwrap();
        cx.run_until_parked();
    };
    let is_open =
        |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).panel_is_open(&PANEL_B.id));
    let before = is_open(cx);
    toggle(cx, "nobody.nothing");
    assert_eq!(is_open(cx), before);
    toggle(cx, PANEL_B.id.as_str());
    assert_ne!(is_open(cx), before);
}
