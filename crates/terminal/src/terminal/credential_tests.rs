use super::*;
use crate::credentials::fake::FakeStore;
use futures::executor::block_on;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use nocterm_session::{Auth, PromptHop, Reply, SecretRequest, Target};
use std::sync::Mutex;

fn prompt(request: SecretRequest) -> Prompt {
    let (reply, _answer) = Reply::channel();
    Prompt::Secret {
        request,
        reply,
        hop: None,
    }
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
            (SettingsConfig::in_memory(Default::default())).install(cx),
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
                    jumps: Vec::new(),
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
                    hop: None,
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
            (SettingsConfig::in_memory(Default::default())).install(cx),
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
                    jumps: Vec::new(),
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

/// Public channel/installation fixture, using the shared session port fakes.
fn association_terminal(
    provider: crate::CredentialProvider,
    cx: &mut TestAppContext,
) -> (
    Entity<Terminal>,
    nocterm_session::SessionDriver,
    Arc<nocterm_session::testing::ScriptedTransport>,
) {
    let transport = Arc::new(nocterm_session::testing::ScriptedTransport::default());
    let driver = transport.queue(None, None);
    let terminal = cx.update(|cx| {
        gpui_kit::init(cx);
        let ui = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            (SettingsConfig::in_memory(Default::default())).install(cx),
            cx,
        );
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
                installed,
                SessionSpec {
                    profile: None,
                    options: Default::default(),
                    title: "association".into(),
                    target: Target::new("user", "host", 22),
                    auth: Auth::Password,
                    launch: None,
                    credential: None,
                    jumps: Vec::new(),
                },
                cx,
            )
        })
    });
    (terminal, driver, transport)
}
fn deliver_association_prompt(
    driver: &nocterm_session::SessionDriver,
    request: SecretRequest,
    cx: &mut TestAppContext,
) -> futures::channel::oneshot::Receiver<Option<Secret>> {
    let (reply, answer) = Reply::channel();
    assert!(block_on(driver.emit(Event::Prompt(Prompt::Secret {
        request,
        reply,
        hop: None
    }))));
    cx.run_until_parked();
    answer
}
#[gpui_kit::test]
fn failed_association_keeps_saved_password_and_original_typed_backend_before_native_cause(
    cx: &mut TestAppContext,
) {
    use nocterm_session::testing::FakeCredentialStore;
    use std::{error::Error as _, io};
    let store = Arc::new(FakeCredentialStore::default());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let provider = crate::CredentialProvider::new(store.clone(), move |spec, id, _| {
        let original = nocterm_store::StoreError::backend(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "history refused",
        ));
        let error = crate::CredentialAssociationError::binding(original);
        let backend = error
            .source()
            .unwrap()
            .downcast_ref::<nocterm_store::StoreError>()
            .unwrap();
        assert!(matches!(backend, nocterm_store::StoreError::Backend(_)));
        let native = backend
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap();
        assert_eq!(native.kind(), io::ErrorKind::PermissionDenied);
        recorded.lock().unwrap().push((spec.target.clone(), id));
        Task::ready(Err(error))
    });
    let (terminal, driver, _) = association_terminal(provider, cx);
    let target = cx.update(|cx| terminal.read(cx).spec().target.clone());
    let request = SecretRequest::Password {
        target: target.clone(),
        retry: false,
    };
    let answer = deliver_association_prompt(&driver, request.clone(), cx);
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.answer_secret_and_remember(Secret::new("account password"), true, cx)
        })
    });
    assert_eq!(
        block_on(answer).unwrap().unwrap().expose(),
        "account password"
    );
    assert!(store.labels().is_empty());
    let mfa = deliver_association_prompt(
        &driver,
        SecretRequest::Interactive {
            prompt: "MFA".into(),
            echo: false,
        },
        cx,
    );
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.answer_secret_and_remember(Secret::new("123456"), true, cx)
        })
    });
    assert_eq!(block_on(mfa).unwrap().unwrap().expose(), "123456");
    assert!(block_on(driver.emit(Event::Connected)));
    cx.run_until_parked();
    let id = cx.update(|cx| terminal.read(cx).spec().credential.unwrap());
    assert_eq!(*calls.lock().unwrap(), [(target, id)]);
    assert_eq!(store.labels(), ["association"]);
    assert_eq!(
        store.lookup(id, &request).unwrap().expose(),
        "account password"
    );
    assert!(cx.update(|cx| {
        terminal
            .read(cx)
            .credential_message()
            .unwrap()
            .contains("Credential saved; could not link")
    }));
}
#[gpui_kit::test]
fn old_association_failure_does_not_replace_a_reconnected_password_prompt(cx: &mut TestAppContext) {
    use futures::{FutureExt as _, channel::oneshot};
    use nocterm_session::testing::FakeCredentialStore;
    let store = Arc::new(FakeCredentialStore::default());
    let (complete, result) = oneshot::channel();
    let waiting = Arc::new(Mutex::new(Some(result)));
    let provider = crate::CredentialProvider::new(store.clone(), move |_, _, cx| {
        let result = waiting.lock().unwrap().take().unwrap();
        cx.spawn(async move |_| result.await.unwrap())
    });
    let (terminal, driver, transport) = association_terminal(provider, cx);
    let next = transport.queue(None, None);
    let target = cx.update(|cx| terminal.read(cx).spec().target.clone());
    let request = SecretRequest::Password {
        target: target.clone(),
        retry: false,
    };
    let answer = deliver_association_prompt(&driver, request.clone(), cx);
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.answer_secret_and_remember(Secret::new("account password"), true, cx)
        })
    });
    assert_eq!(
        block_on(answer).unwrap().unwrap().expose(),
        "account password"
    );
    assert!(block_on(driver.emit(Event::Connected)));
    cx.run_until_parked();
    let id = cx.update(|cx| terminal.read(cx).spec().credential.unwrap());
    cx.update(|cx| terminal.update(cx, |terminal, cx| terminal.reconnect(cx)));
    cx.run_until_parked();
    assert!(driver.closed().now_or_never().is_some());
    let mut replacement = deliver_association_prompt(
        &next,
        SecretRequest::Password {
            target: target.clone(),
            retry: true,
        },
        cx,
    );
    complete
        .send(Err(crate::CredentialAssociationError::binding(
            std::io::Error::other("old history refusal"),
        )))
        .unwrap();
    cx.run_until_parked();
    assert!((&mut replacement).now_or_never().is_none());
    assert_eq!(transport.requests().len(), 2);
    assert_eq!(
        store.lookup(id, &request).unwrap().expose(),
        "account password"
    );
    cx.update(|cx| {
        let terminal = terminal.read(cx);
        assert!(matches!(terminal.prompt(), Some(Prompt::Secret { request: SecretRequest::Password { retry: true, target: current }, .. }) if current == &target));
        assert!(!terminal.credential_message().is_some_and(|message| message.contains("old history refusal")));
    });
}

