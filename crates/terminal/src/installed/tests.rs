//! Installed capabilities remain associated with their original models.
use crate::{CredentialProvider, Deps, Installed, LocalSessions, Status, Terminal};
use gpui_kit::{AppContext as _, Entity, Task, TestAppContext};
use nocterm_session::{
    Auth, CloseReason, Target,
    testing::{FakeCredentialStore, ScriptedTransport},
};
use nocterm_ui::{DesignTokens, SettingsStore};
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
    cx.update(|cx| {
        gpui_kit::init(cx);
        let ready = nocterm_ui::init(
            DesignTokens::builtin(),
            SettingsStore::in_memory(Default::default()),
            cx,
        );
        let mut deps = Deps::new(transport, LocalSessions::disabled(program.into()));
        if let Some(store) = credentials {
            deps = deps.with_credentials(CredentialProvider::new(store, |_, _, _| {
                Task::ready(Ok(()))
            }));
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
