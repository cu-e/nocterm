//! Long engine data remains readable while exact identity controls selection.
use super::{support::*, *};
use nocterm_containers::{Container, Engine, Image, State};
use nocterm_project_providers::{
    EngineInventory, PreparedProject, SetupMode, testing::FakeProjectPreparation,
};

pub(super) fn inventory(engine: Engine) -> EngineInventory {
    let digest = format!("sha256:{}", "a".repeat(64));
    EngineInventory {
        engine,
        containers: Ok((1..=12)
            .map(|index| Container {
                id: format!("{index:064x}"),
                name: format!("team-workspace-{index}-with-a-deliberately-long-container-name"),
                image: if index == 1 {
                    digest.clone()
                } else {
                    format!("sha256:{}", "b".repeat(64))
                },
                state: State::Running,
                status: "Up 3 hours (healthy) with an unusually long engine status summary".into(),
                project: None,
                service: None,
            })
            .collect()),
        images: Ok(vec![Image {
            id: digest,
            repository: Some("registry.example/team/debian-development".into()),
            tag: Some("stable".into()),
            size: "2GB".into(),
            created: "today".into(),
        }]),
    }
}
fn assert_rows_fit(window: &gpui_kit::Window) {
    use gpui_kit::px;
    let first = format!("project-container-Docker-{:064x}", 1);
    let second = format!("project-container-Docker-{:064x}", 2);
    let row = window.find(first.clone()).bounds();
    let name = window.find(format!("{first}-name")).bounds();
    let image = window.find(format!("{first}-image")).bounds();
    let status = window.find(format!("{first}-status")).bounds();
    let list = window.find("project-container-list").bounds();
    assert!(row.size.height > px(50.));
    assert!(name.bottom() <= image.top());
    assert!(image.bottom() <= status.top());
    assert!(status.bottom() <= row.bottom());
    assert!(row.bottom() <= window.find(second.clone()).bounds().top());
    assert!(status.right() <= row.right());
    assert!(list.size.height <= px(208.));
    assert!(list.bottom() <= window.find("project-form-body").bounds().bottom());
    assert!(window.find("project-form-footer").bounds().bottom() <= window.viewport_size().height);
    assert!(
        window
            .find(first)
            .label()
            .unwrap()
            .contains(&"a".repeat(64))
    );
}

#[gpui_kit::test]
fn local_attach_preserves_the_selected_engine_and_full_container_id(cx: &mut TestAppContext) {
    let preparation = FakeProjectPreparation::default();
    preparation
        .inventory(vec![inventory(Engine::Docker), inventory(Engine::Podman)])
        .unwrap();
    preparation
        .respond(Ok(PreparedProject {
            target: definition(20).target().clone(),
            owned: None,
        }))
        .unwrap();
    let (rig, exec, _) = Fixture::local(preparation.clone(), cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        fill(window, "project-name", "Selected container", cx);
        wizard_field(window, "project-mode-existing", cx);
        window.click("project-mode-existing", cx);
        window.render_frame(cx);
        assert_rows_fit(window);
        let selected = format!("project-container-Docker-{:064x}", 2);
        window.click(selected.clone(), cx);
        window.render_frame(cx);
        assert_eq!(window.find(selected).checked(), Some(true));
        create_workspace(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let requests = preparation.requests().unwrap();
    assert_eq!(requests[0].host, HostLocator::Local);
    assert!(matches!(&requests[0].mode, SetupMode::Attach {
        engine: Engine::Docker, container, ..
    } if container == &format!("{:064x}", 2)));
    assert!(Arc::ptr_eq(&preparation.prepared().unwrap()[0], &exec));
}

pub(super) fn native_installation(
    cx: &mut gpui_kit::HeadlessAppContext,
    preparation: FakeProjectPreparation,
    search: nocterm_project_providers::testing::FakeImageSearch,
) -> Installed {
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        gpui_kit::init(cx);
        gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
        cx.set_global(Design::new(DesignTokens::builtin()));
        let model = ProjectsHandle::install(
            PreparedProjects::loaded(Ok(Some(Catalog::default())), true),
            Arc::new(|_| Box::pin(async { Ok(()) })),
            Arc::new(testing::SequenceProjectMetadata::new(10_000, 123)),
            &nocterm_store::ShutdownCoordinator::default(),
            cx,
        )
        .unwrap();
        init(
            model,
            Rc::new(nocterm_workspace::test_support::ProjectTerminalProbe::default()),
            nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx),
            cx,
        )
        .with_image_search(Arc::new(search))
        .with_preparation(Arc::new(preparation))
        .with_local_host(Some(
            Arc::new(nocterm_session::testing::FakeExec::default()),
        ))
    })
}
pub(super) fn native_window(
    cx: &mut gpui_kit::HeadlessAppContext,
) -> (AnyWindowHandle, Entity<Workspace>) {
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::open_window(
            gpui_kit::WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::new(
                    gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(0.)),
                    gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(800.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    Workspace::new(
                        nocterm_workspace::test_support::deps_for(&[&crate::plugin::PLUGIN], cx),
                        window,
                        cx,
                    )
                })
            },
        )
        .unwrap()
    });
    (window, workspace)
}
#[test]
fn shaped_container_rows_keep_separate_lines_and_a_reachable_footer() {
    let screenshot = std::env::var_os("NOCTERM_PROJECT_PICKER_SCREENSHOT");
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
    let preparation = FakeProjectPreparation::default();
    preparation
        .inventory(vec![inventory(Engine::Docker), inventory(Engine::Podman)])
        .unwrap();
    let search = nocterm_project_providers::testing::FakeImageSearch::default();
    search
        .respond(Ok(vec![
            super::image_selection::registry_image("debian"),
            super::image_selection::registry_image("team/debian-development"),
        ]))
        .unwrap();
    let installed = native_installation(&mut cx, preparation, search);
    let (window, workspace) = native_window(&mut cx);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            register(workspace, &installed, window, cx).unwrap();
        });
        window.activate_window();
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        wizard_field(window, "project-mode-existing", cx);
        window.click("project-mode-existing", cx);
        window.render_frame(cx);
        window.click(format!("project-container-Docker-{:064x}", 2), cx);
        window.render_frame(cx);
        assert_rows_fit(window);
    })
    .unwrap();
    cx.advance_clock(std::time::Duration::from_millis(250));
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    if let Some(path) = screenshot.as_ref() {
        cx.capture_screenshot(window).unwrap().save(path).unwrap();
    }
    super::image_selection::native_creation_variants(&mut cx, window, screenshot.as_deref());
}

