//! Installed capabilities remain associated with their original models.
use crate::{CredentialProvider, Deps, Installed, LocalSessions, Status, Terminal};
use gpui_kit::{AppContext as _, Entity, Task, TestAppContext};
use nocterm_session::{
    Auth, CloseReason, Target,
    testing::{FakeCredentialStore, ScriptedTransport},
};
use nocterm_store::SettingsConfig;
use nocterm_ui::DesignTokens;
use nocterm_workspace::SessionSpec;
use std::sync::Arc;

fn remote(installed: Installed, name: &str, cx: &mut TestAppContext) -> Entity<Terminal> {
    cx.update(|cx| {
        cx.new(|cx| {
            Terminal::new(
                installed,
                SessionSpec {
                    profile: None,
                    options: Default::default(),
                    title: name.into(),
                    target: Target::new(name, name, 22),
                    auth: Auth::Password,
                    launch: None,
                    credential: None,
                },
                cx,
            )
        })
    })
}

fn install(
    transport: Arc<ScriptedTransport>,
    credentials: Option<Arc<FakeCredentialStore>>,
    program: &str,
    cx: &mut TestAppContext,
) -> (Installed, nocterm_store::SettingsHandle) {
    let provider =
        credentials.map(|store| CredentialProvider::new(store, |_, _, _| Task::ready(Ok(()))));
    install_with_provider(transport, provider, program, cx)
}
fn install_with_provider(
    transport: Arc<ScriptedTransport>,
    provider: Option<CredentialProvider>,
    program: &str,
    cx: &mut TestAppContext,
) -> (Installed, nocterm_store::SettingsHandle) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        let ready = nocterm_ui::init(
            DesignTokens::builtin(),
            (SettingsConfig::in_memory(Default::default())).install(cx),
            cx,
        );
        let mut deps = Deps::new(transport, LocalSessions::disabled(program.into()));
        if let Some(provider) = provider {
            deps = deps.with_credentials(provider);
        }
        (crate::install(deps, &ready, cx), ready.settings().clone())
    })
}

#[test]
fn installed_can_be_retained_by_the_existing_send_sync_auth_factory() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Installed>();
    assert_send_sync::<CredentialProvider>();
}

#[gpui_kit::test]
fn reconnect_uses_original_transport_after_another_installation(cx: &mut TestAppContext) {
    let first = Arc::new(ScriptedTransport::default());
    let _first_session = first.queue(None, None);
    let _next_session = first.queue(None, None);
    let (a, _) = install(first.clone(), None, "/bin/bash", cx);
    let terminal = remote(a, "first", cx);
    let second = Arc::new(ScriptedTransport::default());
    let _second_session = second.queue(None, None);
    let (b, _) = install(second.clone(), None, "pwsh", cx);
    let _other = remote(b, "second", cx);
    cx.run_until_parked();
    cx.update(|cx| terminal.update(cx, |terminal, cx| terminal.reconnect(cx)));
    cx.run_until_parked();
    assert_eq!(first.requests().len(), 2);
    assert!(
        first
            .requests()
            .iter()
            .all(|request| request.target.host == "first")
    );
    assert_eq!(second.requests().len(), 1);
    assert_eq!(second.requests()[0].target.host, "second");
}

#[gpui_kit::test]
fn vault_availability_belongs_to_each_installed_provider(cx: &mut TestAppContext) {
    let first = Arc::new(ScriptedTransport::default());
    let _first_session = first.queue(None, None);
    let locked = Arc::new(FakeCredentialStore::default());
    locked.set_available(false);
    let (a, _) = install(first, Some(locked.clone()), "/bin/bash", cx);
    let terminal = remote(a, "locked", cx);
    let second = Arc::new(ScriptedTransport::default());
    let _second_session = second.queue(None, None);
    let (b, _) = install(
        second,
        Some(Arc::new(FakeCredentialStore::default())),
        "pwsh",
        cx,
    );
    let other = remote(b, "unlocked", cx);
    cx.update(|cx| {
        assert!(!terminal.read(cx).vault_unlocked(cx));
        assert!(other.read(cx).vault_unlocked(cx));
    });
    locked.set_available(true);
    cx.update(|cx| assert!(terminal.read(cx).vault_unlocked(cx)));
}

