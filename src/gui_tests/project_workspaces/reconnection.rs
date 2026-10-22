//! Normal authentication stays inside the workspace terminal.
use super::*;

#[gpui_kit::test]
fn an_attached_server_opens_in_the_original_hidden_workspace_with_its_saved_route(
    cx: &mut TestAppContext,
) {
    let f = ProjectFixture::new(HostLocator::Local, cx);
    let _initial = f.open_project(cx);
    let source = cx.update(|cx| f.workspace.read(cx).context_source(cx));
    let mut profile = saved_server();
    profile.launch = Some(nocterm_session::ShellLaunch {
        program: Some("/bin/bash".into()),
        args: vec!["--norc".into()],
        env: [("ATTACHED_SERVER".into(), "literal ; $(false)".into())].into(),
        cwd: Some("/srv/other-project".into()),
        integration: false,
    });
    f.save_server(profile.clone(), cx);
    let (general, _general_driver) = f.remote_parent(spec("visible.example"), cx);
    let driver = f.transport.queue(None, None);
    let item = cx
        .update_window(f.window, |_, window, cx| {
            let opener = f.workspace.read(cx).chat_connection_opener().unwrap();
            opener
                .open(
                    &profile.id.to_string(),
                    source,
                    &f.workspace.downgrade(),
                    window,
                    cx,
                )
                .unwrap()
        })
        .unwrap();
    connected(&driver, cx);
    cx.update(|cx| {
        let workspace = f.workspace.read(cx);
        assert_eq!(workspace.selected_context(cx), ContextId::General);
        assert_eq!(workspace.active_item(cx).unwrap().item_id(), general);
        let entries = workspace.context_terminals(source, cx).unwrap();
        assert!(
            entries
                .iter()
                .any(|entry| entry.item == item && entry.background)
        );
        assert!(
            !workspace
                .terminals(cx)
                .iter()
                .any(|entry| entry.item == item)
        );
        let requests = f.transport.requests();
        let request = requests.last().unwrap();
        assert_eq!(request.target, profile.target);
        assert_eq!(request.launch, profile.launch.unwrap());
        assert_eq!(request.jumps.len(), 2);
        assert_eq!(request.jumps[0].target.host, "first.example");
        assert_eq!(request.jumps[1].target.host, "second.example");
    });
}

#[gpui_kit::test]
fn an_attached_server_refuses_foreign_windows_and_closed_workspaces_before_connecting(
    cx: &mut TestAppContext,
) {
    use nocterm_workspace::{ChatConnectionOpenError, ContextSelectionError};
    let f = ProjectFixture::new(HostLocator::Local, cx);
    let _initial = f.open_project(cx);
    let profile = saved_server();
    f.save_server(profile.clone(), cx);
    let (source, opener, held_outer) = cx.update(|cx| {
        let workspace = f.workspace.read(cx);
        (
            workspace.context_source(cx),
            workspace.chat_connection_opener().unwrap(),
            workspace.active_item(cx).unwrap().view(),
        )
    });
    let other = ProjectFixture::new(HostLocator::Local, cx);
    cx.update_window(other.window, |_, window, cx| {
        assert!(matches!(
            opener.open(
                &profile.id.to_string(),
                source,
                &f.workspace.downgrade(),
                window,
                cx
            ),
            Err(ChatConnectionOpenError::Destination(
                ContextSelectionError::StaleWindow
            ))
        ));
    })
    .unwrap();
    cx.update_window(f.window, |_, window, cx| {
        f.workspace
            .update(cx, |workspace, cx| workspace.close_item(0, window, cx));
        assert!(matches!(
            opener.open(
                &profile.id.to_string(),
                source,
                &f.workspace.downgrade(),
                window,
                cx
            ),
            Err(ChatConnectionOpenError::Destination(
                ContextSelectionError::StaleSurface
            ))
        ));
        assert_eq!(f.transport.requests().len(), 1);
    })
    .unwrap();
    drop(held_outer);
}

