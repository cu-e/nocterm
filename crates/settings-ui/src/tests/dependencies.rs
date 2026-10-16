//! Native settings actions keep their explicitly supplied document authority.
use super::*;
use nocterm_store::{SettingsConfig, SettingsHandle};

fn independent_settings(cx: &mut App) -> SettingsHandle {
    SettingsConfig::in_memory(SettingsDocument::default())
        .with(nocterm_ui::AppearanceSettings::default())
        .with(TerminalSettings::default())
        .with(LocalShellSettings::default())
        .with(nocterm_session::SshSettings::default())
        .with(nocterm_session::LoggingOptions::default())
        .with(AiSettings::default())
        .install(cx)
}

fn page(
    installed: Installed,
    themes: nocterm_ui::ThemeUi,
    cx: &mut App,
) -> (gpui_kit::AnyWindowHandle, Entity<SettingsView>) {
    gpui_kit::open_window(test_window_options(), cx, |window, cx| {
        cx.new(|cx| SettingsView::with_pages(Vec::new(), themes, installed, window, cx))
    })
    .unwrap()
}

#[gpui_kit::test]
fn shared_pages_keep_original_settings_after_another_owner_and_window_close(
    cx: &mut TestAppContext,
) {
    let (installed, themes, other, first, first_view, second, second_view) = cx.update(|cx| {
        init(cx);
        let themes = theme_ui(cx);
        let installed = settings_deps(cx);
        let other = independent_settings(cx);
        let (first, a) = page(installed.clone(), themes.clone(), cx);
        let (second, b) = page(installed.clone(), themes.clone(), cx);
        cx.set_global(SettingsStore::from_handle(other.clone()));
        (installed, themes, other, first, a, second, b)
    });
    cx.update_window(first, |_, window, cx| {
        select_field_page("terminal.font_size", window, cx);
        window.click("copy-select", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            installed
                .settings()
                .setting::<TerminalSettings>(cx)
                .copy_on_select
        );
        assert!(!other.setting::<TerminalSettings>(cx).copy_on_select);
    });
    let weak = first_view.downgrade();
    cx.update_window(first, |_, window, _| window.remove_window())
        .unwrap();
    drop(first_view);
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
    type_and_leave(cx, second, &second_view, "terminal.font_size", "19");
    cx.run_until_parked();
    let (third, _third_view) = cx.update(|cx| page(installed.clone(), themes, cx));
    cx.update_window(third, |_, window, cx| {
        select_field_page("terminal.font_size", window, cx);
        assert_eq!(window.find("terminal.font_size").value(), Some("19"));
    })
    .unwrap();
    cx.update(|cx| {
        assert_eq!(
            installed
                .settings()
                .setting::<TerminalSettings>(cx)
                .font_size,
            Some(19.)
        );
        assert_eq!(other.setting::<TerminalSettings>(cx).font_size, None);
    });
}

#[gpui_kit::test]
fn native_ai_policy_and_text_edits_keep_the_supplied_handle(cx: &mut TestAppContext) {
    let (installed, other, handle, view) = cx.update(|cx| {
        init(cx);
        let installed = settings_deps(cx);
        let (handle, view) = page(installed.clone(), theme_ui(cx), cx);
        let other = independent_settings(cx);
        cx.set_global(SettingsStore::from_handle(other.clone()));
        (installed, other, handle, view)
    });
    cx.update_window(handle, |_, window, _| {
        window.resize(gpui_kit::size(gpui_kit::px(1100.), gpui_kit::px(5000.)));
    })
    .unwrap();
    type_and_leave(
        cx,
        handle,
        &view,
        "ai.working_directory",
        "/original/policy",
    );
    cx.update_window(handle, |_, window, cx| window.click("ai-enabled", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let actual = installed.settings().setting::<AiSettings>(cx);
        assert_eq!(
            actual.working_directory.as_deref(),
            Some("/original/policy")
        );
        assert!(!actual.enabled);
        let untouched = other.setting::<AiSettings>(cx);
        assert!(untouched.working_directory.is_none());
        assert!(untouched.enabled);
    });
}

#[gpui_kit::test]
fn deferred_appearance_command_keeps_original_settings_without_global_redirect(
    cx: &mut TestAppContext,
) {
    let (installed, other, handle, workspace) = cx.update(|cx| {
        init(cx);
        let installed = settings_deps(cx);
        let themes = theme_ui(cx);
        let layout = nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx));
        let supplied = installed.clone();
        let (handle, workspace) = gpui_kit::open_window(test_window_options(), cx, |window, cx| {
            cx.new(|cx| {
                let mut workspace = Workspace::new(layout, window, cx);
                register(&mut workspace, themes, supplied);
                workspace
            })
        })
        .unwrap();
        let other = independent_settings(cx);
        cx.set_global(SettingsStore::from_handle(other.clone()));
        (installed, other, handle, workspace)
    });
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        window.focus(&workspace.read(cx).focus_handle(cx), cx);
        window.dispatch_action(Box::new(nocterm_workspace::ChangeColorScheme), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.input("Dark", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            installed
                .settings()
                .setting::<nocterm_ui::AppearanceSettings>(cx)
                .mode,
            nocterm_ui::AppearanceMode::Dark
        );
        assert_eq!(
            other.setting::<nocterm_ui::AppearanceSettings>(cx).mode,
            nocterm_ui::AppearanceMode::System
        );
    });
}
