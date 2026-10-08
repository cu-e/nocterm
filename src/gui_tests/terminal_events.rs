//! User keymaps and inherited Root copy shortcuts meet at the focused screen.
use super::*;

fn drain(driver: &SessionDriver) -> Vec<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(Some(command)) = driver.next_command().now_or_never() {
        if let nocterm_session::Command::Input(input) = command {
            bytes.push(input);
        }
    }
    bytes
}

#[gpui_kit::test]
fn default_ctrl_c_interrupts_in_the_application_keymap(cx: &mut TestAppContext) {
    let (handle, _, _, transport) = fixture(cx);
    emit(cx, &transport, 0, Event::Output(b"target".to_vec()));
    let driver = transport.drivers.lock().unwrap()[0].clone();
    drain(&driver);
    for selected in [false, true] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            if selected {
                window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.press("ctrl-c", cx);
        })
        .unwrap();
        assert_eq!(drain(&driver), [vec![3]], "selected={selected}");
    }
}

#[gpui_kit::test]
fn explicit_user_ctrl_c_copy_binding_overrides_the_screen_interrupt_default(
    cx: &mut TestAppContext,
) {
    let (handle, _, _, transport) = fixture(cx);
    emit(cx, &transport, 0, Event::Output(b"target".to_vec()));
    let driver = transport.drivers.lock().unwrap()[0].clone();
    drain(&driver);
    cx.update(|cx| {
        nocterm_keymap::Keymap::bind(
            "terminal::Copy",
            Some("Terminal".into()),
            None,
            "ctrl-c",
            cx,
        )
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-c", cx);
        assert!(
            cx.read_from_clipboard()
                .unwrap()
                .text()
                .unwrap()
                .contains("target")
        );
    })
    .unwrap();
    assert!(
        drain(&driver).is_empty(),
        "explicit user Copy must not interrupt"
    );
    cx.update(|cx| nocterm_keymap::Keymap::reset("terminal::Copy", cx).unwrap());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-c", cx);
    })
    .unwrap();
    assert_eq!(drain(&driver), [vec![3]], "reset restores interrupt");
}

#[gpui_kit::test]
fn explicit_workspace_ctrl_c_action_remains_reachable_from_the_terminal(cx: &mut TestAppContext) {
    let (handle, _, _, transport) = fixture(cx);
    let driver = transport.drivers.lock().unwrap()[0].clone();
    drain(&driver);
    cx.update(|cx| {
        nocterm_keymap::Keymap::bind(
            "workspace::NewTab",
            Some("Workspace".into()),
            None,
            "ctrl-c",
            cx,
        )
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| window.press("ctrl-c", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("menu-new-connection").visible());
    })
    .unwrap();
    assert!(
        drain(&driver).is_empty(),
        "user workspace action must not interrupt"
    );
}
