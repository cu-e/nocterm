use super::*;
fn physical_field(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    gpui_kit::AnyWindowHandle,
    nocterm_store::SettingsHandle,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.toml");
    std::fs::create_dir(&path).unwrap();
    let (window, settings) = cx.update(|cx| {
        let fixture = init_with_settings(
            SettingsStore::new(
                Default::default(),
                nocterm_settings::SettingsFile::new(path),
            ),
            cx,
        );
        let settings = nocterm_ui::UiReady::installed(cx)
            .unwrap()
            .settings()
            .clone();
        let (window, _) = gpui_kit::open_window(test_window_options(), cx, |window, cx| {
            cx.new(|cx| {
                SettingsView::with_pages(
                    fixture.pages.clone(),
                    fixture.installed.clone(),
                    window,
                    cx,
                )
            })
        })
        .unwrap();
        (window, settings)
    });
    (directory, window, settings)
}

pub(super) fn enter_font(
    handle: gpui_kit::AnyWindowHandle,
    text: Option<&str>,
    cx: &mut TestAppContext,
) {
    cx.update_window(handle, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        select_field_page("terminal.font_size", window, cx);
        window.click("terminal.font_size", cx);
        if let Some(text) = text {
            window.press("ctrl-a", cx);
            window.press("backspace", cx);
            window.input(text, cx);
            assert_eq!(window.find("terminal.font_size").value(), Some(text));
        }
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn text_physical_failure_is_visible_and_keeps_literal(cx: &mut TestAppContext) {
    let (directory, handle, settings) = physical_field(cx);
    enter_font(handle, Some("19"), cx);
    assert!(directory.path().join("settings.toml").is_dir());
    cx.update(|cx| assert_eq!(settings.setting::<TerminalSettings>(cx).font_size, None));
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("terminal.font_size").value(), Some("19"));
        assert!(
            window
                .try_find("settings-error-terminal.font_size")
                .and_then(|error| error.label().map(str::to_owned))
                .is_some_and(|error| error.contains("Could not save settings")),
            "actual field must render physical publication failure after its receipt"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn text_physical_failure_retries_the_same_literal_after_enter(cx: &mut TestAppContext) {
    let (directory, handle, settings) = physical_field(cx);
    enter_font(handle, Some("19"), cx);
    cx.update(|cx| assert_eq!(settings.setting::<TerminalSettings>(cx).font_size, None));
    let path = directory.path().join("settings.toml");
    std::fs::remove_dir(&path).unwrap();
    enter_font(handle, None, cx);
    assert!(
        path.is_file(),
        "same literal Enter must perform a new physical publication after failure"
    );
    let mut reloaded = nocterm_settings::SettingsFile::new(path).load().unwrap();
    reloaded.register::<TerminalSettings>();
    assert_eq!(reloaded.get::<TerminalSettings>().font_size, Some(19.));
    cx.update(|cx| {
        assert_eq!(
            settings.setting::<TerminalSettings>(cx).font_size,
            Some(19.)
        )
    });
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("terminal.font_size").value(), Some("19"));
        assert!(
            window
                .try_find("settings-error-terminal.font_size")
                .is_none()
        );
    })
    .unwrap();
}
