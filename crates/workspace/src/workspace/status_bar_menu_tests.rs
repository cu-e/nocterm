//! The context menu of the status bar hides the item under the pointer, opens the layout editor
//! on it, and lists the layouts.
use std::{cell::RefCell, rc::Rc};

use gpui_kit::{AppContext as _, TestAppContext, test::TestWindowExt as _};
use nocterm_settings::SettingsDocument;
use nocterm_window_layout::LayoutSettings;

use super::layout_switch_tests::{Windows, focus_workspace, register_all, windows_over};
use crate::{
    OpenLayoutSettings,
    test_support::{Toggle, toggle_id},
};

/// A window on the Default layout with the two sidebar panels registered.
fn default_window(cx: &mut TestAppContext) -> Windows {
    let windows = windows_over(cx, SettingsDocument::default(), None, false);
    register_all(cx, windows.handle, &windows.workspace);
    cx.run_until_parked();
    windows
}

fn right_click_first_toggle(windows: &Windows, cx: &mut TestAppContext) {
    cx.update_window(windows.handle, |_, window, cx| {
        focus_workspace(&windows.workspace, window, cx);
        window.render_frame(cx);
        window.right_click(toggle_id(Toggle::Navigation(0)), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(windows.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

#[gpui_kit::test]
fn hiding_an_item_from_its_menu_moves_it_to_the_tray_of_a_custom_layout(cx: &mut TestAppContext) {
    let windows = default_window(cx);
    right_click_first_toggle(&windows, cx);
    cx.update_window(windows.handle, |_, window, cx| {
        assert!(window.try_find("popup-menu").is_some());
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();

    let stored = cx.update(|cx| windows.settings.setting::<LayoutSettings>(cx).clone());
    assert_eq!(
        stored.presets.len(),
        1,
        "the first edit of Default saves a copy"
    );
    assert_eq!(stored.presets[0].tray.len(), 1);
    assert_eq!(stored.presets[0].tray[0].as_str(), "test.panel-a");
}

#[gpui_kit::test]
fn customizing_names_the_item_under_the_pointer(cx: &mut TestAppContext) {
    let windows = default_window(cx);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    cx.update(|cx| {
        cx.on_action(move |action: &OpenLayoutSettings, _| {
            sink.borrow_mut()
                .push(action.select.as_ref().map(ToString::to_string));
        });
    });
    right_click_first_toggle(&windows, cx);
    cx.update_window(windows.handle, |_, window, cx| {
        // Hide, a separator, then Customize.
        window.within("popup-menu").click(2usize, cx);
    })
    .unwrap();
    cx.run_until_parked();

    assert_eq!(*seen.borrow(), [Some("test.panel-a".to_owned())]);
}

#[gpui_kit::test]
fn the_menu_of_empty_space_has_no_item_to_hide(cx: &mut TestAppContext) {
    let windows = default_window(cx);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    cx.update(|cx| {
        cx.on_action(move |action: &OpenLayoutSettings, _| {
            sink.borrow_mut()
                .push(action.select.as_ref().map(ToString::to_string));
        });
    });
    cx.update_window(windows.handle, |_, window, cx| {
        focus_workspace(&windows.workspace, window, cx);
        window.render_frame(cx);
        window.right_click("workspace-footer", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(windows.handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();

    assert_eq!(*seen.borrow(), [None]);
}

#[gpui_kit::test]
fn a_pinned_item_has_no_hide_in_its_menu(cx: &mut TestAppContext) {
    let windows = default_window(cx);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    cx.update(|cx| {
        cx.on_action(move |action: &OpenLayoutSettings, _| {
            sink.borrow_mut()
                .push(action.select.as_ref().map(ToString::to_string));
        });
    });
    cx.update_window(windows.handle, |_, window, cx| {
        focus_workspace(&windows.workspace, window, cx);
        nocterm_ui::notice::error(window, cx, "test", "Something happened", "details");
        window.render_frame(cx);
        window.right_click("notice-bar", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(windows.handle, |_, window, cx| {
        window.render_frame(cx);
        // The first item is Customize: there is nothing to hide.
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();

    assert_eq!(*seen.borrow(), [Some("workspace.notifications".to_owned())]);
    let stored = cx.update(|cx| windows.settings.setting::<LayoutSettings>(cx).clone());
    assert!(stored.presets.is_empty());
}
