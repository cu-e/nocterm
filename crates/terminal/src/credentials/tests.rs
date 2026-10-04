//! A sign-in prompt waiting for the vault answers itself once it is unlocked.
use crate::Terminal;
use futures::{FutureExt as _, executor::block_on};
use gpui_kit::{AppContext as _, Entity, Task, TestAppContext};
use nocterm_session::{
    Auth, CloseReason, CredentialId, Event, Prompt, Reply, Secret, SecretRequest, Target,
};
use nocterm_ui::SettingsStore;
use nocterm_vault::{CredentialBinding, VaultService};
use nocterm_workspace::SessionSpec;
use std::{sync::Arc, time::Duration};

const MASTER: &str = "test unique master passphrase";

struct Setup {
    service: Arc<VaultService>,
    terminal: Entity<Terminal>,
    target: Target,
    _directory: tempfile::TempDir,
}

/// A terminal for a saved server whose password is in a locked vault.
fn setup(cx: &mut TestAppContext) -> Setup {
    let directory = tempfile::tempdir().unwrap();
    let service = Arc::new(
        VaultService::new(directory.path().join("vault"), Duration::from_secs(60)).unwrap(),
    );
    block_on(service.create(Secret::new(MASTER))).unwrap();
    let target = Target::new("test", "test-host", 22);
    let id: CredentialId = block_on(service.put(
        None,
        "test".into(),
        CredentialBinding::Password {
            target: target.clone(),
        },
        Secret::new("stored account password"),
    ))
    .unwrap();
    service.lock();
    let terminal = cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(Default::default()),
            cx,
        );
        crate::init_credentials(service.clone(), |_, _, _| Task::ready(Ok(())), cx);
        cx.new(|cx| {
            Terminal::new(
                SessionSpec {
                    profile: None,
                    options: Default::default(),
                    title: "test".into(),
                    target: target.clone(),
                    auth: Auth::Password,
                    launch: None,
                    credential: Some(id),
                },
                cx,
            )
        })
    });
    Setup {
        service,
        terminal,
        target,
        _directory: directory,
    }
}

fn ask_password(
    setup: &Setup,
    cx: &mut TestAppContext,
) -> futures::channel::oneshot::Receiver<Option<Secret>> {
    let (reply, answer) = Reply::channel();
    let request = SecretRequest::Password {
        target: setup.target.clone(),
        retry: false,
    };
    cx.update(|cx| {
        setup.terminal.update(cx, |terminal, cx| {
            terminal.handle_event(Event::Prompt(Prompt::Secret { request, reply }), cx)
        })
    });
    cx.run_until_parked();
    answer
}

fn wait(cx: &mut TestAppContext) {
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn unlocking_the_vault_answers_the_waiting_prompt_with_the_saved_secret(cx: &mut TestAppContext) {
    let setup = setup(cx);
    let mut answer = ask_password(&setup, cx);
    wait(cx);
    assert!(
        answer.try_recv().unwrap().is_none(),
        "a locked vault cannot answer"
    );
    assert!(cx.update(|cx| {
        setup
            .terminal
            .read(cx)
            .credential_message()
            .is_some_and(|message| message.contains("Unlock the credential vault"))
    }));
    assert!(cx.update(|cx| setup.terminal.read(cx).awaiting_vault()));

    block_on(setup.service.unlock(Secret::new(MASTER))).unwrap();
    wait(cx);
    // The lookup runs on the vault's worker.
    block_on(setup.service.list()).unwrap();
    cx.run_until_parked();
    let secret = answer
        .now_or_never()
        .expect("the prompt was answered after unlocking")
        .unwrap()
        .expect("with a secret");
    assert_eq!(secret.expose(), "stored account password");
    cx.update(|cx| {
        let terminal = setup.terminal.read(cx);
        assert!(terminal.prompt().is_none());
        assert!(terminal.credential_message().is_none());
        assert!(!terminal.awaiting_vault());
    });
}

#[gpui_kit::test]
fn a_prompt_that_ended_is_not_answered_after_unlocking(cx: &mut TestAppContext) {
    let setup = setup(cx);
    let mut answer = ask_password(&setup, cx);
    cx.update(|cx| {
        setup.terminal.update(cx, |terminal, cx| {
            terminal.handle_event(Event::Closed(CloseReason::ClosedByUser), cx)
        })
    });
    block_on(setup.service.unlock(Secret::new(MASTER))).unwrap();
    wait(cx);
    block_on(setup.service.list()).unwrap();
    cx.run_until_parked();
    assert!(
        !matches!(answer.try_recv(), Ok(Some(Some(_)))),
        "a closed session never receives the secret"
    );
}
