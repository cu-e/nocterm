//! Bounded shaped settings captures use the actual native text and WGPU paths.
use super::*;
use gpui_kit::component::{Theme, ThemeMode};
use nocterm_project_providers::testing::{FakeImageSearch, FakeProjectPreparation};

#[test]
fn shaped_environment_settings_keep_facts_and_explicit_actions_reachable() {
    let screenshot = std::env::var_os("NOCTERM_PROJECT_ENVIRONMENT_SCREENSHOT");
    let render = screenshot.is_some();
    let mut cx = gpui_kit::HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(nocterm_ui::Assets::with_marks(crate::marks())),
        move || {
            if render {
                gpui_kit::platform::current_headless_renderer()
            } else {
                Ok(None)
            }
        },
    );
    let fake = FakeWorkspaceEnvironment::default();
    for _ in 0..2 {
        fake.respond_inspect(Ok(snapshot("Python 3.13.9"))).unwrap();
    }
    let preparation = FakeProjectPreparation::default();
    let installed = super::super::container_selection::native_installation(
        &mut cx,
        preparation.clone(),
        FakeImageSearch::default(),
    )
    .with_environment(Arc::new(fake));
    let original = container_definition(HostLocator::Local, Engine::Podman);
    cx.update(|cx| installed.model().save(original, None, cx))
        .detach();
    cx.run_until_parked();
    let (window, workspace) = super::super::container_selection::native_window(&mut cx);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            register(workspace, &installed, window, cx);
            workspace.activate_panel_of::<ProjectsPanel>(window, cx);
        });
        window.activate_window();
    })
    .unwrap();
    cx.run_until_parked();
    for (mode, theme) in [(ThemeMode::Dark, "dark"), (ThemeMode::Light, "light")] {
        cx.update(|cx| Theme::change(mode, None, cx));
        open_row_settings(&mut cx, window, screenshot.as_deref(), theme);
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window
                    .find("project-environment-container")
                    .label()
                    .unwrap()
                    .contains("actual-development-container")
            );
            assert!(
                window.find("project-environment-actions").bounds().bottom()
                    <= window.find("project-save").bounds().top()
            );
            assert!(window.find("project-save").bounds().bottom() < window.viewport_size().height);
        })
        .unwrap();
        capture(&mut cx, window, screenshot.as_deref(), theme, "facts");
        cx.update_window(window, |_, window, cx| {
            reveal_form(window, "project-language-row-java", cx);
            let body = window.find("project-form-body").bounds();
            assert!(window.find("project-language-row-java").bounds().bottom() <= body.bottom());
            assert!(window.find("project-environment-install").bounds().top() >= body.bottom());
        })
        .unwrap();
        capture(&mut cx, window, screenshot.as_deref(), theme, "tools");
        cx.update_window(window, |_, window, cx| window.click("project-cancel", cx))
            .unwrap();
        cx.run_until_parked();
    }
    creation_progress(&mut cx, window, &preparation, screenshot.as_deref());
}
fn open_row_settings(
    cx: &mut gpui_kit::HeadlessAppContext,
    window: AnyWindowHandle,
    screenshot: Option<&std::ffi::OsStr>,
    theme: &str,
) {
    let edit = "project-edit-01010101010101010101010101010101";
    cx.update_window(window, |_, window, cx| {
        window.hover("project-new", cx);
        assert!(!window.find(edit).visible());
    })
    .unwrap();
    capture(cx, window, screenshot, theme, "row-idle");
    cx.update_window(window, |_, window, cx| {
        window.hover("project-row-01010101010101010101010101010101", cx);
        assert!(window.find(edit).visible());
    })
    .unwrap();
    capture(cx, window, screenshot, theme, "row-hover");
    cx.update_window(window, |_, window, cx| window.click(edit, cx))
        .unwrap();
}

fn capture(
    cx: &mut gpui_kit::HeadlessAppContext,
    window: AnyWindowHandle,
    prefix: Option<&std::ffi::OsStr>,
    theme: &str,
    variant: &str,
) {
    if let Some(prefix) = prefix {
        let base = std::path::PathBuf::from(prefix);
        let output = base.with_file_name(format!(
            "{}-{theme}-{variant}.png",
            base.file_stem().unwrap().to_string_lossy()
        ));
        cx.capture_screenshot(window).unwrap().save(output).unwrap();
    }
}

fn creation_progress(
    cx: &mut gpui_kit::HeadlessAppContext,
    window: AnyWindowHandle,
    preparation: &FakeProjectPreparation,
    screenshot: Option<&std::ffi::OsStr>,
) {
    use nocterm_project_providers::{SetupPhase, SetupUpdate, TransferProgress};
    for (mode, theme, total) in [
        (ThemeMode::Dark, "dark", Some(100)),
        (ThemeMode::Light, "light", None),
    ] {
        cx.update(|cx| Theme::change(mode, None, cx));
        preparation
            .progress(vec![SetupUpdate {
                phase: SetupPhase::PullingImage,
                transfer: Some(TransferProgress::new(Some("Layer abc"), 25, total)),
            }])
            .unwrap();
        preparation
            .respond(Err(nocterm_project_providers::ProjectSetupError::Canceled))
            .unwrap();
        let (release, wait) = async_channel::bounded::<()>(1);
        preparation
            .suspend(Box::pin(async move {
                let _ = wait.recv().await;
            }))
            .unwrap();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("project-new", cx);
            window.render_frame(cx);
            fill(window, "project-name", "Development workspace", cx);
            wizard_field(window, "project-mode-new", cx);
            window.click("project-mode-new", cx);
            window.render_frame(cx);
            create_workspace(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.find("project-progress").bounds().right()
                    <= window.find("project-cancel").bounds().left()
            );
            assert!(window.find("project-save").bounds().bottom() < window.viewport_size().height);
        })
        .unwrap();
        capture(cx, window, screenshot, theme, "pull");
        cx.update_window(window, |_, window, cx| window.click("project-cancel", cx))
            .unwrap();
        release.try_send(()).unwrap();
        cx.run_until_parked();
    }
}