#[gpui_kit::test]
fn an_attached_server_refuses_missing_profiles_and_broken_jump_routes_before_connecting(
    cx: &mut TestAppContext,
) {
    use nocterm_connections_model::{jump_route::JumpHostEntry, profiles::ProfileId};
    use nocterm_workspace::ChatConnectionOpenError;
    let f = ProjectFixture::new(HostLocator::Local, cx);
    let _initial = f.open_project(cx);
    let mut profile = saved_server();
    profile.jump_hosts = vec![JumpHostEntry::Profile(ProfileId::from_bytes([8; 16]))];
    f.save_server(profile.clone(), cx);
    cx.update_window(f.window, |_, window, cx| {
        let workspace = f.workspace.read(cx);
        let source = workspace.context_source(cx);
        let opener = workspace.chat_connection_opener().unwrap();
        assert!(matches!(
            opener.open("missing", source, &f.workspace.downgrade(), window, cx),
            Err(ChatConnectionOpenError::MissingProfile)
        ));
        assert!(matches!(
            opener.open(
                &profile.id.to_string(),
                source,
                &f.workspace.downgrade(),
                window,
                cx
            ),
            Err(ChatConnectionOpenError::InvalidRoute(_))
        ));
        assert_eq!(f.transport.requests().len(), 1);
    })
    .unwrap();
}

fn saved_server() -> nocterm_connections_model::profiles::Profile {
    use nocterm_connections_model::{
        jump_route::JumpHostEntry,
        profiles::{Profile, ProfileId},
    };
    Profile {
        id: ProfileId::from_bytes([9; 16]),
        name: "Original SSH server".into(),
        description: String::new(),
        target: nocterm_session::Target::new("test", "destination.example", 22),
        auth: nocterm_session::Auth::Password,
        group: None,
        credential: None,
        launch: None,
        icon: Some("ubuntu".into()),
        icon_color: Some("#E95420".into()),
        country: None,
        options: Default::default(),
        jump_hosts: vec![
            JumpHostEntry::Address(nocterm_session::Target::new("first", "first.example", 2222)),
            JumpHostEntry::Address(nocterm_session::Target::new(
                "second",
                "second.example",
                2200,
            )),
        ],
    }
}
fn saved_host(profile: &nocterm_connections_model::profiles::Profile) -> HostLocator {
    let spec = nocterm_connections::spec_for_profile(profile, &Default::default()).unwrap();
    HostLocator::Ssh {
        profile: profile.id.to_string(),
        target: profile.target.clone(),
        via: spec.address().via,
    }
}
fn assert_only_workspace(f: &ProjectFixture, cx: &mut TestAppContext) {
    cx.update(|cx| {
        let workspace = f.workspace.read(cx);
        assert_eq!(
            workspace.selected_context(cx),
            ContextId::Project(ProjectId::from_bytes([1; 16]))
        );
        assert_eq!(workspace.items(cx).count(), 1);
        assert!(
            workspace
                .context_terminals(workspace.general_surface_source(cx), cx)
                .unwrap()
                .is_empty()
        );
        assert_eq!(workspace.terminals(cx).len(), 1);
    });
}
fn secret_prompt(
    driver: &SessionDriver,
    request: SecretRequest,
    cx: &mut TestAppContext,
) -> futures::channel::oneshot::Receiver<Option<Secret>> {
    let (reply, answer) = Reply::channel();
    assert!(
        driver
            .emit(Event::Prompt(Prompt::Secret {
                request,
                reply,
                hop: None
            }))
            .now_or_never()
            .unwrap()
    );
    cx.run_until_parked();
    answer
}

#[gpui_kit::test]
fn a_local_workspace_connects_without_opening_an_ordinary_server_tab(cx: &mut TestAppContext) {
    let f = ProjectFixture::new(HostLocator::Local, cx);
    let driver = f.transport.queue(None, None);
    f.show_project(cx);
    assert_eq!(f.transport.requests().len(), 1);
    assert_eq!(
        f.transport.requests()[0].launch.cwd.as_deref(),
        Some("/home/test/project")
    );
    connected(&driver, cx);
    assert_only_workspace(&f, cx);
}

