//! The native confirmation keeps metadata-only removal default and explicit actions source-bound.
use super::{support::*, *};
use nocterm_containers::Engine;
use nocterm_project_providers::{
    ContainerAction, ContainerActionError, ContainerActionOutcome, ContainerLocation,
    ContainerOrigin, testing::FakeWorkspaceContainerActions,
};
fn container(host: HostLocator, engine: Engine) -> ProjectDefinition {
    let mut draft = definition(1).to_draft();
    draft.target = ContainerLocation::new(
        host,
        engine,
        "a".repeat(64),
        RootPath::parse("/home").unwrap(),
        ContainerOrigin::Attached,
    )
    .unwrap()
    .to_target()
    .unwrap();
    draft.try_into().unwrap()
}
fn form(
    original: ProjectDefinition,
    fake: &FakeWorkspaceContainerActions,
    writer: nocterm_store::SnapshotWriter<Catalog>,
    cx: &mut TestAppContext,
) -> Fixture {
    let rig = Fixture::with_installation(
        vec![original],
        writer,
        |installed| {
            installed
                .with_local_host(Some(
                    Arc::new(nocterm_session::testing::FakeExec::default()),
                ))
                .with_container_actions(Arc::new(fake.clone()))
        },
        Rc::new(nocterm_workspace::test_support::ProjectTerminalProbe::default()),
        cx,
    );
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.hover("project-row-01010101010101010101010101010101", cx);
        window.click("project-remove-01010101010101010101010101010101", cx);
        window.render_frame(cx);
        assert_eq!(rig.workspace.read(cx).items(cx).count(), 0);
    })
    .unwrap();
    rig
}
#[gpui_kit::test]
fn container_removal_defaults_to_keep_and_explicit_choices_show_their_consequences(
    cx: &mut TestAppContext,
) {
    for engine in Engine::ALL {
        let fake = FakeWorkspaceContainerActions::default();
        let rig = form(
            container(HostLocator::Local, engine),
            &fake,
            Arc::new(|_| Box::pin(async { Ok(()) })),
            cx,
        );
        cx.update_window(rig.window, |_, window, cx| {
            assert_eq!(window.find("project-remove-keep").checked(), Some(true));
            assert_eq!(window.find("ok").label(), Some("Remove workspace"));
            window.click("project-remove-stop", cx);
            window.render_frame(cx);
            assert_eq!(window.find("project-remove-stop").checked(), Some(true));
            assert_eq!(window.find("ok").label(), Some("Stop and remove workspace"));
            assert!(
                window
                    .find("project-remove-consequences")
                    .label()
                    .unwrap()
                    .contains("Other workspaces")
            );
            window.click("project-remove-delete", cx);
            window.render_frame(cx);
            assert_eq!(window.find("project-remove-delete").checked(), Some(true));
            assert_eq!(
                window.find("ok").label(),
                Some("Delete container and workspace")
            );
            window.click("project-remove-keep", cx);
            window.render_frame(cx);
            window.click("ok", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(fake.calls().unwrap().is_empty());
        cx.update(|cx| assert!(rig.model.catalog(cx).get(definition(1).id()).is_none()));
    }
}
#[gpui_kit::test]
fn offline_remote_container_can_keep_but_never_uses_the_available_local_executor(
    cx: &mut TestAppContext,
) {
    let fake = FakeWorkspaceContainerActions::default();
    let rig = form(
        container(remote(Some("remote"), "server.example"), Engine::Docker),
        &fake,
        Arc::new(|_| Box::pin(async { Ok(()) })),
        cx,
    );
    cx.update_window(rig.window, |_, window, cx| {
        assert!(window.try_find("project-remove-unavailable").is_some());
        window.click("project-remove-delete", cx);
        window.render_frame(cx);
        assert_eq!(window.find("project-remove-keep").checked(), Some(true));
        window.click("ok", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(fake.calls().unwrap().is_empty());
    cx.update(|cx| assert!(rig.model.catalog(cx).get(definition(1).id()).is_none()));
}
#[gpui_kit::test]
fn pending_action_blocks_duplicate_submit_choices_and_dismissal_then_reports_failure_inline(
    cx: &mut TestAppContext,
) {
    let fake = FakeWorkspaceContainerActions::default();
    let (release, wait) = async_channel::bounded::<()>(1);
    fake.suspend(Box::pin(async move {
        let _ = wait.recv().await;
    }))
    .unwrap();
    fake.respond(Err(ContainerActionError::TimedOut)).unwrap();
    let original = container(HostLocator::Local, Engine::Podman);
    let rig = form(
        original.clone(),
        &fake,
        Arc::new(|_| Box::pin(async { Ok(()) })),
        cx,
    );
    cx.update_window(rig.window, |_, window, cx| {
        window.click("project-remove-delete", cx);
        window.render_frame(cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
        window.click("ok", cx);
        window.click("cancel", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-remove-dialog").is_some());
    })
    .unwrap();
    assert_eq!(fake.calls().unwrap().len(), 1);
    release.try_send(()).unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("project-remove-error")
                .label()
                .unwrap()
                .contains("outcome may be unknown")
        );
        assert_eq!(rig.model.catalog(cx).get(original.id()), Some(&original));
        window.click("project-remove-keep", cx);
        window.render_frame(cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(fake.calls().unwrap().len(), 1);
    cx.update(|cx| assert!(rig.model.catalog(cx).get(original.id()).is_none()));
}
#[gpui_kit::test]
fn known_deleted_container_with_failed_save_retries_only_metadata(cx: &mut TestAppContext) {
    let fake = FakeWorkspaceContainerActions::default();
    fake.respond(Ok(ContainerActionOutcome {
        action: ContainerAction::Delete,
    }))
    .unwrap();
    let (writer, release, _) = nocterm_store::test_support::gated_writer().into_parts();
    let rig = form(
        container(HostLocator::Local, Engine::Docker),
        &fake,
        writer,
        cx,
    );
    cx.update_window(rig.window, |_, window, cx| {
        window.click("project-remove-delete", cx);
        window.render_frame(cx);
        window.click("ok", cx);
    })
    .unwrap();
    cx.run_until_parked();
    release
        .send(Err(nocterm_store::StoreError::backend(
            std::io::Error::other("disk full"),
        )))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("project-remove-error")
                .label()
                .unwrap()
                .starts_with("Container deleted.")
        );
        assert_eq!(window.find("ok").label(), Some("Retry removal"));
        window.click("ok", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(fake.calls().unwrap().len(), 1);
    cx.update(|cx| assert!(rig.model.catalog(cx).get(definition(1).id()).is_none()));
}

#[test]
fn shaped_removal_choices_and_consequences_keep_the_footer_reachable() {
    use gpui_kit::component::{Theme, ThemeMode};
    let output = std::env::var_os("NOCTERM_PROJECT_REMOVAL_SCREENSHOT");
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
    let installed = super::container_selection::native_installation(
        &mut cx,
        Default::default(),
        Default::default(),
    )
    .with_container_actions(Arc::new(FakeWorkspaceContainerActions::default()));
    for original in [container(HostLocator::Local, Engine::Docker), definition(2)] {
        cx.update(|cx| installed.model().save(original, None, cx))
            .detach();
        cx.run_until_parked();
    }
    let (window, workspace) = super::container_selection::native_window(&mut cx);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            register(workspace, &installed, window, cx).unwrap();
            workspace.open_panel(&crate::plugin::WORKSPACES.id, window, cx);
        });
        window.activate_window();
    })
    .unwrap();
    for (theme, scene, width, height, choice, project) in [
        (ThemeMode::Dark, "dark-keep", 1280., 720., "keep", 1),
        (ThemeMode::Dark, "dark-delete", 1280., 720., "delete", 1),
        (ThemeMode::Light, "light-delete", 1280., 720., "delete", 1),
        (ThemeMode::Dark, "small-delete", 760., 480., "delete", 1),
        (ThemeMode::Dark, "dark-folder", 1280., 720., "keep", 2),
    ] {
        cx.update(|cx| Theme::change(theme, None, cx));
        removal_scene(&mut cx, window, width, height, choice, project);
        if let Some(output) = &output {
            let path =
                std::path::PathBuf::from(output).with_file_name(format!("removal-{scene}.png"));
            cx.capture_screenshot(window).unwrap().save(path).unwrap();
        }
        cx.update_window(window, |_, window, cx| window.click("cancel", cx))
            .unwrap();
        cx.run_until_parked();
    }
}

fn removal_scene(
    cx: &mut gpui_kit::HeadlessAppContext,
    window: AnyWindowHandle,
    width: f32,
    height: f32,
    choice: &str,
    project: u8,
) {
    cx.update_window(window, |_, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(width), gpui_kit::px(height)));
        window.bounds_changed(cx);
        window.render_frame(cx);
        window.hover(format!("project-row-{}", definition(project).id()), cx);
        window.click(format!("project-remove-{}", definition(project).id()), cx);
        window.render_frame(cx);
        if project == 1 {
            window.click(format!("project-remove-{choice}"), cx);
            window.render_frame(cx);
            let identity = window.find("project-remove-container");
            assert_eq!(identity.label(), Some("Docker · aaaaaaaaaaaa"));
            for id in ["keep", "stop", "delete"] {
                let row = window.find(format!("project-remove-{id}"));
                assert!(row.bounds().bottom() <= window.find("ok").bounds().top());
                assert!(row.bounds().right() <= window.viewport_size().width);
            }
        } else {
            assert!(window.try_find("project-remove-choices").is_none());
            assert!(window.find("project-remove-dialog").bounds().size.height < gpui_kit::px(150.));
        }
        let footer = window.find("ok").bounds();
        assert!(window.find("project-remove-consequences").bounds().bottom() <= footer.top());
        assert!(footer.right() <= window.viewport_size().width);
        assert!(footer.bottom() < window.viewport_size().height);
        assert!(window.find("project-remove-dialog").bounds().top() >= gpui_kit::px(0.));
    })
    .unwrap();
    cx.run_until_parked();
    cx.advance_clock(std::time::Duration::from_millis(250));
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
}
