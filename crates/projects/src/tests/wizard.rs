//! Creation asks only the relevant questions and preserves the user's draft.
use super::{support::*, *};
use nocterm_project_providers::{PreparedProject, SetupMode, testing::FakeProjectPreparation};

fn new_wizard(cx: &mut TestAppContext) -> (Fixture, FakeProjectPreparation) {
    let preparation = FakeProjectPreparation::default();
    preparation
        .respond(Ok(PreparedProject {
            target: definition(20).target().clone(),
            owned: None,
        }))
        .unwrap();
    let (rig, _, _) = Fixture::local(preparation.clone(), cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
    })
    .unwrap();
    (rig, preparation)
}

#[gpui_kit::test]
fn connection_branches_show_two_or_three_relevant_steps_without_admitting_setup(
    cx: &mut TestAppContext,
) {
    let (rig, preparation) = new_wizard(cx);
    cx.update_window(rig.window, |_, window, cx| {
        assert!(window.try_find("project-step-tools").is_none());
        assert!(window.try_find("project-name").is_none());
        assert!(window.try_find("project-save").is_none());
        window.click("project-step-project", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("project-step-heading").label(),
            Some("Connection")
        );
        window.click("project-mode-new", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-step-tools").is_some());
        assert!(window.try_find("project-languages").is_none());
        window.click("project-mode-existing", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-step-tools").is_none());
        window.click("project-next", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("project-step-heading").label(),
            Some("Connection")
        );
        assert!(window.try_find("project-form-error").is_some());
    })
    .unwrap();
    cx.run_until_parked();
    assert!(preparation.requests().unwrap().is_empty());
}

#[gpui_kit::test]
fn next_validates_only_its_section_and_back_preserves_project_fields(cx: &mut TestAppContext) {
    let (rig, preparation) = new_wizard(cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.press("enter", cx);
        window.render_frame(cx);
        assert_eq!(window.find("project-step-heading").label(), Some("Project"));
        assert!(window.try_find("project-save").is_some());
        window.click("project-save", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-form-error").is_some());
        window.input("Retained name", cx);
        window.click("project-back", cx);
        window.render_frame(cx);
        window.click("project-next", cx);
        window.render_frame(cx);
        assert_eq!(
            window.within("project-name").find("input").value(),
            Some("Retained name")
        );
        assert!(window.try_find("project-git").is_some());
        assert!(window.try_find("project-mode-new").is_none());
    })
    .unwrap();
    cx.run_until_parked();
    assert!(preparation.requests().unwrap().is_empty());
}

#[gpui_kit::test]
fn invalid_custom_image_stays_on_connection_and_receives_keyboard_focus(cx: &mut TestAppContext) {
    let (rig, preparation) = new_wizard(cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.click("project-mode-new", cx);
        window.render_frame(cx);
        window.click("project-image-Custom image", cx);
        window.render_frame(cx);
        window.click("project-next", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("project-step-heading").label(),
            Some("Connection")
        );
        assert!(window.try_find("project-form-error").is_some());
        window.input("alpine:3.22", cx);
        window.click("project-next", cx);
        window.render_frame(cx);
        assert_eq!(window.find("project-step-heading").label(), Some("Project"));
        assert!(window.try_find("project-save").is_none());
    })
    .unwrap();
    cx.run_until_parked();
    assert!(preparation.requests().unwrap().is_empty());
}

#[gpui_kit::test]
fn switching_to_direct_omits_retained_container_tool_drafts_from_the_final_request(
    cx: &mut TestAppContext,
) {
    let (rig, preparation) = new_wizard(cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.click("project-mode-new", cx);
        window.render_frame(cx);
        window.click("project-image-Custom image", cx);
        window.render_frame(cx);
        fill(window, "project-custom-image", "team/retained:custom", cx);
        window.click("project-next", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Retained workspace", cx);
        window.click("project-next", cx);
        window.render_frame(cx);
        window.click("project-language-rust", cx);
        window.click("project-back", cx);
        window.render_frame(cx);
        window.click("project-back", cx);
        window.render_frame(cx);
        assert_eq!(
            window.within("project-custom-image").find("input").value(),
            Some("team/retained:custom")
        );
        window.click("project-mode-direct", cx);
        window.render_frame(cx);
        window.click("project-next", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-step-tools").is_none());
        assert_eq!(
            window.within("project-name").find("input").value(),
            Some("Retained workspace")
        );
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(preparation.requests().unwrap().len(), 1);
    assert!(matches!(
        preparation.requests().unwrap()[0].mode,
        SetupMode::Direct { .. }
    ));
}

#[test]
fn shaped_wizard_keeps_relevant_steps_and_footer_visible_at_720p_and_in_small_windows() {
    use gpui_kit::component::ThemeMode;
    let output = std::env::var_os("NOCTERM_PROJECT_WIZARD_SCREENSHOT");
    let render = output.is_some();
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
    let preparation = FakeProjectPreparation::default();
    for _ in 0..2 {
        preparation
            .inventory(vec![
                super::container_selection::inventory(nocterm_containers::Engine::Docker),
                super::container_selection::inventory(nocterm_containers::Engine::Podman),
            ])
            .unwrap();
    }
    let installed =
        super::container_selection::native_installation(&mut cx, preparation, Default::default());
    let (window, workspace) = super::container_selection::native_window(&mut cx);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            register(workspace, &installed, window, cx).unwrap();
        });
        window.resize(gpui_kit::size(gpui_kit::px(1280.), gpui_kit::px(720.)));
        window.bounds_changed(cx);
        assert_eq!(
            window.viewport_size(),
            gpui_kit::size(gpui_kit::px(1280.), gpui_kit::px(720.))
        );
        window.activate_window();
    })
    .unwrap();
    for (theme, name) in [(ThemeMode::Dark, "dark"), (ThemeMode::Light, "light")] {
        wizard_native_theme(&mut cx, window, output.as_deref(), theme, name);
    }

    cx.update_window(window, |_, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(760.), gpui_kit::px(480.)));
        window.bounds_changed(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.viewport_size(),
            gpui_kit::size(gpui_kit::px(760.), gpui_kit::px(480.))
        );
        window.click("project-new", cx);
        window.render_frame(cx);
        window.click("project-mode-new", cx);
        window.render_frame(cx);
        window.click("project-next", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Small window", cx);
        window.click("project-next", cx);
        window.render_frame(cx);
        assert!(
            window.find("project-form-footer").bounds().bottom() <= window.viewport_size().height
        );
        assert!(window.find("project-wizard-steps").bounds().top() >= gpui_kit::px(0.));
        assert!(
            window.find("project-form-footer").bounds().right() <= window.viewport_size().width
        );
        assert!(window.find("project-form-body").bounds().size.height <= gpui_kit::px(180.));
    })
    .unwrap();
    if let Some(output) = output.as_deref() {
        let path = std::path::PathBuf::from(output).with_file_name("wizard-small-fallback.png");
        cx.capture_screenshot(window).unwrap().save(path).unwrap();
    }
}