#[gpui_kit::test]
fn a_saved_workspace_authenticates_its_jump_route_in_its_inner_terminal(cx: &mut TestAppContext) {
    let profile = saved_server();
    let f = ProjectFixture::new(saved_host(&profile), cx);
    f.save_server(profile.clone(), cx);
    let driver = f.transport.queue(None, None);
    f.show_project(cx);
    assert_eq!(f.transport.requests().len(), 1);
    assert_eq!(f.transport.requests()[0].jumps.len(), 2);
    assert_eq!(f.transport.requests()[0].target, profile.target);
    let (reply, answer) = Reply::channel();
    assert!(
        driver
            .emit(Event::Prompt(Prompt::UnknownHostKey {
                host: "first.example".into(),
                algorithm: "ssh-ed25519".into(),
                fingerprint: "SHA256:workspace-test".into(),
                hop: Some(nocterm_session::PromptHop {
                    index: 0,
                    target: f.transport.requests()[0].jumps[0].target.clone()
                }),
                reply,
            }))
            .now_or_never()
            .unwrap()
    );
    cx.run_until_parked();
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("host-key-once").visible());
        assert!(window.try_find("project-connect").is_none());
        window.click("host-key-once", cx);
    })
    .unwrap();
    assert_eq!(
        answer.now_or_never().unwrap().unwrap(),
        nocterm_session::HostKeyDecision::AcceptOnce
    );
    connected(&driver, cx);
    assert_only_workspace(&f, cx);
}

#[gpui_kit::test]
fn a_workspace_password_prompt_is_answered_inside_the_workspace(cx: &mut TestAppContext) {
    let profile = saved_server();
    let f = ProjectFixture::new(saved_host(&profile), cx);
    f.save_server(profile.clone(), cx);
    let driver = f.transport.queue(None, None);
    f.show_project(cx);
    let answer = secret_prompt(
        &driver,
        SecretRequest::Password {
            target: profile.target,
            retry: false,
        },
        cx,
    );
    assert_only_workspace(&f, cx);
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("secret-input").visible());
        window.click("secret-input", cx);
        window.input("workspace test password", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        answer.now_or_never().unwrap().unwrap().unwrap().expose(),
        "workspace test password"
    );
    connected(&driver, cx);
    assert_only_workspace(&f, cx);
}

#[gpui_kit::test]
fn a_workspace_uses_its_saved_vault_credential_after_unlock(cx: &mut TestAppContext) {
    let mut profile = saved_server();
    let f = ProjectFixture::new(saved_host(&profile), cx);
    let request = SecretRequest::Password {
        target: profile.target.clone(),
        retry: false,
    };
    profile.credential = Some(
        f.credentials
            .insert(
                "workspace server".into(),
                &request,
                Secret::new("saved test password"),
            )
            .unwrap(),
    );
    f.save_server(profile, cx);
    f.credentials.set_available(false);
    let driver = f.transport.queue(None, None);
    f.show_project(cx);
    let answer = secret_prompt(&driver, request, cx);
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("auth-open-vault").visible());
    })
    .unwrap();
    assert_only_workspace(&f, cx);
    f.credentials.set_available(true);
    cx.run_until_parked();
    assert_eq!(
        answer.now_or_never().unwrap().unwrap().unwrap().expose(),
        "saved test password"
    );
    connected(&driver, cx);
    assert_only_workspace(&f, cx);
}

#[gpui_kit::test]
fn opening_a_saved_workspace_refuses_a_changed_jump_route_before_any_connection(
    cx: &mut TestAppContext,
) {
    let mut profile = saved_server();
    let f = ProjectFixture::new(saved_host(&profile), cx);
    profile.jump_hosts.reverse();
    f.save_server(profile, cx);
    f.show_project(cx);
    assert!(f.transport.requests().is_empty());
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("project-unavailable").visible());
        assert!(window.try_find("project-connect").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn workspace_header_and_sidebar_name_the_server_instead_of_the_workspace(cx: &mut TestAppContext) {
    let profile = saved_server();
    let f = ProjectFixture::new(saved_host(&profile), cx);
    f.save_server(profile, cx);
    let driver = f.transport.queue(None, None);
    f.show_project(cx);
    connected(&driver, cx);
    cx.update_window(f.window, |_, window, cx| {
        window.dispatch_action(Box::new(nocterm_projects::ToggleProjects), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("project-context").label(),
            Some("Original SSH server")
        );
        assert_eq!(
            window.find("project-server").label(),
            Some("Original SSH server")
        );
        assert!(window.find("project-server-icon").visible());
        assert!(window.find("project-path").visible());
    })
    .unwrap();
}
