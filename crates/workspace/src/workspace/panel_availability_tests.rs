//! A sidebar panel the host does not offer stays out of the footer, `NextPanel` and the panel
//! queries, and the panels that remain keep working by their position in the sidebar.
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, TestAppContext, px, size, test::TestWindowExt as _,
};

use super::PanelRegistration;
use crate::{
    NextPanel, Workspace,
    test_support::{
        CORE_PANEL, PANEL_A, PANEL_B, TEST_CORE, TEST_PLUGIN, TestPanel, Toggle, deps_for_core,
        toggle_id,
    },
    workspace::tests::fixture_with_deps,
};
use nocterm_plugin_model::SystemContribution;

const SIDEBAR: [&SystemContribution; 3] = [PANEL_A, PANEL_B, CORE_PANEL];

/// A window whose sidebar holds the three registered panels, the first one showing.
fn sidebar(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    let (handle, workspace, _) =
        fixture_with_deps(cx, |_, cx| deps_for_core(&[TEST_CORE], &[&TEST_PLUGIN], cx));
    cx.update_window(handle, |_, window, cx| {
        window.resize(size(px(1400.), px(700.)));
        workspace.update(cx, |workspace, cx| {
            for contribution in SIDEBAR {
                let registration = PanelRegistration::new(contribution, move |_, _, cx| {
                    TestPanel::builder(contribution.title).build(cx)
                });
                workspace
                    .register_panel(registration, window, cx)
                    .expect("the panel is in the catalog");
            }
        });
    })
    .unwrap();
    (handle, workspace)
}

fn withdraw(cx: &mut TestAppContext, handle: AnyWindowHandle, workspace: &Entity<Workspace>) {
    set_second_available(cx, handle, workspace, false);
}

fn set_second_available(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    available: bool,
) {
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.set_panel_available(&PANEL_B.id, available, window, cx)
        })
    })
    .unwrap();
}

/// The positions whose footer button is drawn.
fn footer_positions(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<usize> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        (0..SIDEBAR.len())
            .filter(|ix| {
                window
                    .try_find(toggle_id(Toggle::Navigation(*ix)))
                    .is_some()
            })
            .collect()
    })
    .unwrap()
}

fn open_panels(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<&'static str> {
    cx.update(|cx| {
        let workspace = workspace.read(cx);
        SIDEBAR
            .into_iter()
            .filter(|contribution| workspace.panel_is_open(&contribution.id))
            .map(|contribution| contribution.title)
            .collect()
    })
}

#[gpui_kit::test]
fn a_withdrawn_panel_leaves_the_footer_and_the_others_keep_their_positions(
    cx: &mut TestAppContext,
) {
    let (handle, workspace) = sidebar(cx);
    assert_eq!(footer_positions(cx, handle), [0, 1, 2]);
    withdraw(cx, handle, &workspace);
    assert_eq!(footer_positions(cx, handle), [0, 2]);
    cx.update_window(handle, |_, window, cx| {
        window.click(toggle_id(Toggle::Navigation(2)), cx)
    })
    .unwrap();
    assert_eq!(open_panels(cx, &workspace), [CORE_PANEL.title]);
    set_second_available(cx, handle, &workspace, true);
    assert_eq!(footer_positions(cx, handle), [0, 1, 2]);
}

#[gpui_kit::test]
fn next_panel_skips_a_withdrawn_panel(cx: &mut TestAppContext) {
    let (handle, workspace) = sidebar(cx);
    withdraw(cx, handle, &workspace);
    let next = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.on_next_panel(&NextPanel, window, cx)
            })
        })
        .unwrap();
    };
    assert_eq!(open_panels(cx, &workspace), [PANEL_A.title]);
    next(cx);
    assert_eq!(open_panels(cx, &workspace), [CORE_PANEL.title]);
    next(cx);
    assert_eq!(
        open_panels(cx, &workspace),
        [PANEL_A.title],
        "it wraps past the gap"
    );
}

#[gpui_kit::test]
fn a_withdrawn_panel_cannot_be_opened_by_id_or_position(cx: &mut TestAppContext) {
    let (handle, workspace) = sidebar(cx);
    withdraw(cx, handle, &workspace);
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_panel(&PANEL_B.id, window, cx);
            workspace.open_panel(&PANEL_B.id, window, cx);
            workspace.activate_panel(1, window, cx);
        })
    })
    .unwrap();
    assert_eq!(open_panels(cx, &workspace), [PANEL_A.title]);
    cx.update(|cx| assert!(!workspace.read(cx).panel_is_available(&PANEL_B.id)));
}
