//! Titlebar commands follow focus and act on the right window.
use super::*;

#[gpui_kit::test]
fn titlebar_find_and_settings_keep_their_field_focus(cx: &mut TestAppContext) {
    let (handle, workspace, terminal, _) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for (index, label) in ["Session", "Edit", "Search", "Window", "Help"]
            .iter()
            .enumerate()
        {
            assert_eq!(
                window
                    .within("app-menu-bar")
                    .within(index)
                    .find("menu")
                    .label(),
                Some(*label)
            );
        }
        window
            .within("app-menu-bar")
            .within(2usize)
            .click("menu", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("find-previous").is_some());
        assert!(terminal.read(cx).focus_handle(cx).is_focused(window));
        window
            .within("app-menu-bar")
            .within(0usize)
            .click("menu", cx);
        window.render_frame(cx);
        window.within("popup-menu").hover(6usize, cx);
        window.render_frame(cx);
        window
            .within("submenu")
            .within("popup-menu")
            .click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let settings = workspace
            .read(cx)
            .find_item::<nocterm_settings_ui::SettingsView>()
            .unwrap();
        assert!(settings.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn titlebar_about_and_new_window_use_application_actions(cx: &mut TestAppContext) {
    let (first, workspace, _, transport) = fixture(cx);
    cx.update_window(first, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
        window
            .within("app-menu-bar")
            .within(4usize)
            .click("menu", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(first, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("about-close").is_some());
        window.click("about-close", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(first, |_, window, cx| {
        window.render_frame(cx);
        window
            .within("app-menu-bar")
            .within(0usize)
            .click("menu", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(1usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let second = cx.update(|cx| {
        let windows = cx.windows();
        assert_eq!(windows.len(), 2);
        assert_eq!(
            transport.drivers.lock().unwrap().len(),
            1,
            "opening a window does not start another connection"
        );
        assert!(workspace.read(cx).active_session(cx).unwrap().connected);
        *windows.iter().find(|window| **window != first).unwrap()
    });
    cx.update_window(second, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
        window.dispatch_action(Box::new(nocterm_workspace::CloseWindow), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(cx.windows(), vec![first]);
        assert!(workspace.read(cx).active_session(cx).unwrap().connected);
    });
}

#[gpui_kit::test]
fn titlebar_commands_follow_bottom_screen_focus_while_find_remains_open(cx: &mut TestAppContext) {
    let (handle, workspace, central, transport) = fixture(cx);
    let bottom = cx
        .update_window(handle, |_, window, cx| {
            let local_transport = transport.clone();
            nocterm_terminal::init_local(Arc::new(move |_| local_transport.clone()), cx);
            let bottom = cx.new(|cx| TerminalView::new_local(window, cx));
            workspace.update(cx, |workspace, cx| {
                workspace.set_local_terminal(bottom.clone(), window, cx);
            });
            bottom
        })
        .unwrap();
    cx.simulate_window_resize(
        handle,
        gpui_kit::size(gpui_kit::px(1200.), gpui_kit::px(800.)),
    );
    emit(cx, &transport, 1, Event::Connected);
    let grid_focus = bottom.read_with(cx, |bottom, cx| bottom.focus_handle(cx));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(grid_focus.is_focused(window));
        window
            .within("app-menu-bar")
            .within(2usize)
            .click("menu", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("find-previous").is_some());
        assert!(bottom.read(cx).focus_handle(cx).is_focused(window));
        // TerminalElement is a canvas rather than an observed component. Send
        // native pointer events inside the bottom grid, above the footer.
        let position = gpui_kit::point(
            window.viewport_size().width - gpui_kit::px(100.),
            window.viewport_size().height - gpui_kit::px(70.),
        );
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            gpui_kit::MouseUpEvent {
                button: gpui_kit::MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert!(
            grid_focus.is_focused(window),
            "the click must actually focus the bottom terminal screen"
        );
        assert!(
            window.try_find("find-previous").is_some(),
            "the Find bar remains mounted"
        );
        window
            .within("app-menu-bar")
            .within(1usize)
            .click("menu", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(9usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("Local terminal"),
            "Copy Connection Name must target the focused bottom screen despite its open Find bar"
        );
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .within("app-menu-bar")
            .within(0usize)
            .click("menu", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(3usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        assert!(!bottom.read(cx).terminal().read(cx).is_connected());
        assert!(
            central.read(cx).terminal().read(cx).is_connected(),
            "Disconnect from the bottom screen must preserve the central remote session"
        );
    });
}

#[gpui_kit::test]
fn titlebar_close_window_retires_connected_session_and_releases_its_models(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, terminal, transport) = fixture(cx);
    let driver = transport.drivers.lock().unwrap()[0].clone();
    let workspace_weak = workspace.downgrade();
    let terminal_weak = terminal.downgrade();
    // Keep only the references a running application owns, so closing the
    // window must dispose its entities and retire its session driver.
    drop(workspace);
    drop(terminal);
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
        window
            .within("app-menu-bar")
            .within(0usize)
            .click("menu", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(10usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(cx.windows().is_empty());
        assert!(
            workspace_weak.upgrade().is_none(),
            "the closed window must release its Workspace"
        );
        assert!(
            terminal_weak.upgrade().is_none(),
            "the closed window must release its TerminalView"
        );
    });
    assert!(
        driver.closed().now_or_never().is_some(),
        "closing the window must retire the connected session"
    );
}
