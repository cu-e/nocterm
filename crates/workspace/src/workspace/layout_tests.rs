//! The body's columns: resizing, hiding a sidebar panel from its own
//! button, and swapping sides.
use crate::test_support::{
    EXPANDABLE, PANEL_A, PANEL_B, TestPanel, TestPanelBuilder, Toggle, toggle_id,
};
use gpui_kit::{
    App, AppContext as _, Entity, Focusable, TestAppContext, Window, px, test::TestWindowExt as _,
};
use nocterm_ui::IconName;

use super::{Arrangement, Column};
use crate::workspace::panel_groups::GroupKey;
use crate::workspace::tests::test_plugin::{fixture_with_test_plugin, register_built};
use nocterm_plugin_model::Side;

fn side_panel(element_id: &'static str, title: &'static str, icon: IconName) -> TestPanelBuilder {
    TestPanel::builder(element_id).title(title).icon(icon)
}

fn group_column(group: GroupKey, side: Side) -> Column {
    Column::Group {
        group,
        class: group.width_class(),
        side,
    }
}

#[test]
fn hidden_columns_leave_the_group_and_swapping_mirrors_it() {
    let arrangement = |navigation: bool, assistant: bool, swapped: bool| {
        let side = |side: Side| side.swapped_if(swapped);
        let shown = [
            navigation.then_some((GroupKey::Navigation, side(Side::Left))),
            assistant.then_some((GroupKey::Assistant, side(Side::Right))),
        ];
        Arrangement::new(shown.into_iter().flatten(), None)
    };
    let navigation = |side| group_column(GroupKey::Navigation, side);
    let assistant = |side| group_column(GroupKey::Assistant, side);
    assert_eq!(
        arrangement(true, false, false).columns(),
        [navigation(Side::Left), Column::Content]
    );
    assert_eq!(
        arrangement(true, true, true).columns(),
        [
            assistant(Side::Left),
            Column::Content,
            navigation(Side::Right)
        ]
    );
    for swapped in [false, true] {
        let side = |side: Side| side.swapped_if(swapped);
        let shown = |navigation: bool| {
            let navigation = navigation.then_some((GroupKey::Navigation, side(Side::Left)));
            let assistant = Some((GroupKey::Assistant, side(Side::Right)));
            Arrangement::new(
                navigation.into_iter().chain(assistant),
                Some(GroupKey::Assistant),
            )
        };
        let expected = if swapped {
            [assistant(Side::Left), navigation(Side::Right)]
        } else {
            [navigation(Side::Left), assistant(Side::Right)]
        };
        assert_eq!(shown(true).columns(), expected);
        assert_eq!(shown(false).columns(), [assistant(side(Side::Right))]);
    }
}

#[test]
fn groups_on_one_side_run_from_the_window_edge_inward() {
    let shown = [
        (GroupKey::Navigation, Side::Right),
        (GroupKey::Assistant, Side::Right),
    ];
    assert_eq!(
        Arrangement::new(shown, None).columns(),
        [
            Column::Content,
            group_column(GroupKey::Assistant, Side::Right),
            group_column(GroupKey::Navigation, Side::Right),
        ]
    );
    let shown = [
        (GroupKey::Navigation, Side::Left),
        (GroupKey::Assistant, Side::Left),
    ];
    assert_eq!(
        Arrangement::new(shown, None).columns(),
        [
            group_column(GroupKey::Navigation, Side::Left),
            group_column(GroupKey::Assistant, Side::Left),
            Column::Content,
        ]
    );
}