#[gpui_kit::test]
fn container_search_matches_name_full_id_image_and_engine_without_changing_selection(
    cx: &mut TestAppContext,
) {
    let preparation = FakeProjectPreparation::default();
    preparation
        .inventory(vec![inventory(Engine::Docker), inventory(Engine::Podman)])
        .unwrap();
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
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-mode-existing", cx);
        window.render_frame(cx);
        fill(window, "project-container-search", "Podman", cx);
        window.render_frame(cx);
        window.click(format!("project-container-Podman-{:064x}", 2), cx);
        for query in [
            "team-workspace-1-",
            &format!("{:064x}", 1),
            "debian-development",
            "DOCKER",
        ] {
            fill(window, "project-container-search", query, cx);
            window.render_frame(cx);
            assert!(
                window
                    .try_find(format!("project-container-Docker-{:064x}", 1))
                    .is_some(),
                "{query}"
            );
        }
        assert!(
            window
                .try_find(format!("project-container-Podman-{:064x}", 2))
                .is_none()
        );
        assert!(
            window
                .find("project-container-selected")
                .label()
                .unwrap()
                .ends_with(&format!("{:064x}", 2))
        );
        fill(window, "project-container-search", "does-not-exist", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-container-no-matches").is_some());
        fill(window, "project-container-search", "", cx);
        window.render_frame(cx);
        assert_eq!(
            window
                .find(format!("project-container-Podman-{:064x}", 2))
                .checked(),
            Some(true)
        );
        window.click("project-next", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Filtered selection", cx);
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        matches!(&preparation.requests().unwrap()[0].mode, SetupMode::Attach { engine: Engine::Podman, container, root: Some(root) } if container == &format!("{:064x}", 2) && root.as_str() == "/home")
    );
}

#[gpui_kit::test]
fn container_search_keeps_independent_engine_errors_visible(cx: &mut TestAppContext) {
    let preparation = FakeProjectPreparation::default();
    let mut podman = inventory(Engine::Podman);
    podman.containers = Err(nocterm_containers::ContainersError::Unsupported);
    preparation
        .inventory(vec![inventory(Engine::Docker), podman])
        .unwrap();
    let (rig, _, _) = Fixture::local(preparation, cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-mode-existing", cx);
        window.render_frame(cx);
        fill(window, "project-container-search", "missing", cx);
        window.render_frame(cx);
        assert!(
            window
                .try_find("project-container-engine-error-Podman")
                .is_some()
        );
        assert!(window.try_find("project-container-no-matches").is_some());
    })
    .unwrap();
}
