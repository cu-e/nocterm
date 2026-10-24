//! Native workspace creation keeps source context fixed and discovery results readable.
use super::{support::*, *};
use gpui_kit::{
    component::{Theme, ThemeMode},
    px,
};
use nocterm_containers::{Engine, Image};
use nocterm_project_providers::testing::{FakeImageSearch, FakeProjectPreparation};

#[test]
fn shaped_workspace_discovery_keeps_header_results_and_footer_usable() {
    let output = std::env::var_os("NOCTERM_PROJECT_USABILITY_SCREENSHOT");
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
    let search = FakeImageSearch::default();
    for _ in 0..2 {
        let mut inventory = super::container_selection::inventory(Engine::Docker);
        inventory.images = Ok((0..5)
            .map(|index| Image {
                id: format!("local-{index}"),
                repository: Some(format!("team/debian-development-{index}")),
                tag: Some("stable".into()),
                size: "120 MB".into(),
                created: "today".into(),
            })
            .collect());
        preparation
            .inventory(vec![
                inventory,
                super::container_selection::inventory(Engine::Podman),
            ])
            .unwrap();
        search
            .respond(Ok((0..5)
                .map(|index| {
                    super::image_selection::registry_image(&format!(
                        "team/debian-development-{index}"
                    ))
                })
                .collect()))
            .unwrap();
    }
    let installed = super::container_selection::native_installation(&mut cx, preparation, search);
    let (window, workspace) = super::container_selection::native_window(&mut cx);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            register(workspace, &installed, window, cx).unwrap();
        });
        window.resize(gpui_kit::size(px(1280.), px(720.)));
        window.bounds_changed(cx);
        window.activate_window();
    })
    .unwrap();
    for (theme, label) in [(ThemeMode::Dark, "dark"), (ThemeMode::Light, "light")] {
        native_discovery(&mut cx, window, theme, label, output.as_deref());
        native_settings(&mut cx, &installed, theme, label, output.as_deref());
    }
    cx.update_window(window, |_, window, cx| {
        window.resize(gpui_kit::size(px(760.), px(480.)));
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        window.click("project-mode-new", cx);
        window.render_frame(cx);
        window.click("project-image-Custom image", cx);
        window.render_frame(cx);
        assert_fixed_chrome(window);
    })
    .unwrap();
    capture(&mut cx, window, output.as_deref(), "small", "discovery");
}

fn native_discovery(
    cx: &mut gpui_kit::HeadlessAppContext,
    window: AnyWindowHandle,
    theme: ThemeMode,
    label: &str,
    output: Option<&std::ffi::OsStr>,
) {
    cx.update(|cx| Theme::change(theme, None, cx));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-mode-existing", cx);
        window.render_frame(cx);
        fill(window, "project-container-search", "Podman", cx);
        window.render_frame(cx);
        window.click(format!("project-container-Podman-{:064x}", 2), cx);
        assert_fixed_chrome(window);
    })
    .unwrap();
    capture(cx, window, output, label, "containers");
    native_images(cx, window, label, output);
    cx.update_window(window, |_, window, cx| {
        window.click("project-next", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Development workspace", cx);
        assert_fixed_chrome(window);
    })
    .unwrap();
    capture(cx, window, output, label, "project");
    cx.update_window(window, |_, window, cx| {
        window.click("project-next", cx);
        window.render_frame(cx);
        assert_fixed_chrome(window);
    })
    .unwrap();
    capture(cx, window, output, label, "tools");
    cx.update_window(window, |_, window, cx| window.click("project-cancel", cx))
        .unwrap();
}

fn native_images(
    cx: &mut gpui_kit::HeadlessAppContext,
    window: AnyWindowHandle,
    label: &str,
    output: Option<&std::ffi::OsStr>,
) {
    cx.update_window(window, |_, window, cx| {
        window.click("project-mode-new", cx);
        window.render_frame(cx);
        window.click("project-image-Custom image", cx);
        window.render_frame(cx);
        fill(window, "project-custom-image", "debian", cx);
        window.render_frame(cx);
        assert_three_rows(
            window,
            "project-available-images",
            "project-local-image-local-0",
            "project-local-image-local-2",
        );
    })
    .unwrap();
    capture(cx, window, output, label, "local-images");
    cx.advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.click("project-image-source-registry", cx);
        window.render_frame(cx);
        assert_three_rows(
            window,
            "project-registry-images",
            "project-registry-image-team/debian-development-0",
            "project-registry-image-team/debian-development-2",
        );
        assert_eq!(
            window.within("project-custom-image").find("input").value(),
            Some("debian")
        );
    })
    .unwrap();
    capture(cx, window, output, label, "registry-images");
}

fn assert_fixed_chrome(window: &gpui_kit::Window) {
    let server = window.find("project-form-server").bounds();
    let steps = window.find("project-wizard-steps").bounds();
    let body = window.find("project-form-body").bounds();
    let footer = window.find("project-form-footer").bounds();
    assert!(server.bottom() <= steps.top());
    assert!(steps.bottom() <= body.top());
    assert!(body.bottom() <= footer.top());
    assert!(footer.bottom() <= window.viewport_size().height);
    assert!(footer.right() <= window.viewport_size().width);
}
fn assert_three_rows(window: &gpui_kit::Window, list: &str, first: &str, third: &str) {
    assert_fixed_chrome(window);
    let bounds = window.find(list.to_owned()).bounds();
    assert!(bounds.size.height >= px(140.));
    assert!(window.find(first.to_owned()).bounds().top() >= bounds.top());
    assert!(window.find(third.to_owned()).bounds().bottom() <= bounds.bottom());
    assert!(bounds.bottom() <= window.find("project-form-body").bounds().bottom());
}
fn native_settings(
    cx: &mut gpui_kit::HeadlessAppContext,
    installed: &Installed,
    theme: ThemeMode,
    label: &str,
    output: Option<&std::ffi::OsStr>,
) {
    cx.update(|cx| Theme::change(theme, None, cx));
    let (window, _) = cx.update(|cx| {
        gpui_kit::open_window(
            gpui_kit::WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::new(
                    gpui_kit::point(px(0.), px(0.)),
                    gpui_kit::size(px(1280.), px(720.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    crate::settings_page::WorkspacesPage::new(
                        installed.settings().clone(),
                        window,
                        cx,
                    )
                })
            },
        )
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.find("workspace-default-directory").bounds().bottom()
                < window.viewport_size().height
        );
    })
    .unwrap();
    capture(cx, window, output, label, "settings");
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
}
fn capture(
    cx: &mut gpui_kit::HeadlessAppContext,
    window: AnyWindowHandle,
    output: Option<&std::ffi::OsStr>,
    theme: &str,
    scene: &str,
) {
    cx.run_until_parked();
    cx.advance_clock(std::time::Duration::from_millis(250));
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    if let Some(output) = output {
        if std::env::var("NOCTERM_PROJECT_USABILITY_SCENES")
            .is_ok_and(|scenes| !scenes.split(',').any(|selected| selected == scene))
        {
            return;
        }
        let path = std::path::PathBuf::from(output)
            .with_file_name(format!("workspace-{theme}-{scene}.png"));
        cx.capture_screenshot(window).unwrap().save(path).unwrap();
    }
}
