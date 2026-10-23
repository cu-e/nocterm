use super::{support::*, *};

#[gpui_kit::test]
fn creation_keeps_the_original_server_and_folder_after_another_connection_is_selected(
    cx: &mut TestAppContext,
) {
    let rig = Fixture::loaded(vec![], cx);
    let a = remote(Some("profile-a"), "a.example");
    let _parent_a = rig.parent(a.clone(), "Server A", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Original project", cx);
        assert!(window.try_find("project-form-server").is_some());
    })
    .unwrap();
    let _parent_b = rig.parent(remote(Some("profile-b"), "b.example"), "Server B", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        create_workspace(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = rig.installed.catalog(cx).iter().next().unwrap();
        assert_eq!(saved.name(), "Original project");
        assert_eq!(saved.host_folder().unwrap().0, &a);
        assert_eq!(
            saved.host_folder().unwrap().1.as_str(),
            "/srv/original-folder"
        );
        assert_eq!(rig.workspace.read(cx).items(cx).count(), 2);
    });
}

#[gpui_kit::test]
fn creation_uses_the_actual_direct_destination_and_only_name_and_folder_controls(
    cx: &mut TestAppContext,
) {
    let rig = Fixture::loaded(vec![], cx);
    let host = remote(None, "direct.example");
    let _parent = rig.parent(host.clone(), "Temporary connection", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Folder source", cx);
        fill(window, "project-path", "/srv/exact-folder", cx);
        assert!(window.try_find("project-writable").is_none());
        assert!(window.try_find("project-execution").is_none());
        create_workspace(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = rig.installed.catalog(cx).iter().next().unwrap();
        assert_eq!(saved.host_folder().unwrap().0, &host);
        assert_eq!(saved.host_folder().unwrap().1.as_str(), "/srv/exact-folder");
    });
}

#[gpui_kit::test]
fn a_captured_folder_draft_can_be_saved_after_its_original_parent_disconnects(
    cx: &mut TestAppContext,
) {
    let rig = Fixture::loaded(vec![], cx);
    let host = remote(Some("a"), "a.example");
    let parent = rig.parent(host.clone(), "Server A", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Saved folder", cx);
    })
    .unwrap();
    cx.update(|cx| {
        parent
            .item
            .update(cx, |item, cx| item.set_connected(false, cx).unwrap())
    });
    let _other = rig.parent(remote(Some("b"), "b.example"), "Server B", cx);
    cx.update_window(rig.window, |_, window, cx| create_workspace(window, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = rig.installed.catalog(cx).iter().next().unwrap();
        assert_eq!(saved.host_folder().unwrap().0, &host);
        assert_eq!(saved.name(), "Saved folder");
    });
}

#[gpui_kit::test]
fn a_failed_save_keeps_the_literal_draft_for_a_successful_retry(cx: &mut TestAppContext) {
    let (writer, release, writes) = nocterm_store::test_support::gated_writer().into_parts();
    let rig = Fixture::new(vec![], writer, cx);
    let host = remote(Some("a"), "a.example");
    let _parent = rig.parent(host.clone(), "Server A", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Keep this draft", cx);
        fill(window, "project-path", "/srv/draft", cx);
        create_workspace(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(writes.lock().unwrap().len(), 1);
    release
        .send(Err(nocterm_store::StoreError::backend(
            std::io::Error::other("disk full"),
        )))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("project-form-error").is_some());
        assert_eq!(rig.installed.catalog(cx).iter().count(), 0);
        create_workspace(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = rig.installed.catalog(cx).iter().next().unwrap();
        assert_eq!(saved.name(), "Keep this draft");
        assert_eq!(saved.host_folder().unwrap().1.as_str(), "/srv/draft");
        assert_eq!(saved.host_folder().unwrap().0, &host);
    });
}

#[gpui_kit::test]
fn the_filesystem_root_is_a_valid_folder_workspace(cx: &mut TestAppContext) {
    let rig = Fixture::loaded(vec![], cx);
    let host = remote(None, "server.example");
    let _parent = rig.parent(host.clone(), "Server", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Root folder", cx);
        fill(window, "project-path", "/", cx);
        create_workspace(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = rig.installed.catalog(cx).iter().next().unwrap();
        assert_eq!(saved.host_folder().unwrap().1.as_str(), "/");
        assert_eq!(saved.host_folder().unwrap().0, &host);
    });
}

