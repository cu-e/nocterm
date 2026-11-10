//! Environment facts are observed and tool changes require a separate explicit action.
use super::{support::*, *};
mod native;
use nocterm_containers::Engine;
use nocterm_project_providers::{
    ContainerFacts, ContainerLocation, ContainerOrigin, DistributionFamily, EnvironmentAccount,
    EnvironmentError, EnvironmentExecution, EnvironmentOs, EnvironmentSnapshot, LibcFamily,
    ProgrammingLanguage, ToolObservation, testing::FakeWorkspaceEnvironment,
};

fn container_definition(host: HostLocator, engine: Engine) -> ProjectDefinition {
    let mut draft = definition(1).to_draft();
    draft.target = ContainerLocation::new(
        host,
        engine,
        "a".repeat(64),
        RootPath::parse("/work").unwrap(),
        ContainerOrigin::Attached,
    )
    .unwrap()
    .to_target()
    .unwrap();
    draft.try_into().unwrap()
}
fn snapshot(version: &str) -> EnvironmentSnapshot {
    EnvironmentSnapshot {
        os: EnvironmentOs {
            kernel: "Linux".into(),
            id: "alpine".into(),
            name: "Alpine Linux".into(),
            release: "3.22".into(),
            architecture: "x86_64".into(),
            libc: LibcFamily::Musl,
            family: DistributionFamily::Alpine,
        },
        account: EnvironmentAccount {
            user: "alice".into(),
            uid: 1000,
            home: "/home/alice".into(),
            writable_home: true,
        },
        tools: vec![ToolObservation {
            language: ProgrammingLanguage::Python,
            version: Some(version.into()),
        }],
        container: Some(ContainerFacts {
            engine: Engine::Podman,
            id: "a".repeat(64),
            name: Some("actual-development-container".into()),
            image: Some("registry.example/custom:current".into()),
        }),
        available_programs: vec!["curl".into(), "tar".into()],
    }
}
fn environment_form(
    original: ProjectDefinition,
    fake: FakeWorkspaceEnvironment,
    cx: &mut TestAppContext,
) -> (Fixture, Arc<dyn nocterm_session::HostExec>) {
    let exec: Arc<dyn nocterm_session::HostExec> =
        Arc::new(nocterm_session::testing::FakeExec::default());
    let rig = Fixture::with_installation(
        vec![original],
        Arc::new(|_| Box::pin(async { Ok(()) })),
        |installed| {
            installed
                .with_local_host(Some(exec.clone()))
                .with_environment(Arc::new(fake))
        },
        Rc::new(nocterm_workspace::test_support::ProjectTerminalProbe::default()),
        cx,
    );
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.hover("project-row-01010101010101010101010101010101", cx);
        window.click("project-edit-01010101010101010101010101010101", cx);
    })
    .unwrap();
    cx.run_until_parked();
    (rig, exec)
}
fn choose_python(rig: &Fixture, cx: &mut TestAppContext) {
    cx.update_window(rig.window, |_, window, cx| {
        reveal_form(window, "project-language-python", cx);
        window.click("project-language-python", cx);
    })
    .unwrap();
}
#[gpui_kit::test]
fn settings_show_observed_container_os_and_tools_without_changing_the_catalog(
    cx: &mut TestAppContext,
) {
    let fake = FakeWorkspaceEnvironment::default();
    fake.respond_inspect(Ok(snapshot("3.13.9"))).unwrap();
    let original = container_definition(HostLocator::Local, Engine::Podman);
    let (rig, exec) = environment_form(original.clone(), fake.clone(), cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("project-environment-container")
                .label()
                .unwrap()
                .contains("actual-development-container")
        );
        assert_eq!(
            window.find("project-environment-container-summary").label(),
            Some("Podman · actual-development-container · aaaaaaaaaaaa")
        );
        assert!(
            window
                .find("project-environment-container")
                .label()
                .unwrap()
                .ends_with(&"a".repeat(64))
        );
        assert_eq!(
            window.find("project-environment-image").label(),
            Some("registry.example/custom:current")
        );
        assert_eq!(
            window.find("project-environment-os").label(),
            Some("Alpine Linux 3.22 · x86_64")
        );
        assert_eq!(
            window.find("project-installed-python").label(),
            Some("Installed: 3.13.9")
        );
        assert!(
            window.find("project-environment-install").bounds().bottom()
                < window.find("project-save").bounds().top()
        );
        assert!(window.find("project-save").bounds().bottom() <= window.viewport_size().height);
        assert_eq!(
            rig.installed.catalog(cx).get(original.id()),
            Some(&original)
        );
    })
    .unwrap();
    let calls = fake.calls().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(
        matches!(calls[0].source.execution(), EnvironmentExecution::Host(actual) if Arc::ptr_eq(actual, &exec))
    );
    assert_eq!(calls[0].source.target(), original.target());
}
#[gpui_kit::test]
fn metadata_save_never_installs_tools(cx: &mut TestAppContext) {
    let fake = FakeWorkspaceEnvironment::default();
    fake.respond_inspect(Ok(snapshot("3.13.9"))).unwrap();
    let (rig, _) = environment_form(definition(1), fake.clone(), cx);
    choose_python(&rig, cx);
    cx.update_window(rig.window, |_, window, cx| {
        reveal_form(window, "project-name", cx);
        fill(window, "project-name", "Metadata only", cx);
        window.click("project-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        fake.calls()
            .unwrap()
            .iter()
            .all(|call| call.selections.is_none())
    );
    cx.update(|cx| {
        assert_eq!(
            rig.installed
                .catalog(cx)
                .get(definition(1).id())
                .unwrap()
                .name(),
            "Metadata only"
        )
    });
}
#[gpui_kit::test]
fn explicit_install_preserves_the_workspace_and_replaces_observed_versions(
    cx: &mut TestAppContext,
) {
    let fake = FakeWorkspaceEnvironment::default();
    fake.respond_inspect(Ok(snapshot("3.12.8"))).unwrap();
    fake.respond_install(Ok(snapshot("3.13.9"))).unwrap();
    let original = definition(1);
    let (rig, _) = environment_form(original.clone(), fake.clone(), cx);
    choose_python(&rig, cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.click("project-environment-install", cx)
    })
    .unwrap();
    cx.run_until_parked();
    let calls = fake.calls().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[1].selections.as_ref().unwrap()[0].language(),
        ProgrammingLanguage::Python
    );
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("project-installed-python").label(),
            Some("Installed: 3.13.9")
        );
        assert_eq!(
            rig.installed.catalog(cx).get(original.id()),
            Some(&original)
        );
        assert!(window.try_find("project-name").is_some());
    })
    .unwrap();
}
#[gpui_kit::test]
fn failed_install_keeps_observed_versions_and_can_retry_without_recreating_workspace(
    cx: &mut TestAppContext,
) {
    let fake = FakeWorkspaceEnvironment::default();
    fake.respond_inspect(Ok(snapshot("3.12.8"))).unwrap();
    fake.respond_install(Err(EnvironmentError::ReadOnlyHome))
        .unwrap();
    fake.respond_install(Ok(snapshot("3.13.9"))).unwrap();
    let original = definition(1);
    let (rig, _) = environment_form(original.clone(), fake.clone(), cx);
    choose_python(&rig, cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.click("project-environment-install", cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("project-installed-python").label(),
            Some("Installed: 3.12.8")
        );
        assert!(window.try_find("project-environment-error").is_some());
        assert_eq!(
            rig.installed.catalog(cx).get(original.id()),
            Some(&original)
        );
        window.click("project-environment-install", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(fake.calls().unwrap().len(), 3);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("project-environment-error").is_none());
    })
    .unwrap();
}
#[gpui_kit::test]
fn disconnected_remote_environment_never_uses_the_injected_local_executor(cx: &mut TestAppContext) {
    let fake = FakeWorkspaceEnvironment::default();
    let original = container_definition(remote(Some("remote"), "remote.example"), Engine::Docker);
    let (rig, _) = environment_form(original, fake.clone(), cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("project-environment-connect").is_some());
        assert!(window.try_find("project-environment-install").is_none());
        assert!(
            window
                .find("project-environment-container")
                .label()
                .unwrap()
                .contains(&"a".repeat(64))
        );
    })
    .unwrap();
    assert!(fake.calls().unwrap().is_empty());
}
#[gpui_kit::test]
fn a_changed_saved_binding_refuses_tool_install_before_io(cx: &mut TestAppContext) {
    let fake = FakeWorkspaceEnvironment::default();
    fake.respond_inspect(Ok(snapshot("3.12.8"))).unwrap();
    let original = definition(1);
    let (rig, _) = environment_form(original.clone(), fake.clone(), cx);
    choose_python(&rig, cx);
    let mut changed = original.to_draft();
    changed.target = ProjectTarget::HostFolder {
        host: HostLocator::Local,
        root: RootPath::parse("/changed-target").unwrap(),
    };
    cx.update(|cx| {
        rig.model
            .save(changed.try_into().unwrap(), Some(original), cx)
    })
    .detach();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.click("project-environment-install", cx);
        window.render_frame(cx);
        assert!(
            window
                .find("project-environment-error")
                .label()
                .unwrap()
                .contains("changed")
        );
    })
    .unwrap();
    assert_eq!(fake.calls().unwrap().len(), 1);
}
#[gpui_kit::test]
fn container_row_keeps_its_engine_right_of_the_server_without_an_os_icon(cx: &mut TestAppContext) {
    let original = container_definition(remote(Some("remote"), "remote.example"), Engine::Docker);
    let rig = Fixture::loaded(vec![original], cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        let title = window
            .find("project-row-server-01010101010101010101010101010101")
            .bounds();
        assert!(
            window
                .try_find("project-row-server-icon-01010101010101010101010101010101")
                .is_none()
        );
        let badge = window.find("project-row-engine-01010101010101010101010101010101");
        assert_eq!(badge.label(), Some("Docker"));
        assert!(title.right() <= badge.bounds().left());
        assert_eq!(title.top(), badge.bounds().top());
    })
    .unwrap();
}

