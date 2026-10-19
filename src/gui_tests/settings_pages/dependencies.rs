//! Native settings actions keep their explicitly supplied document authority.
use super::*;
use nocterm_store::{SettingsConfig, SettingsHandle};

fn independent_settings(cx: &mut App) -> SettingsHandle {
    let fixture = init_with_settings(SettingsConfig::in_memory(SettingsDocument::default()), cx);
    let settings = fixture.installed.settings().clone();
    page(fixture.installed, fixture.pages, cx);
    settings
}

fn page(
    installed: Installed,
    pages: SettingsCatalog,
    cx: &mut App,
) -> (gpui_kit::AnyWindowHandle, Entity<SettingsView>) {
    gpui_kit::open_window(test_window_options(), cx, |window, cx| {
        cx.new(|cx| SettingsView::with_pages(pages, installed, window, cx))
    })
    .unwrap()
}

#[gpui_kit::test]
fn shared_pages_keep_original_settings_after_another_owner_and_window_close(
    cx: &mut TestAppContext,
) {
    let (installed, themes, other, first, first_view, second, second_view) = cx.update(|cx| {
        let fixture = init(cx);
        let themes = fixture.pages.clone();
        let installed = fixture.installed.clone();
        let other = independent_settings(cx);
        let (first, a) = page(installed.clone(), themes.clone(), cx);
        let (second, b) = page(installed.clone(), themes.clone(), cx);
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
        let fixture = init(cx);
        let installed = fixture.installed.clone();
        let (handle, view) = page(installed.clone(), fixture.pages.clone(), cx);
        let other = independent_settings(cx);
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
        let fixture = init(cx);
        let installed = fixture.installed.clone();
        let themes = fixture.pages.clone();
        let layout = nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx));
        let supplied = installed.clone();
        let (handle, workspace) = gpui_kit::open_window(test_window_options(), cx, |window, cx| {
            cx.new(|cx| {
                let mut workspace = Workspace::new(layout, window, cx);
                crate::settings_pages::register_appearance_commands(
                    &mut workspace,
                    fixture.themes.clone(),
                    supplied.settings().clone(),
                );
                register(&mut workspace, themes, supplied);
                workspace
            })
        })
        .unwrap();
        let other = independent_settings(cx);
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

struct GatedPage {
    window: gpui_kit::AnyWindowHandle,
    workspace: Entity<Workspace>,
    settings: nocterm_store::SettingsHandle,
    release: nocterm_store::test_support::WriteRelease,
    writes: nocterm_store::test_support::WriteLog<SettingsDocument>,
}

fn gated_page(cx: &mut TestAppContext) -> GatedPage {
    let (writer, release, writes) = nocterm_store::test_support::gated_writer().into_parts();
    let (window, workspace, settings) = cx.update(|cx| {
        let fixture =
            init_with_settings(SettingsConfig::with_writer(Default::default(), writer), cx);
        let settings = fixture.installed.settings().clone();
        let layout = nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx));
        let (window, workspace) = gpui_kit::open_window(test_window_options(), cx, |window, cx| {
            let workspace = cx.new(|cx| {
                let mut workspace = Workspace::new(layout, window, cx);
                register(
                    &mut workspace,
                    fixture.pages.clone(),
                    fixture.installed.clone(),
                );
                workspace
            });
            window.activate_window();
            window.focus(&workspace.read(cx).focus_handle(cx), cx);
            workspace
        })
        .unwrap();
        (window, workspace, settings)
    });
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.dispatch_action(Box::new(OpenSettings), cx)
    })
    .unwrap();
    cx.run_until_parked();
    GatedPage {
        window,
        workspace,
        settings,
        release,
        writes,
    }
}

fn font_draft(handle: gpui_kit::AnyWindowHandle, text: &str, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        select_field_page("terminal.font_size", window, cx);
        window.click("terminal.font_size", cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.input(text, cx);
        assert_eq!(window.find("terminal.font_size").value(), Some(text));
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
async fn closing_the_actual_settings_item_admits_latest_text_before_capture(
    cx: &mut TestAppContext,
) {
    let f = gated_page(cx);
    enter_font(f.window, Some("19"), cx);
    font_draft(f.window, "20", cx);
    let weak = cx.update(|cx| {
        f.workspace
            .read(cx)
            .find_item::<SettingsView>()
            .unwrap()
            .downgrade()
    });
    cx.update_window(f.window, |_, window, cx| {
        window.dispatch_action(Box::new(nocterm_workspace::CloseTab), cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(f.workspace.read(cx).items().count(), 0);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
    let work = cx.update(|cx| f.settings.capture_shutdown(cx));
    let task = cx.background_executor.spawn(work);
    f.release.send(Ok(())).unwrap();
    task.await.unwrap();
    let values: Vec<_> = f
        .writes
        .lock()
        .unwrap()
        .iter()
        .map(|d| d.get::<TerminalSettings>().font_size)
        .collect();
    assert_eq!(values, [Some(19.), Some(20.)]);
}

#[gpui_kit::test]
fn leaving_the_actual_page_flushes_new_text_behind_a_pending_receipt(cx: &mut TestAppContext) {
    let f = gated_page(cx);
    enter_font(f.window, Some("19"), cx);
    font_draft(f.window, "20", cx);
    cx.update_window(f.window, |_, window, cx| {
        select_field_page("local.args", window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    f.release.send(Ok(())).unwrap();
    cx.run_until_parked();
    let values: Vec<_> = f
        .writes
        .lock()
        .unwrap()
        .iter()
        .map(|d| d.get::<TerminalSettings>().font_size)
        .collect();
    assert_eq!(values, [Some(19.), Some(20.)]);
    cx.update(|cx| {
        assert_eq!(
            f.settings.setting::<TerminalSettings>(cx).font_size,
            Some(20.)
        )
    });
    cx.update_window(f.window, |_, window, cx| {
        select_field_page("terminal.font_size", window, cx);
        assert_eq!(window.find("terminal.font_size").value(), Some("20"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn newer_validation_error_survives_an_older_physical_success(cx: &mut TestAppContext) {
    let f = gated_page(cx);
    enter_font(f.window, Some("19"), cx);
    enter_font(f.window, Some("100"), cx);
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("terminal.font_size").value(), Some("100"));
        assert!(
            window
                .find("settings-error-terminal.font_size")
                .label()
                .unwrap()
                .starts_with("Font size:")
        );
    })
    .unwrap();
    f.release.send(Ok(())).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            f.settings.setting::<TerminalSettings>(cx).font_size,
            Some(19.)
        )
    });
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("terminal.font_size").value(), Some("100"));
        assert!(
            window
                .find("settings-error-terminal.font_size")
                .label()
                .unwrap()
                .starts_with("Font size:")
        );
    })
    .unwrap();
    assert_eq!(f.writes.lock().unwrap().len(), 1);
    enter_font(f.window, Some("20"), cx);
    cx.update(|cx| {
        assert_eq!(
            f.settings.setting::<TerminalSettings>(cx).font_size,
            Some(20.)
        )
    });
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .try_find("settings-error-terminal.font_size")
                .is_none()
        );
    })
    .unwrap();
    assert_eq!(f.writes.lock().unwrap().len(), 2);
}