#[gpui_kit::test]
fn sidebar_resizes_with_the_side_panel_closed_and_its_button_hides_it(cx: &mut TestAppContext) {
    let (handle, workspace, _) = fixture_with_test_plugin(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1400.), px(700.)));
        let side = side_panel("side-probe", "Servers", IconName::Server).build(cx);
        let right = TestPanel::builder("right-probe")
            .title("AI Agents")
            .build(cx);
        workspace.update(cx, |workspace, cx| {
            register_built(workspace, PANEL_A, &side, window, cx);
            register_built(workspace, EXPANDABLE, &right, window, cx);
        });
        window.render_frame(cx);
        assert!(!workspace.read(cx).panel_is_open(&EXPANDABLE.id));
        let before = window.find("side-probe").bounds().size.width;
        let bounds = window.find("side-probe").bounds();
        let inset = gpui_kit::component::floating::FloatingCards::get(cx)
            .map_or(px(0.), |cards| cards.margin() + px(1.));
        let from = gpui_kit::point(bounds.right() + inset, bounds.center().y);
        window.drag(from, gpui_kit::point(from.x + px(150.), from.y), cx);
        window.render_frame(cx);
        let after = window.find("side-probe").bounds().size.width;
        assert!(after > before + px(100.), "{before:?} → {after:?}");

        // The shown panel's own button hides the sidebar, and shows it again.
        window.click(toggle_id(Toggle::Navigation(0)), cx);
        window.render_frame(cx);
        assert!(window.try_find("side-probe").is_none());
        window.click(toggle_id(Toggle::Navigation(0)), cx);
        window.render_frame(cx);
        assert!(window.try_find("side-probe").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn swapping_puts_the_side_panel_on_the_left(cx: &mut TestAppContext) {
    let (handle, workspace, _) = fixture_with_test_plugin(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1400.), px(700.)));
        let side = side_panel("side-probe", "Servers", IconName::Server).build(cx);
        let right = TestPanel::builder("right-probe")
            .title("AI Agents")
            .build(cx);
        workspace.update(cx, |workspace, cx| {
            register_built(workspace, PANEL_A, &side, window, cx);
            register_built(workspace, EXPANDABLE, &right, window, cx);
            workspace.toggle_panel(&EXPANDABLE.id, window, cx);
        });
        window.render_frame(cx);
        assert!(
            window.find("side-probe").bounds().left() < window.find("right-probe").bounds().left()
        );
        window.dispatch_action(Box::new(crate::SwapSides), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.find("right-probe").bounds().left() < window.find("side-probe").bounds().left()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn hidden_and_reordered_columns_keep_each_owners_visible_width(cx: &mut TestAppContext) {
    let (handle, workspace, layout) = fixture_with_test_plugin(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1400.), px(700.)));
        layout.set("workspace.sidebar", px(260.), cx).unwrap();
        layout.set("workspace.right_panel", px(420.), cx).unwrap();
        let side = side_panel("side-probe", "Servers", IconName::Server).build(cx);
        let right = TestPanel::builder("right-probe")
            .title("AI Agents")
            .build(cx);
        workspace.update(cx, |workspace, cx| {
            register_built(workspace, PANEL_A, &side, window, cx);
            register_built(workspace, EXPANDABLE, &right, window, cx);
            workspace.toggle_panel(&EXPANDABLE.id, window, cx);
        });
        window.render_frame(cx);
        let sidebar_width = window.find("side-probe").bounds().size.width;
        let right_width = window.find("right-probe").bounds().size.width;
        workspace.update(cx, |workspace, cx| workspace.toggle_sidebar(window, cx));
        window.render_frame(cx);
        assert!(window.try_find("side-probe").is_none());
        assert_eq!(window.find("right-probe").bounds().size.width, right_width);
        workspace.update(cx, |workspace, cx| {
            workspace.swap_sides(cx);
            workspace.toggle_sidebar(window, cx);
        });
        window.render_frame(cx);
        assert!(
            window.find("right-probe").bounds().left() < window.find("side-probe").bounds().left()
        );
        assert_eq!(window.find("side-probe").bounds().size.width, sidebar_width);
        assert_eq!(window.find("right-probe").bounds().size.width, right_width);
        assert_eq!(layout.get("workspace.sidebar", cx), Some(px(260.)));
        assert_eq!(layout.get("workspace.right_panel", cx), Some(px(420.)));
    })
    .unwrap();
}

#[gpui_kit::test]
fn maximized_panel_does_not_persist_transient_window_or_widen_sizes(cx: &mut TestAppContext) {
    let (handle, workspace, layout) = fixture_with_test_plugin(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1400.), px(700.)));
        layout.set("workspace.sidebar", px(260.), cx).unwrap();
        layout.set("workspace.right_panel", px(420.), cx).unwrap();
        let (_, right) = install_columns(&workspace, window, cx);
        window.render_frame(cx);
        let normal = window.find("right-probe").bounds().size.width;
        let sidebar = window.find("side-probe").bounds().size.width;
        workspace.update(cx, |workspace, cx| {
            workspace.set_panel_expanded(&EXPANDABLE.id, true, cx)
        });
        window.render_frame(cx);
        assert!(window.find("right-probe").bounds().size.width > normal);
        let bounds = window.find("side-probe").bounds();
        let inset = gpui_kit::component::floating::FloatingCards::get(cx)
            .map_or(px(0.), |cards| cards.margin() + px(1.));
        let from = gpui_kit::point(bounds.right() + inset, bounds.center().y);
        window.drag(from, gpui_kit::point(from.x + px(80.), from.y), cx);
        right.update(cx, |_, cx| cx.emit(crate::PanelRequest::Widen(px(200.))));
        window.resize(gpui_kit::size(px(1500.), px(700.)));
        window.render_frame(cx);
        assert_eq!(layout.get("workspace.sidebar", cx), Some(px(260.)));
        assert_eq!(layout.get("workspace.right_panel", cx), Some(px(420.)));
        workspace.update(cx, |workspace, cx| {
            workspace.set_panel_expanded(&EXPANDABLE.id, false, cx)
        });
        window.render_frame(cx);
        assert_eq!(window.find("right-probe").bounds().size.width, normal);
        assert_eq!(window.find("side-probe").bounds().size.width, sidebar);
        assert_eq!(layout.get("workspace.sidebar", cx), Some(px(260.)));
        assert_eq!(layout.get("workspace.right_panel", cx), Some(px(420.)));
    })
    .unwrap();
}