fn wizard_native_theme(
    cx: &mut gpui_kit::HeadlessAppContext,
    window: AnyWindowHandle,
    output: Option<&std::ffi::OsStr>,
    theme: gpui_kit::component::ThemeMode,
    name: &str,
) {
    use gpui_kit::component::Theme;
    cx.update(|cx| Theme::change(theme, None, cx));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
    })
    .unwrap();
    wizard_capture(
        cx,
        window,
        output,
        name,
        "direct-connection",
        "project-mode-new",
    );
    cx.update_window(window, |_, window, cx| {
        window.click("project-next", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Development workspace", cx);
    })
    .unwrap();
    wizard_capture(cx, window, output, name, "project", "project-git");
    cx.update_window(window, |_, window, cx| {
        window.click("project-back", cx);
        window.render_frame(cx);
        window.click("project-mode-existing", cx);
        window.render_frame(cx);
    })
    .unwrap();
    wizard_capture(
        cx,
        window,
        output,
        name,
        "existing-connection",
        "project-container-list",
    );
    cx.update_window(window, |_, window, cx| {
        window.click("project-mode-new", cx);
        window.render_frame(cx);
        window.click("project-image-Custom image", cx);
        window.render_frame(cx);
        fill(
            window,
            "project-custom-image",
            "team/debian-development:stable",
            cx,
        );
    })
    .unwrap();
    wizard_capture(
        cx,
        window,
        output,
        name,
        "new-image",
        "project-available-images",
    );
    cx.update_window(window, |_, window, cx| {
        window.click("project-next", cx);
        window.render_frame(cx);
        window.click("project-next", cx);
        window.render_frame(cx);
    })
    .unwrap();
    wizard_capture(
        cx,
        window,
        output,
        name,
        "tools",
        "project-language-row-java",
    );
    cx.update_window(window, |_, window, cx| window.click("project-cancel", cx))
        .unwrap();
}

fn wizard_capture(
    cx: &mut gpui_kit::HeadlessAppContext,
    window: AnyWindowHandle,
    output: Option<&std::ffi::OsStr>,
    theme: &str,
    step: &str,
    last_field: &str,
) {
    cx.run_until_parked();
    cx.advance_clock(std::time::Duration::from_millis(250));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let body = window.find("project-form-body").bounds();
        let field = window.find(last_field.to_owned()).bounds();
        assert!(
            field.top() >= body.top(),
            "{theme} {step}: field above body"
        );
        assert!(
            field.bottom() <= body.bottom(),
            "{theme} {step}: body needs scrolling"
        );
        assert!(
            window.find("project-form-footer").bounds().bottom() <= window.viewport_size().height
        );
    })
    .unwrap();
    if let Some(output) = output {
        let path =
            std::path::PathBuf::from(output).with_file_name(format!("wizard-{theme}-{step}.png"));
        cx.capture_screenshot(window).unwrap().save(path).unwrap();
    }
}

#[gpui_kit::test]
fn returning_to_new_container_retains_tool_versions_and_validates_the_visible_step(
    cx: &mut TestAppContext,
) {
    let (rig, preparation) = new_wizard(cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.click("project-mode-new", cx);
        window.render_frame(cx);
        window.click("project-next", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Version draft", cx);
        window.click("project-next", cx);
        window.render_frame(cx);
        super::creation::open_custom_version(window, "python", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        fill(
            window,
            "project-version-custom-python",
            "invalid version",
            cx,
        );
        window.click("project-back", cx);
        window.render_frame(cx);
        window.click("project-back", cx);
        window.render_frame(cx);
        window.click("project-mode-direct", cx);
        window.render_frame(cx);
        window.click("project-mode-new", cx);
        window.render_frame(cx);
        window.click("project-next", cx);
        window.render_frame(cx);
        window.click("project-next", cx);
        window.render_frame(cx);
        assert_eq!(
            window
                .within("project-version-custom-python")
                .find("input")
                .value(),
            Some("invalid version")
        );
        window.click("project-save", cx);
        window.render_frame(cx);
        assert_eq!(window.find("project-step-heading").label(), Some("Tools"));
        assert!(window.try_find("project-form-error").is_some());
        window.press("ctrl-a", cx);
        window.input("3.13", cx);
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let requests = preparation.requests().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        matches!(&requests[0].mode, SetupMode::Create { languages, .. } if languages == &[
            nocterm_project_providers::LanguageSelection::parse(nocterm_project_providers::ProgrammingLanguage::Python, "3.13").unwrap()
        ])
    );
}