#[gpui_kit::test]
fn invalid_relative_folder_does_not_publish_or_replace_the_draft(cx: &mut TestAppContext) {
    let rig = Fixture::loaded(vec![], cx);
    let _parent = rig.parent(remote(None, "a.example"), "Server A", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Valid name", cx);
        fill(window, "project-path", "relative/path", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        create_workspace(window, cx);
        window.render_frame(cx);
        assert!(window.try_find("project-form-error").is_some());
        assert_eq!(rig.installed.catalog(cx).iter().count(), 0);
        fill(window, "project-path", "/srv/fixed", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("project-form-error").is_none());
        create_workspace(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = rig.installed.catalog(cx).iter().next().unwrap();
        assert_eq!(saved.name(), "Valid name");
        assert_eq!(saved.host_folder().unwrap().1.as_str(), "/srv/fixed");
    });
}

#[gpui_kit::test]
fn editing_the_name_clears_its_error_before_the_draft_is_saved(cx: &mut TestAppContext) {
    let rig = Fixture::loaded(vec![], cx);
    let _parent = rig.parent(remote(None, "a.example"), "Server A", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        create_workspace(window, cx);
        window.render_frame(cx);
        assert!(window.try_find("project-form-error").is_some());
        fill(window, "project-name", "Corrected name", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("project-form-error").is_none());
        assert_eq!(rig.installed.catalog(cx).iter().count(), 0);
        create_workspace(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = rig.installed.catalog(cx).iter().next().unwrap();
        assert_eq!(saved.name(), "Corrected name");
        assert_eq!(
            saved.host_folder().unwrap().1.as_str(),
            "/srv/original-folder"
        );
    });
}

#[gpui_kit::test]
fn creation_connection_choices_reveal_only_the_selected_modes_controls(cx: &mut TestAppContext) {
    let rig = Fixture::loaded(vec![], cx);
    let _parent = rig.parent(remote(None, "server.example"), "Server", cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-mode-direct").is_some());
        assert!(window.try_find("project-mode-existing").is_some());
        assert!(window.try_find("project-mode-new").is_some());
        assert!(window.try_find("project-container-refresh").is_none());
        assert!(window.try_find("project-engine-Docker").is_none());
        wizard_field(window, "project-mode-existing", cx);
        window.click("project-mode-existing", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-container-refresh").is_some());
        assert!(window.try_find("project-engine-Docker").is_none());
        wizard_field(window, "project-mode-new", cx);
        window.click("project-mode-new", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-container-refresh").is_none());
        assert!(window.try_find("project-engine-Docker").is_some());
        assert!(window.try_find("project-engine-Podman").is_some());
        wizard_field(window, "project-mode-direct", cx);
        window.click("project-mode-direct", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-engine-Docker").is_none());
        assert!(window.try_find("project-path").is_none());
        window.click("project-next", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-path").is_some());
    })
    .unwrap();
}

fn preparation_form(
    cx: &mut TestAppContext,
) -> (
    Fixture,
    nocterm_project_providers::testing::FakeProjectPreparation,
) {
    let preparation = nocterm_project_providers::testing::FakeProjectPreparation::default();
    preparation
        .respond(Ok(nocterm_project_providers::PreparedProject {
            target: definition(20).target().clone(),
            owned: None,
        }))
        .unwrap();
    let rig = Fixture::preparing(preparation.clone(), cx);
    let parent = rig.parent(remote(None, "server.example"), "Server", cx);
    cx.update(|cx| {
        parent.item.update(cx, |item, cx| {
            item.set_executor(Arc::new(nocterm_session::testing::FakeExec::default()), cx);
        })
    });
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Container workspace", cx);
        wizard_field(window, "project-mode-new", cx);
        window.click("project-mode-new", cx);
        window.render_frame(cx);
    })
    .unwrap();
    (rig, preparation)
}

#[gpui_kit::test]
fn new_container_keeps_default_python_when_another_language_is_selected(cx: &mut TestAppContext) {
    use nocterm_project_providers::{
        LanguageSelection, ProgrammingLanguage, SetupMode, WorkspaceImage,
    };
    let (rig, preparation) = preparation_form(cx);
    cx.update_window(rig.window, |_, window, cx| {
        wizard_field(window, "project-language-rust", cx);
        window.click("project-language-rust", cx);
        wizard_field(window, "project-engine-Podman", cx);
        window.click("project-engine-Podman", cx);
        create_workspace(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let requests = preparation.requests().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(matches!(&requests[0].mode, SetupMode::Create {
        engine: nocterm_containers::Engine::Podman,
        image: WorkspaceImage::Ubuntu,
        languages,
        root: Some(root),
    } if root.as_str() == "/home" && languages == &[LanguageSelection::default_for(ProgrammingLanguage::Python), LanguageSelection::default_for(ProgrammingLanguage::Rust)]));
    cx.update(|cx| assert_eq!(rig.installed.catalog(cx).len(), 1));
}

#[gpui_kit::test]
fn custom_image_uses_the_literal_reference_and_selected_languages(cx: &mut TestAppContext) {
    use nocterm_project_providers::{SetupMode, WorkspaceImage};
    let (rig, preparation) = preparation_form(cx);
    cx.update_window(rig.window, |_, window, cx| {
        wizard_field(window, "project-language-rust", cx);
        window.click("project-language-rust", cx);
        wizard_field(window, "project-image-Custom image", cx);
        window.click("project-image-Custom image", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-custom-image").is_some());
        fill(
            window,
            "project-custom-image",
            "registry.example/team/image:stable",
            cx,
        );
        reveal_form(window, "project-language-java", cx);
        wizard_field(window, "project-language-java", cx);
        window.click("project-language-java", cx);
        create_workspace(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let requests = preparation.requests().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(matches!(&requests[0].mode, SetupMode::Create {
        image: WorkspaceImage::Custom(reference), languages, ..
    } if reference == "registry.example/team/image:stable" && languages.len() == 3));
    cx.update(|cx| assert_eq!(rig.installed.catalog(cx).len(), 1));
}

#[gpui_kit::test]
fn an_empty_workspace_creates_a_local_folder_with_shared_home_navigation(cx: &mut TestAppContext) {
    let preparation = nocterm_project_providers::testing::FakeProjectPreparation::default();
    preparation
        .respond(Ok(nocterm_project_providers::PreparedProject {
            target: ProjectTarget::HostFolder {
                host: HostLocator::Local,
                root: RootPath::parse("/home/alice").unwrap(),
            },
            owned: None,
        }))
        .unwrap();
    let (rig, exec, _) = Fixture::local(preparation.clone(), cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("project-context").label(),
            Some("This computer")
        );
        window.dispatch_action(Box::new(NewProject), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("project-mode-existing").is_some());
        assert!(window.try_find("project-mode-new").is_some());
        fill(window, "project-name", "Local folder", cx);
        create_workspace(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let saved = rig.installed.catalog(cx).iter().next().unwrap();
        assert_eq!(saved.host_folder().unwrap().0, &HostLocator::Local);
        assert_eq!(saved.host_folder().unwrap().1.as_str(), "/home/alice");
        assert_eq!(rig.workspace.read(cx).items(cx).count(), 0);
    });
    assert!(Arc::ptr_eq(&preparation.discovered().unwrap()[0], &exec));
}

#[gpui_kit::test]
fn local_container_creation_keeps_the_injected_machine_executor(cx: &mut TestAppContext) {
    let preparation = nocterm_project_providers::testing::FakeProjectPreparation::default();
    preparation
        .respond(Ok(nocterm_project_providers::PreparedProject {
            target: definition(20).target().clone(),
            owned: None,
        }))
        .unwrap();
    let (rig, exec, _) = Fixture::local(preparation.clone(), cx);
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("project-new", cx);
        window.render_frame(cx);
        fill(window, "project-name", "Local container", cx);
        wizard_field(window, "project-mode-new", cx);
        window.click("project-mode-new", cx);
        window.render_frame(cx);
        wizard_field(window, "project-engine-Podman", cx);
        window.click("project-engine-Podman", cx);
    })
    .unwrap();
    let _remote = rig.parent(remote(None, "selected-later.example"), "Another server", cx);
    cx.update_window(rig.window, |_, window, cx| create_workspace(window, cx))
        .unwrap();
    cx.run_until_parked();
    let requests = preparation.requests().unwrap();
    assert_eq!(requests[0].host, HostLocator::Local);
    assert!(matches!(
        requests[0].mode,
        nocterm_project_providers::SetupMode::Create {
            engine: nocterm_containers::Engine::Podman,
            ..
        }
    ));
    assert!(Arc::ptr_eq(&preparation.prepared().unwrap()[0], &exec));
    cx.update(|cx| assert_eq!(rig.installed.catalog(cx).len(), 1));
}

#[gpui_kit::test]
fn a_selected_disconnected_remote_refuses_local_creation(cx: &mut TestAppContext) {
    let preparation = nocterm_project_providers::testing::FakeProjectPreparation::default();
    let (rig, _, fs) = Fixture::local(preparation.clone(), cx);
    let parent = rig.parent(remote(None, "offline.example"), "Offline server", cx);
    cx.update(|cx| {
        parent
            .item
            .update(cx, |item, cx| item.set_connected(false, cx).unwrap())
    });
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("project-context").label(),
            Some("Offline server")
        );
        window.click("project-new", cx);
        window.dispatch_action(Box::new(NewProject), cx);
        window.render_frame(cx);
        assert!(window.try_find("project-name").is_none());
    })
    .unwrap();
    assert!(preparation.discovered().unwrap().is_empty());
    assert!(preparation.requests().unwrap().is_empty());
    assert_eq!(fs.activity().requests, 0);
}

#[gpui_kit::test]
fn a_stale_remote_capture_cannot_redirect_creation_to_this_computer(cx: &mut TestAppContext) {
    use nocterm_workspace_ports::{
        ProjectParentDescriptor, ProjectParentError, testing::StaticProjectParent,
    };
    let preparation = nocterm_project_providers::testing::FakeProjectPreparation::default();
    let (rig, _, fs) = Fixture::local(preparation.clone(), cx);
    let host = remote(None, "stale.example");
    let parent = rig.parent(host.clone(), "Stale server", cx);
    cx.update(|cx| {
        parent.item.update(cx, |item, cx| {
            item.set_project_parent(
                Rc::new(
                    StaticProjectParent::new(
                        ProjectParentDescriptor::new(host, "Stale server".into()).unwrap(),
                        None,
                    )
                    .with_refusal(ProjectParentError::StaleSession),
                ),
                cx,
            );
        });
    });
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(NewProject), cx);
        window.render_frame(cx);
        assert!(window.try_find("project-name").is_none());
    })
    .unwrap();
    assert!(preparation.discovered().unwrap().is_empty());
    assert!(preparation.requests().unwrap().is_empty());
    assert_eq!(fs.activity().requests, 0);
}