#[gpui_kit::test]
fn reconnect_observes_its_published_settings_after_ui_is_reinstalled(cx: &mut TestAppContext) {
    let first = Arc::new(ScriptedTransport::default());
    let _first_session = first.queue(None, None);
    let _next_session = first.queue(None, None);
    let (a, settings) = install(first.clone(), None, "/bin/bash", cx);
    let terminal = remote(a, "first", cx);
    let second = Arc::new(ScriptedTransport::default());
    let _second_session = second.queue(None, None);
    let (b, _) = install(second.clone(), None, "pwsh", cx);
    let _other = remote(b, "second", cx);
    cx.update(|cx| {
        settings
            .update::<nocterm_ui::TerminalSettings>(cx, |settings| settings.term = "vt100".into())
            .detach()
    });
    cx.run_until_parked();
    cx.update(|cx| terminal.update(cx, |terminal, cx| terminal.reconnect(cx)));
    cx.run_until_parked();
    assert_eq!(first.requests()[1].term, "vt100");
    assert_ne!(second.requests()[0].term, "vt100");
}

#[gpui_kit::test]
fn isolated_command_overrides_absent_default_local_factory(cx: &mut TestAppContext) {
    let remote = Arc::new(ScriptedTransport::default());
    let (installed, _) = install(remote.clone(), None, "/bin/bash", cx);
    let command = Arc::new(ScriptedTransport::default());
    let _driver = command.queue(None, None);
    let terminal = cx.update(|cx| {
        cx.new(|cx| {
            Terminal::new_local_command(
                installed,
                nocterm_session::ShellLaunch {
                    program: Some("isolated".into()),
                    ..Default::default()
                },
                "Auth".into(),
                command.clone(),
                |_, _| {},
                cx,
            )
        })
    });
    cx.run_until_parked();
    assert!(remote.requests().is_empty());
    assert_eq!(
        command.requests()[0].launch.program.as_deref(),
        Some("isolated")
    );
    cx.update(|cx| assert!(!matches!(terminal.read(cx).status(), Status::Closed(_))));
}

#[gpui_kit::test]
fn missing_local_provider_closes_without_using_remote_transport(cx: &mut TestAppContext) {
    let remote = Arc::new(ScriptedTransport::default());
    let (installed, _) = install(remote.clone(), None, "pwsh", cx);
    let terminal = cx.update(|cx| cx.new(|cx| Terminal::new_local(installed, cx)));
    cx.update(|cx| {
        assert!(matches!(
            terminal.read(cx).status(),
            Status::Closed(CloseReason::Failed(_))
        ))
    });
    assert!(remote.requests().is_empty());
}

#[gpui_kit::test]
fn original_installed_association_callback_exposes_its_native_leaf(cx: &mut TestAppContext) {
    use futures::executor::block_on;
    use nocterm_session::{Event, Prompt, Reply, Secret, SecretRequest};
    use std::{error::Error as _, io, sync::Mutex};
    let store = Arc::new(FakeCredentialStore::default());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let provider = CredentialProvider::new(store.clone(), move |spec, id, _| {
        let error = crate::CredentialAssociationError::binding(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "history refused",
        ));
        let original = error.source().unwrap().downcast_ref::<io::Error>().unwrap();
        assert_eq!(original.kind(), io::ErrorKind::PermissionDenied);
        assert!(original.source().is_none());
        assert_eq!(
            error.to_string(),
            "could not associate the saved credential: history refused"
        );
        recorded.lock().unwrap().push((spec.target.clone(), id));
        Task::ready(Err(error))
    });
    let transport = Arc::new(ScriptedTransport::default());
    let driver = transport.queue(None, None);
    let (original, _) = install_with_provider(transport, Some(provider), "/bin/bash", cx);
    let terminal = remote(original, "original", cx);
    let other = Arc::new(ScriptedTransport::default());
    let (replacement, _) = install(
        other,
        Some(Arc::new(FakeCredentialStore::default())),
        "pwsh",
        cx,
    );
    drop(replacement);
    let target = Target::new("original", "original", 22);
    let request = SecretRequest::Password {
        target: target.clone(),
        retry: false,
    };
    let (reply, answer) = Reply::channel();
    assert!(block_on(driver.emit(Event::Prompt(Prompt::Secret {
        request: request.clone(),
        reply
    }))));
    cx.run_until_parked();
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.answer_secret_and_remember(Secret::new("saved answer"), true, cx)
        })
    });
    assert_eq!(block_on(answer).unwrap().unwrap().expose(), "saved answer");
    assert!(store.labels().is_empty());
    assert!(block_on(driver.emit(Event::Connected)));
    cx.run_until_parked();
    let id = cx.update(|cx| terminal.read(cx).spec().credential.unwrap());
    assert_eq!(*calls.lock().unwrap(), [(target, id)]);
    assert_eq!(store.lookup(id, &request).unwrap().expose(), "saved answer");
    assert!(cx.update(|cx| {
        terminal
            .read(cx)
            .credential_message()
            .unwrap()
            .contains("history refused")
    }));
    let unavailable = crate::CredentialAssociationError::Unavailable;
    assert!(unavailable.source().is_none());
    assert_eq!(
        unavailable.to_string(),
        "connection history is unavailable for credential association"
    );
}