fn install_columns(
    workspace: &Entity<crate::Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> (Entity<TestPanel>, Entity<TestPanel>) {
    let side = side_panel("side-probe", "Servers", IconName::Server).build(cx);
    let right = TestPanel::builder("right-probe")
        .title("AI Agents")
        .build(cx);
    workspace.update(cx, |workspace, cx| {
        register_built(workspace, PANEL_A, &side, window, cx);
        register_built(workspace, EXPANDABLE, &right, window, cx);
        workspace.toggle_panel(&EXPANDABLE.id, window, cx);
    });
    (side, right)
}

fn assert_expanded_columns(window: &mut Window, cx: &mut App, swapped: bool) {
    window.render_frame(cx);
    window.render_frame(cx);
    let side = window.find("side-probe").bounds();
    let right = window.find("right-probe").bounds();
    assert!(window.try_find("empty-new-tab").is_none());
    assert!(window.try_find(toggle_id(Toggle::SidePanel)).is_some());
    assert!(side.size.width > px(100.) && right.size.width > px(0.));
    if swapped {
        assert!(right.right() <= side.left());
    } else {
        assert!(side.right() <= right.left());
    }
    assert!(side.left() >= px(0.) && right.left() >= px(0.));
    assert!(side.right() <= window.viewport_size().width);
    assert!(right.right() <= window.viewport_size().width);
}

#[gpui_kit::test]
fn expanded_chat_keeps_navigation_beside_it_at_supported_window_widths(cx: &mut TestAppContext) {
    let (handle, workspace, layout) = fixture_with_test_plugin(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1400.), px(700.)));
        layout.set("workspace.sidebar", px(260.), cx).unwrap();
        let _ = install_columns(&workspace, window, cx);
        window.render_frame(cx);
        let sidebar_width = window.find("side-probe").bounds().size.width;
        let docked_width = window.find("right-probe").bounds().size.width;
        workspace.update(cx, |workspace, cx| {
            workspace.set_panel_expanded(&EXPANDABLE.id, true, cx)
        });
        assert_expanded_columns(window, cx, false);
        assert!(window.find("right-probe").bounds().size.width > docked_width);
        for swapped in [false, true] {
            if swapped {
                workspace.update(cx, |workspace, cx| workspace.swap_sides(cx));
            }
            for width in [600., 1000., 1400.] {
                window.resize(gpui_kit::size(px(width), px(700.)));
                assert_expanded_columns(window, cx, swapped);
                assert_eq!(window.find("side-probe").bounds().size.width, sidebar_width);
            }
        }
        workspace.update(cx, |workspace, cx| {
            workspace.set_panel_available(&EXPANDABLE.id, false, window, cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("empty-new-tab").is_some());
        assert!(window.try_find("side-probe").is_some());
        assert!(window.try_find("right-probe").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn chat_expand_restore_and_close_events_preserve_visible_navigation(cx: &mut TestAppContext) {
    let (handle, workspace, _) = fixture_with_test_plugin(cx);
    cx.run_until_parked();
    let (right, sidebar_width, docked_width) = cx
        .update_window(handle, |_, window, cx| {
            window.resize(gpui_kit::size(px(1400.), px(700.)));
            let (_, right) = install_columns(&workspace, window, cx);
            window.render_frame(cx);
            window.render_frame(cx);
            (
                right,
                window.find("side-probe").bounds().size.width,
                window.find("right-probe").bounds().size.width,
            )
        })
        .unwrap();
    use crate::PanelRequest::{Close, ToggleExpanded as ToggleMaximized};
    for (event, expanded, open) in [
        (ToggleMaximized, true, true),
        (ToggleMaximized, false, true),
        (ToggleMaximized, true, true),
        (Close, false, false),
    ] {
        cx.update(|cx| right.update(cx, |_, cx| cx.emit(event)));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("side-probe").bounds().size.width,
                sidebar_width,
                "after {event:?}, expanded={expanded}, open={open}"
            );
            assert_eq!(window.try_find("empty-new-tab").is_none(), expanded);
            assert_eq!(window.try_find("right-probe").is_some(), open);
            if expanded {
                assert_expanded_columns(window, cx, false);
                assert!(window.find("right-probe").bounds().size.width > docked_width);
            } else if open {
                assert_eq!(window.find("right-probe").bounds().size.width, docked_width);
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn sidebar_controls_preserve_expansion_and_focus_visible_chat_when_closed(cx: &mut TestAppContext) {
    let (handle, workspace, _) = fixture_with_test_plugin(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1400.), px(700.)));
        let (_, right) = install_columns(&workspace, window, cx);
        let other = side_panel("side-probe", "Servers", IconName::Server).build(cx);
        workspace.update(cx, |workspace, cx| {
            register_built(workspace, PANEL_B, &other, window, cx);
            workspace.set_panel_expanded(&EXPANDABLE.id, true, cx);
        });
        assert_expanded_columns(window, cx, false);
        window.click(toggle_id(Toggle::Navigation(1)), cx);
        assert_expanded_columns(window, cx, false);
        assert!(other.focus_handle(cx).is_focused(window));
        window.click(toggle_id(Toggle::Navigation(1)), cx);
        window.render_frame(cx);
        assert!(window.try_find("side-probe").is_none());
        assert!(window.try_find("empty-new-tab").is_none());
        assert!(right.focus_handle(cx).is_focused(window));
        let full_width = window.find("right-probe").bounds().size.width;
        workspace.update(cx, |workspace, cx| workspace.toggle_sidebar(window, cx));
        assert_expanded_columns(window, cx, false);
        assert!(window.find("right-probe").bounds().size.width < full_width);
        workspace.update(cx, |workspace, cx| {
            workspace.swap_sides(cx);
            workspace.toggle_sidebar(window, cx);
        });
        window.render_frame(cx);
        assert!(window.try_find("empty-new-tab").is_none());
        window.click(toggle_id(Toggle::Navigation(0)), cx);
        assert_expanded_columns(window, cx, true);
    })
    .unwrap();
}

#[gpui_kit::test]
fn expanded_resize_receipts_cannot_overwrite_remembered_docked_widths(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let layout = nocterm_ui::LayoutHandle::in_memory(cx);
        layout.set("workspace.sidebar", px(260.), cx).unwrap();
        layout.set("workspace.right_panel", px(420.), cx).unwrap();
        let body = super::Body::new(layout.clone());
        for swapped in [false, true] {
            let side = |side: Side| side.swapped_if(swapped);
            let expanded = Arrangement::new(
                [
                    (GroupKey::Navigation, side(Side::Left)),
                    (GroupKey::Assistant, side(Side::Right)),
                ],
                Some(GroupKey::Assistant),
            );
            body.resized(&expanded, vec![px(350.), px(1050.)], cx);
        }
        assert_eq!(layout.get("workspace.sidebar", cx), Some(px(260.)));
        assert_eq!(layout.get("workspace.right_panel", cx), Some(px(420.)));
    });
}

