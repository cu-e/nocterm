//! A sign-in prompt waiting for the vault answers itself once it is unlocked.
use crate::{Terminal, credentials::fake::FakeStore};
use futures::FutureExt as _;
use gpui_kit::{AppContext as _, Entity, Task, TestAppContext};
use nocterm_session::{Auth, CloseReason, Event, Prompt, Reply, Secret, SecretRequest, Target};
use nocterm_ui::SettingsStore;
use nocterm_workspace::SessionSpec;
use std::{sync::Arc, time::Duration};

struct Setup {
    store: Arc<FakeStore>,
    terminal: Entity<Terminal>,
    target: Target,
}

/// A terminal for a saved server whose password is in a locked vault.
fn setup(cx: &mut TestAppContext) -> Setup {
    let store = Arc::new(FakeStore::default());
    let target = Target::new("test", "test-host", 22);
    let id = store.insert(
        SecretRequest::Password {
            target: target.clone(),
            retry: false,
        },
        "stored account password",
    );
    let terminal = cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(Default::default()),
            cx,
        );
        crate::init_credentials(store.clone(), |_, _, _| Task::ready(Ok(())), cx);
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
        store,
        terminal,
        target,
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
    drain(setup, cx);
    answer
}

fn drain(_setup: &Setup, cx: &mut TestAppContext) {
    cx.run_until_parked();
}

fn wait(setup: &Setup, cx: &mut TestAppContext) {
    cx.executor().advance_clock(Duration::from_secs(1));
    drain(setup, cx);
}

#[gpui_kit::test]
fn unlocking_the_vault_answers_the_waiting_prompt_with_the_saved_secret(cx: &mut TestAppContext) {
    let setup = setup(cx);
    let mut answer = ask_password(&setup, cx);
    wait(&setup, cx);
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

    setup.store.unlock();
    wait(&setup, cx);
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
    setup.store.unlock();
    wait(&setup, cx);
    assert!(
        !matches!(answer.try_recv(), Ok(Some(Some(_)))),
        "a closed session never receives the secret"
    );
}
