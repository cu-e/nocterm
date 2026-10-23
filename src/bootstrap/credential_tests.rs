//! A refused connection history write keeps the authenticated vault credential.
use super::*;
use futures::executor::block_on;
use gpui_kit::AppContext as _;
use nocterm_session::{
    Auth, CredentialStore as _, Event, Prompt, Reply, Secret, SecretRequest, Target,
    testing::ScriptedTransport,
};

fn history_profile(target: Target) -> nocterm_connections_model::profiles::Profile {
    nocterm_connections_model::profiles::Profile {
        id: nocterm_connections_model::profiles::ProfileId::from_bytes([7; 16]),
        name: "History".into(),
        description: String::new(),
        options: Default::default(),
        target,
        auth: Auth::Password,
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
        group: None,
        jump_hosts: Vec::new(),
    }
}
fn association_fixture(
    profile: nocterm_connections_model::profiles::Profile,
    file: std::path::PathBuf,
    cx: &mut gpui_kit::TestAppContext,
) -> (
    gpui_kit::Entity<nocterm_terminal::Terminal>,
    Arc<dyn VaultAccess>,
    gpui_kit::Entity<nocterm_connections::Connections>,
    nocterm_session::SessionDriver,
) {
    let mut profiles = nocterm_connections_model::profiles::Profiles::default();
    profiles.upsert(profile.clone());
    let transport = Arc::new(ScriptedTransport::default());
    let driver = transport.queue(None, None);
    let (terminal, service, connections) = cx.update(|cx| {
        gpui_kit::init(cx);
        let ready = nocterm_ui::init(
            DesignTokens::builtin(),
            SettingsConfig::in_memory(Default::default()).install(cx),
            cx,
        );
        let connections = cx.new(|_| {
            nocterm_connections::Connections::with_backends(
                profiles,
                Default::default(),
                Arc::new(|_| {
                    Box::pin(async {
                        Err(nocterm_store::StoreError::backend(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "history refused",
                        )))
                    })
                }),
                Arc::new(|_| Box::pin(async { Ok(()) })),
            )
        });
        let opened = open_vault(
            VaultSetup {
                file,
                device_unlock: false,
            },
            &ready,
            Some(connections.clone()),
            cx,
        )
        .unwrap();
        let service = opened.installed.service().clone();
        let installed = nocterm_terminal::install(
            nocterm_terminal::Deps::new(
                transport,
                nocterm_terminal::LocalSessions::disabled("/bin/bash".into()),
            )
            .with_credentials(opened.credentials),
            &ready,
            cx,
        );
        let terminal = cx.new(|cx| {
            nocterm_terminal::Terminal::new(
                installed,
                nocterm_workspace::SessionSpec {
                    profile: Some(profile.id.to_string().into()),
                    options: Default::default(),
                    title: "History".into(),
                    target: profile.target.clone(),
                    auth: Auth::Password,
                    launch: None,
                    credential: None,
                    jumps: Vec::new(),
                },
                cx,
            )
        });
        (terminal, service, connections)
    });
    (terminal, service, connections, driver)
}
#[gpui_kit::test]
async fn actual_connection_callback_refusal_keeps_the_authenticated_vault_credential(
    cx: &mut gpui_kit::TestAppContext,
) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let target = Target::new("user", "host", 22);
    let profile = history_profile(target.clone());
    let (terminal, service, connections, driver) =
        association_fixture(profile.clone(), directory.path().join("vault"), cx);
    block_on(service.create(Secret::new("long master password"))).unwrap();
    let request = SecretRequest::Password {
        target,
        retry: false,
    };
    let (reply, answer) = Reply::channel();
    assert!(block_on(driver.emit(Event::Prompt(Prompt::Secret {
        request: request.clone(),
        reply,
        hop: None
    }))));
    cx.run_until_parked();
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.answer_secret_and_remember(Secret::new("account password"), true, cx)
        })
    });
    assert_eq!(
        block_on(answer).unwrap().unwrap().expose(),
        "account password"
    );
    assert!(block_on(service.list()).unwrap().is_empty());
    assert!(block_on(driver.emit(Event::Connected)));
    terminal
        .condition::<nocterm_terminal::TerminalEvent>(cx, |terminal, _| {
            terminal
                .credential_message()
                .is_some_and(|message| message.contains("history refused"))
        })
        .await;
    let id = cx.update(|cx| terminal.read(cx).spec().credential.unwrap());
    let credentials = nocterm_vault_ui::VaultCredentials::new(service);
    assert_eq!(
        block_on(credentials.get(id, &request)).unwrap().expose(),
        "account password"
    );
    assert!(cx.update(|cx| {
        terminal
            .read(cx)
            .credential_message()
            .unwrap()
            .contains("history refused")
    }));
    assert!(connections.read_with(cx, |connections, cx| {
        connections
            .profiles(cx)
            .get(profile.id)
            .unwrap()
            .credential
            .is_none()
    }));
}