fn bastion() -> Target {
    Target::new("ops", "bastion", 22)
}

fn hop_prompt(
    request: SecretRequest,
    hop: PromptHop,
) -> (Prompt, futures::channel::oneshot::Receiver<Option<Secret>>) {
    let (reply, answer) = Reply::channel();
    (
        Prompt::Secret {
            request,
            hop: Some(hop),
            reply,
        },
        answer,
    )
}

/// A terminal for `destination` reached through the bastion, whose password
/// is saved as `hop_credential`.
fn jump_terminal(
    store: &Arc<FakeStore>,
    destination_credential: Option<nocterm_session::CredentialId>,
    hop_credential: Option<nocterm_session::CredentialId>,
    saved: &Arc<Mutex<Vec<nocterm_session::CredentialId>>>,
    cx: &mut TestAppContext,
) -> (
    Entity<Terminal>,
    Arc<nocterm_session::testing::ScriptedTransport>,
    nocterm_session::SessionDriver,
) {
    let transport = Arc::new(nocterm_session::testing::ScriptedTransport::default());
    let driver = transport.queue(None, None);
    let sink = saved.clone();
    let store = store.clone();
    let terminal = cx.update(|cx| {
        gpui_kit::init(cx);
        let ui = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            (SettingsConfig::in_memory(Default::default())).install(cx),
            cx,
        );
        let provider = crate::CredentialProvider::new(store, move |_, id, _| {
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
                installed,
                SessionSpec {
                    profile: None,
                    options: Default::default(),
                    title: "inner".into(),
                    target: Target::new("app", "inner", 22),
                    auth: Auth::Password,
                    launch: None,
                    credential: destination_credential,
                    jumps: vec![nocterm_session::ResolvedJump {
                        host: nocterm_session::JumpHost {
                            target: bastion(),
                            auth: Auth::Password,
                        },
                        credential: hop_credential,
                    }],
                },
                cx,
            )
        })
    });
    (terminal, transport, driver)
}

