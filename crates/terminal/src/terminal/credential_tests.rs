use super::*;
use futures::executor::block_on;
use gpui_kit::{AppContext as _, TestAppContext};
use nocterm_session::{Auth, Reply, SecretRequest, Target};
use nocterm_vault::{CredentialBinding, VaultService};
use std::{cell::RefCell, rc::Rc};

fn prompt(request: SecretRequest) -> Prompt {
    let (reply, _answer) = Reply::channel();
    Prompt::Secret { request, reply }
}
#[gpui_kit::test]
fn retrieves_only_matching_non_retry_credentials_and_ignores_stale_requests(
    cx: &mut TestAppContext,
) {
    // The vault worker is a real thread; let its completions wake the test scheduler.
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let service = Arc::new(
        VaultService::new(directory.path().join("vault"), Duration::from_secs(60)).unwrap(),
    );
    block_on(service.create(Secret::new("test unique master passphrase"))).unwrap();
    let target = Target::new("test", "test-host", 22);
    let id = block_on(service.put(
        None,
        "test".into(),
        CredentialBinding::Password {
            target: target.clone(),
        },
        Secret::new("stored account password"),
    ))
    .unwrap();
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
    block_on(service.list()).unwrap();
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
    block_on(service.list()).unwrap();
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
    block_on(service.list()).unwrap();
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
    block_on(service.list()).unwrap();
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
fn remembers_password_only_after_success_and_never_remembers_mfa(cx: &mut TestAppContext) {
    // The vault worker is a real thread; let its completions wake the test scheduler.
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let service = Arc::new(
        VaultService::new(directory.path().join("vault"), Duration::from_secs(60)).unwrap(),
    );
    block_on(service.create(Secret::new("test unique master passphrase"))).unwrap();
    let saved = Rc::new(RefCell::new(Vec::new()));
    let sink = saved.clone();
    let target = Target::new("test", "test-host", 22);
    let terminal = cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            SettingsStore::in_memory(Default::default()),
            cx,
        );
        crate::init_credentials(
            service.clone(),
            move |_, id, _| {
                sink.borrow_mut().push(id);
                Task::ready(Ok(()))
            },
            cx,
        );
        cx.new(|cx| {
            Terminal::new(
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
        block_on(service.list()).unwrap().is_empty(),
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
    let records = block_on(service.list()).unwrap();
    cx.run_until_parked();
    assert_eq!(records.len(), 1);
    assert_eq!(saved.borrow().len(), 1);
    let binding = CredentialBinding::Password {
        target: target.clone(),
    };
    assert_eq!(
        block_on(service.get(records[0].id, binding))
            .unwrap()
            .expose(),
        "saved account password"
    );
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
        block_on(service.list()).unwrap().len(),
        1,
        "failed attempts must never be saved"
    );
}