#[gpui_kit::test]
fn a_docked_receipt_arriving_after_the_side_panel_expanded_is_dropped(cx: &mut TestAppContext) {
    let (handle, workspace, layout) = fixture_with_test_plugin(cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(px(1400.), px(700.)));
        install_columns(&workspace, window, cx);
        layout.set("workspace.sidebar", px(260.), cx).unwrap();
        layout.set("workspace.right_panel", px(420.), cx).unwrap();
        let docked = Arrangement::new(
            [
                (GroupKey::Navigation, Side::Left),
                (GroupKey::Assistant, Side::Right),
            ],
            None,
        );
        let receipt = |workspace: &mut crate::Workspace, cx: &mut gpui_kit::Context<_>| {
            workspace.body_resized(&docked, vec![px(300.), px(700.), px(400.)], cx)
        };
        workspace.update(cx, |workspace, cx| {
            workspace.set_panel_expanded(&EXPANDABLE.id, true, cx);
            receipt(workspace, cx);
        });
        assert_eq!(layout.get("workspace.sidebar", cx), Some(px(260.)));
        assert_eq!(layout.get("workspace.right_panel", cx), Some(px(420.)));
        workspace.update(cx, |workspace, cx| {
            workspace.set_panel_expanded(&EXPANDABLE.id, false, cx);
            receipt(workspace, cx);
        });
        assert_eq!(layout.get("workspace.sidebar", cx), Some(px(300.)));
        assert_eq!(layout.get("workspace.right_panel", cx), Some(px(400.)));
    })
    .unwrap();
}
