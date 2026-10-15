//! Real bootstrap compositions register agent policy before opening any view.
use super::*;

#[gpui_kit::test]
fn configured_and_malformed_ai_policy_are_registered_before_first_window(cx: &mut TestAppContext) {
    for (text, enabled, malformed) in [
        ("[ai]\nenabled = false\n", false, false),
        ("[ai]\nenabled = 'invalid'\n", true, true),
    ] {
        cx.update(|cx| {
            let document =
                nocterm_settings::SettingsDocument::from_table(toml::from_str(text).unwrap());
            let booted = boot(
                tempfile::tempdir().unwrap(),
                document,
                Arc::new(MockTransport::default()),
                false,
                cx,
            );
            let settings = nocterm_ui::UiReady::installed(cx)
                .unwrap()
                .settings()
                .clone();
            assert!(
                settings
                    .document(cx)
                    .is_registered::<nocterm_ai::AiSettings>()
            );
            assert_eq!(settings.section_errors(cx).is_empty(), !malformed);
            let (_, workspace) = open_workspace(&booted, cx);
            assert_eq!(workspace.read(cx).right_panel_is_available(), enabled);
        });
    }
}

#[gpui_kit::test]
fn surviving_and_new_windows_keep_original_agent_installation_after_first_closes(
    cx: &mut TestAppContext,
) {
    let (booted, first, first_workspace, second, second_workspace) = cx.update(|cx| {
        let mut settings = nocterm_settings::SettingsDocument::default();
        settings.update::<nocterm_ai::AiSettings>(|ai| ai.sessions.warm_start = false);
        let booted = boot(
            tempfile::tempdir().unwrap(),
            settings,
            Arc::new(MockTransport::default()),
            false,
            cx,
        );
        let (first, first_workspace) = open_workspace(&booted, cx);
        let (second, second_workspace) = open_workspace(&booted, cx);
        booted
            .agent_installation()
            .runtime()
            .set_last_agent("codex", cx);
        (booted, first, first_workspace, second, second_workspace)
    });
    let weak = first_workspace.downgrade();
    cx.update_window(first, |_, window, _| window.remove_window())
        .unwrap();
    drop(first_workspace);
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
    let before = cx.update(|cx| cx.windows());
    cx.update_window(second, |_, window, cx| {
        window.dispatch_action(Box::new(nocterm_workspace::NewWindow), cx)
    })
    .unwrap();
    cx.run_until_parked();
    let third = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|window| {
                !before
                    .iter()
                    .any(|old| old.window_id() == window.window_id())
            })
            .unwrap()
    });
    cx.update_window(third, |_, window, cx| {
        let workspace = gpui_kit::base::Root::read(window, cx)
            .view()
            .clone()
            .downcast::<Workspace>()
            .unwrap();
        assert!(workspace.read(cx).right_panel_is_available());
        assert!(second_workspace.read(cx).right_panel_is_available());
        assert_eq!(
            booted
                .agent_installation()
                .runtime()
                .favorites(cx)
                .last_agent
                .as_deref(),
            Some("codex")
        );
    })
    .unwrap();
    cx.update_window(second, |_, window, cx| {
        window.activate_window();
        window.focus(&second_workspace.read(cx).focus_handle(cx), cx);
        window.dispatch_action(Box::new(nocterm_agent::NewThreadWithLastAgent), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            booted
                .agent_installation()
                .runtime()
                .favorites(cx)
                .last_agent
                .as_deref(),
            Some("codex")
        )
    });
}
