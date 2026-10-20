//! Secret prompts keep tab navigation and return focus after an answer.
use super::*;

#[gpui_kit::test]
fn secret_prompt_keeps_tab_navigation(cx: &mut TestAppContext) {
    let (handle, _, terminal, transport) = fixture(cx);
    let (reply, _answer) = Reply::<Option<Secret>>::channel();
    emit(
        cx,
        &transport,
        0,
        Event::Prompt(Prompt::Secret {
            request: SecretRequest::Password {
                target: spec("first.test").target,
                retry: false,
            },
            reply,
        }),
    );
    let driver = transport.drivers.lock().unwrap()[0].clone();
    while driver.next_command().now_or_never().flatten().is_some() {}
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let input_focus = terminal.read(cx).focus_handle(cx);
        assert!(input_focus.is_focused(window));
        window.press("tab", cx);
        window.render_frame(cx);
        assert!(!input_focus.is_focused(window));
        window.press("shift-tab", cx);
        window.render_frame(cx);
        assert!(input_focus.is_focused(window));
    })
    .unwrap();
    while let Some(Some(command)) = driver.next_command().now_or_never() {
        assert!(!matches!(command, nocterm_session::Command::Input(_)));
    }
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn delayed_secret_prompt_preserves_other_tab_focus_and_restores_input_on_activation(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, first, transport) = fixture(cx);
    let second = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.open_session(spec("second.test"), window, cx)
            });
            workspace
                .read(cx)
                .active_item()
                .unwrap()
                .view()
                .downcast::<TerminalView>()
                .unwrap()
        })
        .unwrap();
    emit(cx, &transport, 1, Event::Connected);
    let (reply, mut answer) = Reply::<Option<Secret>>::channel();
    emit(
        cx,
        &transport,
        0,
        Event::Prompt(Prompt::Secret {
            request: SecretRequest::Interactive {
                prompt: "Code:".into(),
                echo: true,
            },
            reply,
        }),
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            second.read(cx).focus_handle(cx).is_focused(window),
            "an inactive tab must not steal focus for its prompt"
        );
        workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(first.entity_id(), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(first.read(cx).focus_handle(cx).is_focused(window));
        window.input("123456", cx);
        window.click("secret-submit", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        answer.try_recv().unwrap().unwrap().unwrap().expose(),
        "123456",
        "activation must focus the secret input instead of sending text to the terminal"
    );

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(second.entity_id(), window, cx)
        });
    })
    .unwrap();
    let (reply, _answer) = Reply::<Option<Secret>>::channel();
    emit(
        cx,
        &transport,
        0,
        Event::Prompt(Prompt::Secret {
            request: SecretRequest::Interactive {
                prompt: "Again:".into(),
                echo: true,
            },
            reply,
        }),
    );
    emit(
        cx,
        &transport,
        0,
        Event::Closed(CloseReason::Exited(Some(0))),
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            second.read(cx).focus_handle(cx).is_focused(window),
            "removing an inactive prompt must not steal focus"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn active_secret_prompt_accepts_text_and_releases_focus_after_submit(cx: &mut TestAppContext) {
    let (handle, _, terminal, transport) = fixture(cx);
    let grid_focus = terminal.read_with(cx, |terminal, cx| terminal.focus_handle(cx));
    let (reply, mut answer) = Reply::<Option<Secret>>::channel();
    emit(
        cx,
        &transport,
        0,
        Event::Prompt(Prompt::Secret {
            request: SecretRequest::Interactive {
                prompt: "Code:".into(),
                echo: true,
            },
            reply,
        }),
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let prompt_focus = terminal.read(cx).focus_handle(cx);
        assert_ne!(prompt_focus, grid_focus);
        assert!(prompt_focus.is_focused(window));
        window.input("654321", cx);
        window.click("secret-submit", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        answer.try_recv().unwrap().unwrap().unwrap().expose(),
        "654321"
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(grid_focus.is_focused(window));
    })
    .unwrap();
}