fn password(target: Target) -> SecretRequest {
    SecretRequest::Password {
        target,
        retry: false,
    }
}

fn show(terminal: &Entity<Terminal>, prompt: Prompt, cx: &mut TestAppContext) {
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.handle_event(Event::Prompt(prompt), cx)
        })
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn connect_request_carries_the_session_jump_hosts(cx: &mut TestAppContext) {
    let store = Arc::new(FakeStore::unlocked());
    let (_terminal, transport, _driver) = jump_terminal(&store, None, None, &Arc::default(), cx);
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].jumps,
        [nocterm_session::JumpHost {
            target: bastion(),
            auth: Auth::Password,
        }]
    );
}

#[gpui_kit::test]
fn a_jump_host_prompt_is_answered_from_that_hops_credential(cx: &mut TestAppContext) {
    let store = Arc::new(FakeStore::unlocked());
    let bastion_id = store.insert(password(bastion()), "bastion password");
    let (terminal, _transport, _driver) =
        jump_terminal(&store, None, Some(bastion_id), &Arc::default(), cx);
    let (prompt, answer) = hop_prompt(
        password(bastion()),
        PromptHop {
            index: 0,
            target: bastion(),
        },
    );
    show(&terminal, prompt, cx);
    assert_eq!(
        block_on(answer).unwrap().unwrap().expose(),
        "bastion password"
    );
}

#[gpui_kit::test]
fn the_destination_prompt_keeps_using_the_destination_credential(cx: &mut TestAppContext) {
    let store = Arc::new(FakeStore::unlocked());
    let bastion_id = store.insert(password(bastion()), "bastion password");
    let inner = Target::new("app", "inner", 22);
    let inner_id = store.insert(password(inner.clone()), "inner password");
    let (terminal, _transport, _driver) = jump_terminal(
        &store,
        Some(inner_id),
        Some(bastion_id),
        &Arc::default(),
        cx,
    );
    let (reply, answer) = Reply::channel();
    show(
        &terminal,
        Prompt::Secret {
            request: password(inner),
            hop: None,
            reply,
        },
        cx,
    );
    assert_eq!(
        block_on(answer).unwrap().unwrap().expose(),
        "inner password"
    );
}

#[gpui_kit::test]
fn a_hop_prompt_for_another_host_is_not_answered_from_the_route_entry(cx: &mut TestAppContext) {
    let store = Arc::new(FakeStore::unlocked());
    let bastion_id = store.insert(password(bastion()), "bastion password");
    let (terminal, _transport, _driver) =
        jump_terminal(&store, None, Some(bastion_id), &Arc::default(), cx);
    let stranger = Target::new("ops", "stranger", 22);
    let (prompt, mut answer) = hop_prompt(
        password(stranger.clone()),
        PromptHop {
            index: 0,
            target: stranger,
        },
    );
    show(&terminal, prompt, cx);
    cx.update(|cx| {
        let terminal = terminal.read(cx);
        assert!(terminal.prompt().is_some(), "the prompt waits for the user");
        assert_eq!(
            terminal.credential_message(),
            None,
            "no saved credential was even tried"
        );
    });
    assert!(futures::FutureExt::now_or_never(&mut answer).is_none());
}

#[gpui_kit::test]
fn remembering_on_a_jump_host_prompt_saves_nothing(cx: &mut TestAppContext) {
    let store = Arc::new(FakeStore::unlocked());
    let saved = Arc::new(Mutex::new(Vec::new()));
    let (terminal, _transport, _driver) = jump_terminal(&store, None, None, &saved, cx);
    let (prompt, answer) = hop_prompt(
        password(bastion()),
        PromptHop {
            index: 0,
            target: bastion(),
        },
    );
    show(&terminal, prompt, cx);
    cx.update(|cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.answer_secret_and_remember(Secret::new("typed"), true, cx);
            terminal.handle_event(Event::Connected, cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(block_on(answer).unwrap().unwrap().expose(), "typed");
    assert!(store.records().is_empty());
    assert!(saved.lock().unwrap().is_empty());
}