#[gpui_kit::test]
fn settings_use_the_original_workspace_surface_when_another_workspace_is_active(
    cx: &mut TestAppContext,
) {
    let host = remote(Some("original"), "original.example");
    let exec: Arc<dyn nocterm_session::HostExec> =
        Arc::new(nocterm_session::testing::FakeExec::default());
    let fake = FakeWorkspaceEnvironment::default();
    fake.respond_inspect(Ok(snapshot("3.13.9"))).unwrap();
    let terminals = nocterm_workspace::test_support::ProjectTerminalProbe::with_sessions(
        nocterm_workspace::SessionSpec {
            profile: Some("original".into()),
            options: Default::default(),
            title: "Workspace shell".into(),
            target: nocterm_session::Target::new("alice", "original.example", 22),
            auth: Default::default(),
            launch: None,
            credential: None,
            jumps: vec![],
        },
    )
    .with_executor(exec.clone());
    let rig = Fixture::with_installation(
        vec![on_host(1, host.clone()), on_host(2, host.clone())],
        Arc::new(|_| Box::pin(async { Ok(()) })),
        |installed| {
            installed
                .with_environment(Arc::new(fake.clone()))
                .with_terminal_resolver(Rc::new(terminals.clone()))
        },
        Rc::new(terminals.clone()),
        cx,
    );
    let _parent = rig.parent(host.clone(), "Original host", cx);
    cx.update_window(rig.window, |_, window, cx| {
        rig.workspace.update(cx, |workspace, cx| {
            rig.installed
                .open(definition(1).id(), workspace, window, cx)
                .unwrap();
            rig.installed
                .open(definition(2).id(), workspace, window, cx)
                .unwrap();
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.hover("project-row-01010101010101010101010101010101", cx);
        window.click("project-edit-01010101010101010101010101010101", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let calls = fake.calls().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].source.target(), on_host(1, host).target());
    assert!(
        matches!(calls[0].source.execution(), EnvironmentExecution::Scoped(actual) if Arc::ptr_eq(actual, &exec))
    );
}

#[gpui_kit::test]
fn requested_numeric_version_is_sent_only_when_install_is_pressed(cx: &mut TestAppContext) {
    let fake = FakeWorkspaceEnvironment::default();
    fake.respond_inspect(Ok(snapshot("3.12.8"))).unwrap();
    fake.respond_install(Ok(snapshot("3.13.9"))).unwrap();
    let (rig, _) = environment_form(definition(1), fake.clone(), cx);
    choose_python(&rig, cx);
    cx.update_window(rig.window, |_, window, cx| {
        super::creation::open_custom_version(window, "python", cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        reveal_form(window, "project-version-custom-python", cx);
        fill(window, "project-version-custom-python", "3.13.9", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(fake.calls().unwrap().len(), 1);
    cx.update_window(rig.window, |_, window, cx| {
        window.click("project-environment-install", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        fake.calls().unwrap()[1].selections.as_ref().unwrap()[0]
            .version()
            .as_str(),
        "3.13.9"
    );
}

#[gpui_kit::test]
fn closing_settings_cancels_pending_install_and_keeps_the_saved_workspace(cx: &mut TestAppContext) {
    let fake = FakeWorkspaceEnvironment::default();
    fake.respond_inspect(Ok(snapshot("3.12.8"))).unwrap();
    let (release, wait) = async_channel::bounded::<()>(1);
    fake.install_after(
        Box::pin(async move {
            let _ = wait.recv().await;
        }),
        Ok(snapshot("3.13.9")),
    )
    .unwrap();
    let original = definition(1);
    let (rig, _) = environment_form(original.clone(), fake.clone(), cx);
    choose_python(&rig, cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.click("project-environment-install", cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-cancel", cx);
    })
    .unwrap();
    assert!(fake.calls().unwrap()[1].cancel.is_cancelled());
    release.try_send(()).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            rig.installed.catalog(cx).get(original.id()),
            Some(&original)
        )
    });
}