pub(super) fn open_custom_version(window: &mut gpui_kit::Window, language: &str, cx: &mut App) {
    reveal_form(window, &format!("project-version-{language}"), cx);
    window
        .within(format!("project-version-{language}"))
        .click("input", cx);
    window.press("down", cx);
    window.press("enter", cx);
}
fn custom_version(handle: AnyWindowHandle, language: &str, version: &str, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        open_custom_version(window, language, cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        reveal_form(window, &format!("project-version-custom-{language}"), cx);
        fill(
            window,
            &format!("project-version-custom-{language}"),
            version,
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn selected_language_versions_survive_toggles_and_reach_preparation(cx: &mut TestAppContext) {
    use nocterm_project_providers::{LanguageSelection, ProgrammingLanguage, SetupMode};
    let (rig, preparation) = preparation_form(cx);
    custom_version(rig.window, "python", "3.13", cx);
    for (mark, version) in [
        ("rust", "1.90.0"),
        ("bun", "1.3.3"),
        ("nodejs", "22"),
        ("java", "21"),
    ] {
        cx.update_window(rig.window, |_, window, cx| {
            reveal_form(window, &format!("project-language-{mark}"), cx);
            window.click(format!("project-language-{mark}"), cx);
        })
        .unwrap();
        custom_version(rig.window, mark, version, cx);
    }
    cx.update_window(rig.window, |_, window, cx| {
        reveal_form(window, "project-language-rust", cx);
        wizard_field(window, "project-language-rust", cx);
        window.click("project-language-rust", cx);
        wizard_field(window, "project-language-rust", cx);
        window.click("project-language-rust", cx);
        window.render_frame(cx);
        assert_eq!(
            window
                .within("project-version-custom-rust")
                .find("input")
                .value(),
            Some("1.90.0")
        );
        reveal_form(window, "project-image-Custom image", cx);
        wizard_field(window, "project-image-Custom image", cx);
        window.click("project-image-Custom image", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        reveal_form(window, "project-image-Ubuntu 24.04", cx);
        wizard_field(window, "project-image-Ubuntu 24.04", cx);
        window.click("project-image-Ubuntu 24.04", cx);
        create_workspace(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let expected: Vec<_> = [
        (ProgrammingLanguage::Python, "3.13"),
        (ProgrammingLanguage::Rust, "1.90.0"),
        (ProgrammingLanguage::Bun, "1.3.3"),
        (ProgrammingLanguage::Node, "22"),
        (ProgrammingLanguage::Java, "21"),
    ]
    .into_iter()
    .map(|(language, version)| LanguageSelection::parse(language, version).unwrap())
    .collect();
    assert!(
        matches!(&preparation.requests().unwrap()[0].mode, SetupMode::Create { languages, .. } if languages == &expected)
    );
}

#[gpui_kit::test]
fn invalid_custom_language_version_is_visible_and_prevents_preparation(cx: &mut TestAppContext) {
    let (rig, preparation) = preparation_form(cx);
    custom_version(rig.window, "python", "3.13; echo no", cx);
    cx.update_window(rig.window, |_, window, cx| {
        create_workspace(window, cx);
        window.render_frame(cx);
        assert!(
            window
                .find("project-form-error")
                .label()
                .unwrap()
                .contains("version")
        );
        assert!(window.try_find("project-languages").is_some());
    })
    .unwrap();
    cx.run_until_parked();
    assert!(preparation.requests().unwrap().is_empty());
}

#[gpui_kit::test]
fn pull_bytes_render_left_of_the_create_action_without_changing_its_label(cx: &mut TestAppContext) {
    use nocterm_project_providers::{SetupPhase, SetupUpdate, TransferProgress};
    let (rig, preparation) = preparation_form(cx);
    preparation
        .progress(vec![SetupUpdate {
            phase: SetupPhase::PullingImage,
            transfer: Some(TransferProgress::new(Some("Layer abc"), 25, Some(100))),
        }])
        .unwrap();
    let (release, wait) = async_channel::bounded::<()>(1);
    preparation
        .suspend(Box::pin(async move {
            let _ = wait.recv().await;
        }))
        .unwrap();
    cx.update_window(rig.window, |_, window, cx| create_workspace(window, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        let progress = window.find("project-progress");
        assert_eq!(progress.label(), Some("Layer abc: 25 / 100 bytes (25%)"));
        assert!(progress.bounds().right() <= window.find("project-cancel").bounds().left());
        assert!(window.find("project-save").bounds().bottom() <= window.viewport_size().height);
    })
    .unwrap();
    release.try_send(()).unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn unknown_pull_totals_show_the_latest_phase_and_cancel_clears_the_operation(
    cx: &mut TestAppContext,
) {
    use nocterm_project_providers::{SetupPhase, SetupUpdate, TransferProgress};
    let (rig, preparation) = preparation_form(cx);
    preparation
        .progress(vec![
            SetupUpdate {
                phase: SetupPhase::PullingImage,
                transfer: Some(TransferProgress::new(None, 2000, None)),
            },
            SetupUpdate::phase(SetupPhase::StartingContainer),
        ])
        .unwrap();
    let (release, wait) = async_channel::bounded::<()>(1);
    preparation
        .suspend(Box::pin(async move {
            let _ = wait.recv().await;
        }))
        .unwrap();
    cx.update_window(rig.window, |_, window, cx| create_workspace(window, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(rig.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("project-progress").label(),
            Some("Starting container…")
        );
        window.click("project-cancel", cx);
        window.render_frame(cx);
        assert!(window.try_find("project-progress").is_none());
    })
    .unwrap();
    release.try_send(()).unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(rig.installed.catalog(cx).is_empty()));
}
