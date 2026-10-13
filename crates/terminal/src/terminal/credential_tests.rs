use super::*;
use crate::credentials::fake::FakeStore;
use futures::executor::block_on;
use gpui_kit::{AppContext as _, TestAppContext};
use nocterm_session::{Auth, Reply, SecretRequest, Target};
use std::sync::Mutex;

fn prompt(request: SecretRequest) -> Prompt {
    let (reply, _answer) = Reply::channel();
    Prompt::Secret { request, reply }
}
#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn retrieves_only_matching_non_retry_credentials_and_ignores_stale_requests(
    cx: &mut TestAppContext,
) {
    let store = Arc::new(FakeStore::unlocked());
    let target = Target::new("test", "test-host", 22);
    let id = store.insert(
        SecretRequest::Password {
            target: target.clone(),
            retry: false,
        },
        "stored account password",
    );
    let transport = Arc::new(nocterm_session::testing::ScriptedTransport::default());
    let _driver = transport.queue(None, None);
    let terminal = cx.update(|cx| {
        gpui_kit::init(cx);
        let ui = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(Default::default()),
            cx,
        );
        let provider = crate::CredentialProvider::new(store.clone(), |_, _, _| Task::ready(Ok(())));
        let installed = crate::install(
            crate::Deps::new(
                transport.clone(),
                crate::LocalSessions::disabled("/bin/bash".into()),
            )
            .with_credentials(provider),
            &ui,
            cx,
        );
        cx.new(|cx| {
            Terminal::new(
                installed.clone(),
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
    let (reply, answer) = Reply::channel();
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(
                Event::Prompt(Prompt::Secret {
                    request: SecretRequest::Password {
                        target: target.clone(),
                        retry: false,
                    },
                    reply,
                }),
                cx,
            )
        })
    });
    cx.run_until_parked();
    assert_eq!(
        block_on(answer).unwrap().unwrap().expose(),
        "stored account password"
    );
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(
                Event::Prompt(prompt(SecretRequest::Password {
                    target: target.clone(),
                    retry: true,
                })),
                cx,
            )
        })
    });
    cx.run_until_parked();
    assert!(
        cx.update(|cx| terminal.read(cx).prompt().is_some()),
        "a rejected credential must not loop automatically"
    );
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(
                Event::Prompt(prompt(SecretRequest::Password {
                    target: Target::new("other", "other-host", 22),
                    retry: false,
                })),
                cx,
            )
        })
    });
    cx.run_until_parked();
    assert!(cx.update(|cx| {
        terminal
            .read(cx)
            .credential_message()
            .unwrap()
            .contains("does not match")
    }));
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(
                Event::Prompt(prompt(SecretRequest::Password {
                    target,
                    retry: false,
                })),
                cx,
            );
            terminal.handle_event(
                Event::Prompt(prompt(SecretRequest::Interactive {
                    prompt: "MFA".into(),
                    echo: false,
                })),
                cx,
            );
        })
    });
    cx.run_until_parked();
    assert!(
        cx.update(|cx| matches!(
            terminal.read(cx).prompt(),
            Some(Prompt::Secret {
                request: SecretRequest::Interactive { .. },
                ..
            })
        )),
        "late vault lookup cannot answer a newer MFA prompt"
    );
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn remembers_password_only_after_success_and_never_remembers_mfa(cx: &mut TestAppContext) {
    let store = Arc::new(FakeStore::unlocked());
    let saved = Arc::new(Mutex::new(Vec::new()));
    let sink = saved.clone();
    let target = Target::new("test", "test-host", 22);
    let transport = Arc::new(nocterm_session::testing::ScriptedTransport::default());
    let _driver = transport.queue(None, None);
    let terminal = cx.update(|cx| {
        gpui_kit::init(cx);
        let ui = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(Default::default()),
            cx,
        );
        let provider = crate::CredentialProvider::new(store.clone(), move |_, id, _| {
            sink.lock().unwrap().push(id);
            Task::ready(Ok(()))
        });
        let installed = crate::install(
            crate::Deps::new(
                transport.clone(),
                crate::LocalSessions::disabled("/bin/bash".into()),
            )
            .with_credentials(provider),
            &ui,
            cx,
        );
        cx.new(|cx| {
            Terminal::new(
                installed.clone(),
                SessionSpec {
                    profile: None,
                    options: Default::default(),
                    title: "test".into(),
                    target: target.clone(),
                    auth: Auth::Password,
                    launch: None,
                    credential: None,
                },
                cx,
            )
        })
    });
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(
                Event::Prompt(prompt(SecretRequest::Password {
                    target: target.clone(),
                    retry: false,
                })),
                cx,
            );
            terminal.answer_secret_and_remember(Secret::new("saved account password"), true, cx);
        })
    });
    assert!(
        store.records().is_empty(),
        "answering a prompt is not successful authentication"
    );
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(
                Event::Prompt(prompt(SecretRequest::Interactive {
                    prompt: "MFA".into(),
                    echo: false,
                })),
                cx,
            );
            terminal.answer_secret_and_remember(Secret::new("123456"), true, cx);
            terminal.handle_event(Event::Connected, cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(
        store.records(),
        [(
            SecretRequest::Password {
                target: target.clone(),
                retry: false,
            },
            "saved account password".to_owned()
        )]
    );
    assert_eq!(saved.lock().unwrap().len(), 1);
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(
                Event::Prompt(prompt(SecretRequest::Password {
                    target,
                    retry: false,
                })),
                cx,
            );
            terminal.answer_secret_and_remember(Secret::new("wrong-password"), true, cx);
            terminal.handle_event(Event::Closed(CloseReason::ClosedByUser), cx);
            terminal.handle_event(Event::Connected, cx);
        })
    });
    assert_eq!(
        store.records().len(),
        1,
        "failed attempts must never be saved"
    );
}
