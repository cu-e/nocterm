use super::*;
use futures::FutureExt as _;
use gpui_kit::{TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_session::{
    Auth, ConnectRequest, ConnectStage, Event, PromptHop, Reply, Session, SessionDriver, Target,
    Transport,
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct PromptTransport(Mutex<Option<Arc<SessionDriver>>>);
impl Transport for PromptTransport {
    fn open(&self, _: ConnectRequest) -> Session {
        let (session, driver) = nocterm_session::channel(None);
        *self.0.lock().unwrap() = Some(Arc::new(driver));
        session
    }
}
fn emit(
    cx: &mut TestAppContext,
    transport: &PromptTransport,
    request: SecretRequest,
) -> futures::future::BoxFuture<'static, Option<Secret>> {
    let (reply, answer) = Reply::channel();
    let driver = transport.0.lock().unwrap().as_ref().unwrap().clone();
    cx.background_executor
        .spawn(async move {
            driver
                .emit(Event::Prompt(Prompt::Secret {
                    request,
                    reply,
                    hop: None,
                }))
                .await;
        })
        .detach();
    cx.run_until_parked();
    async move { answer.await.ok().flatten() }.boxed()
}
fn password() -> SecretRequest {
    SecretRequest::Password {
        target: Target::new("me", "host", 22),
        retry: false,
    }
}
fn connect(cx: &mut TestAppContext, transport: &PromptTransport) {
    let driver = transport.0.lock().unwrap().as_ref().unwrap().clone();
    cx.background_executor
        .spawn(async move { driver.emit(Event::Connected).await })
        .detach();
    cx.run_until_parked();
}
fn credential_fixture(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::AnyWindowHandle,
    Arc<PromptTransport>,
    Arc<nocterm_session::testing::FakeCredentialStore>,
) {
    let store = Arc::new(nocterm_session::testing::FakeCredentialStore::default());
    let provider = crate::CredentialProvider::new(store.clone(), |_, _, _| Task::ready(Ok(())));
    let (handle, view, transport) = prompt_fixture(cx, Some(provider));
    cx.update_window(handle, |_, window, cx| {
        window.focus(&view.read(cx).focus_handle(cx), cx);
    })
    .unwrap();
    (handle, transport, store)
}
#[gpui_kit::test]
fn superseded_secret_recreates_a_masked_empty_focused_field_without_remember(
    cx: &mut TestAppContext,
) {
    let (handle, transport, store) = credential_fixture(cx);
    let _visible = emit(
        cx,
        &transport,
        SecretRequest::Interactive {
            prompt: "Visible code".into(),
            echo: true,
        },
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("remember-credential").is_none());
        window.input("previous answer", cx);
        window.render_frame(cx);
        // An echoed field exposes its text; a masked one never does.
        assert_eq!(window.find("secret-input").value(), Some("previous answer"));
    })
    .unwrap();
    let _remembered = emit(cx, &transport, password());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let field = window.find("secret-input");
        assert_eq!(field.value(), None);
        assert_eq!(field.focused(), Some(true));
        window.click("remember-credential", cx);
        window.render_frame(cx);
        assert_eq!(window.find("remember-credential").checked(), Some(true));
        window.input("first password", cx);
    })
    .unwrap();
    // Even the same question is a new authentication attempt.
    let answer = emit(cx, &transport, password());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("remember-credential").checked(), Some(false));
        assert_eq!(window.find("secret-input").focused(), Some(true));
        // A recreated field has no undo history of earlier answers.
        window.press("ctrl-z", cx);
        window.input("fresh password", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let answer = answer.now_or_never().unwrap().unwrap();
    assert_eq!(answer.expose(), "fresh password");
    connect(cx, &transport);
    assert!(
        store.labels().is_empty(),
        "the superseded field's remember choice was kept"
    );
}
#[gpui_kit::test]
fn checked_remember_saves_the_answer_after_sign_in(cx: &mut TestAppContext) {
    let (handle, transport, store) = credential_fixture(cx);
    let answer = emit(cx, &transport, password());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("remember-credential", cx);
        window.input("kept password", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        answer.now_or_never().unwrap().unwrap().expose(),
        "kept password"
    );
    assert!(store.labels().is_empty(), "saved before sign in");
    connect(cx, &transport);
    assert_eq!(store.labels().len(), 1);
}

fn prompt_fixture(
    cx: &mut TestAppContext,
    credentials: Option<crate::CredentialProvider>,
) -> (
    gpui_kit::AnyWindowHandle,
    Entity<TerminalView>,
    Arc<PromptTransport>,
) {
    prompt_fixture_via(cx, credentials, Vec::new())
}
fn prompt_fixture_via(
    cx: &mut TestAppContext,
    credentials: Option<crate::CredentialProvider>,
    jumps: Vec<nocterm_session::ResolvedJump>,
) -> (
    gpui_kit::AnyWindowHandle,
    Entity<TerminalView>,
    Arc<PromptTransport>,
) {
    let transport = Arc::new(PromptTransport::default());
    let (handle, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        let ui = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            (nocterm_store::SettingsConfig::in_memory(Default::default())).install(cx),
            cx,
        );
        let mut deps = crate::Deps::new(
            transport.clone(),
            crate::LocalSessions::disabled("/bin/bash".into()),
        );
        if let Some(credentials) = credentials {
            deps = deps.with_credentials(credentials);
        }
        let installed = crate::install(deps, &ui, cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                TerminalView::new(
                    installed.clone(),
                    SessionSpec {
                        profile: None,
                        options: Default::default(),
                        title: "test".into(),
                        target: Target::new("me", "host", 22),
                        auth: Auth::Password,
                        credential: None,
                        launch: None,
                        jumps,
                    },
                    window,
                    cx,
                )
            })
        })
        .unwrap()
    });
    (handle, view, transport)
}
fn emit_changed(
    cx: &mut TestAppContext,
    transport: &PromptTransport,
    unavailable: bool,
) -> futures::future::BoxFuture<'static, Option<HostKeyDecision>> {
    let (reply, answer) = Reply::channel();
    let driver = transport.0.lock().unwrap().as_ref().unwrap().clone();
    cx.background_executor
        .spawn(async move {
            driver
                .emit(Event::Prompt(Prompt::ChangedHostKey {
                    host: "host".into(),
                    port: 22,
                    algorithm: "ssh-ed25519".into(),
                    old_fingerprints: vec![
                        "SHA256:BSxf4FJAM6+KpCeKyk1kM0PG/OkbwTk79Pxvc12or4c".into(),
                    ],
                    fingerprint: "SHA256:qoUNJBOnhjgeRaLSuJzlFb/dbgesgwNUceBElNcALc8".into(),
                    known_hosts: "/home/example/.local/share/nocterm/ssh/known_hosts".into(),
                    line: 33,
                    replacement_error: unavailable
                        .then(|| "A conflicting key is in a read-only SSH trust file.".into()),
                    reply,
                    hop: None,
                }))
                .await;
        })
        .detach();
    cx.run_until_parked();
    async move { answer.await.ok() }.boxed()
}
#[gpui_kit::test]
fn changed_host_key_buttons_answer_exactly_once_and_restore_terminal_focus(
    cx: &mut TestAppContext,
) {
    let (handle, view, transport) = prompt_fixture(cx, None);
    for (button, decision) in [
        ("host-key-reject", HostKeyDecision::Reject),
        ("host-key-once", HostKeyDecision::AcceptOnce),
        ("host-key-remember", HostKeyDecision::AcceptAndRemember),
    ] {
        let mut answer = emit_changed(cx, &transport, false);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(view.read(cx).terminal.read(cx).prompt().is_some());
            window.click(button, cx);
            assert!(view.read(cx).terminal.read(cx).prompt().is_none());
            assert!(view.read(cx).focus_handle.is_focused(window));
        })
        .unwrap();
        assert_eq!((&mut answer).now_or_never().unwrap().unwrap(), decision);
    }
}
#[gpui_kit::test]
fn unsafe_changed_host_cannot_save_but_can_connect_once(cx: &mut TestAppContext) {
    let (handle, view, transport) = prompt_fixture(cx, None);
    let mut answer = emit_changed(cx, &transport, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("host-key-remember", cx);
        assert!((&mut answer).now_or_never().is_none());
        assert!(view.read(cx).terminal.read(cx).prompt().is_some());
        window.click("host-key-once", cx);
        assert!(view.read(cx).terminal.read(cx).prompt().is_none());
    })
    .unwrap();
    assert_eq!(
        (&mut answer).now_or_never().unwrap().unwrap(),
        HostKeyDecision::AcceptOnce
    );
}
#[gpui_kit::test]
fn operational_notices_are_event_driven_deduplicated_and_clear_credential_state(
    cx: &mut TestAppContext,
) {
    let store = Arc::new(nocterm_session::testing::FakeCredentialStore::default());
    store.fail_next(nocterm_session::testing::FakeCredentialError::NotFound);
    let provider = crate::CredentialProvider::new(store, |_, _, _| Task::ready(Ok(())));
    let (handle, view, _) = super::tests::fixture_with_credentials(cx, Some(provider));
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.terminal.update(cx, |t, cx| {
                let (reply, _) = nocterm_session::Reply::channel();
                t.handle_event(
                    Event::Prompt(Prompt::Secret {
                        request: SecretRequest::Password {
                            target: t.spec().target.clone(),
                            retry: false,
                        },
                        reply,
                        hop: None,
                    }),
                    cx,
                );
                t.answer_secret_and_remember(Secret::new("temporary test secret"), true, cx);
                t.handle_event(Event::Connected, cx);
            });
        });
        window.render_frame(cx);
        assert!(
            nocterm_ui::notice::count(window, cx) == 0,
            "render posted an operational notice"
        );
        view.update(cx, |v, cx| {
            v.terminal
                .update(cx, |_, cx| cx.emit(TerminalEvent::Changed));
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(nocterm_ui::notice::count(window, cx), 1);
        window.render_frame(cx);
        assert!(nocterm_ui::notice::run_action(
            window,
            cx,
            "terminal-credential"
        ));
    })
    .unwrap();
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, cx| {
        v.terminal.read(cx).credential_message().is_none()
    }));
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        nocterm_ui::notice::clear(window, cx);
        view.update(cx, |v, cx| {
            v.terminal.update(cx, |t, cx| t.start_recording(cx))
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(nocterm_ui::notice::count(window, cx), 1);
        nocterm_ui::notice::clear(window, cx);
        view.update(cx, |v, cx| {
            v.terminal
                .update(cx, |_, cx| cx.emit(TerminalEvent::Output))
        });
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            nocterm_ui::notice::count(window, cx) == 0,
            "unchanged error returned after dismissal"
        );
    })
    .unwrap();
}

fn bastion_hop() -> PromptHop {
    PromptHop {
        index: 0,
        target: Target::new("ops", "bastion", 22),
    }
}
fn emit_prompt(cx: &mut TestAppContext, transport: &PromptTransport, prompt: Prompt) {
    let driver = transport.0.lock().unwrap().as_ref().unwrap().clone();
    cx.background_executor
        .spawn(async move {
            driver.emit(Event::Prompt(prompt)).await;
        })
        .detach();
    cx.run_until_parked();
}
#[gpui_kit::test]
fn a_host_key_card_names_the_jump_host_only_for_a_hop(cx: &mut TestAppContext) {
    let (handle, _view, transport) = prompt_fixture(cx, None);
    let unknown = |hop| {
        let (reply, _answer) = Reply::channel();
        Prompt::UnknownHostKey {
            host: "bastion".into(),
            algorithm: "ssh-ed25519".into(),
            fingerprint: "SHA256:qoUNJBOnhjgeRaLSuJzlFb/dbgesgwNUceBElNcALc8".into(),
            hop,
            reply,
        }
    };
    emit_prompt(cx, &transport, unknown(Some(bastion_hop())));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("prompt-hop").is_some());
        window.click("host-key-reject", cx);
    })
    .unwrap();
    emit_prompt(cx, &transport, unknown(None));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("host-key-once").is_some());
        assert!(window.try_find("prompt-hop").is_none());
    })
    .unwrap();
}
#[gpui_kit::test]
fn a_jump_host_password_prompt_offers_no_remember_choice(cx: &mut TestAppContext) {
    let (handle, transport, _store) = credential_fixture(cx);
    let (reply, _answer) = Reply::channel();
    emit_prompt(
        cx,
        &transport,
        Prompt::Secret {
            request: SecretRequest::Password {
                target: bastion_hop().target,
                retry: false,
            },
            hop: Some(bastion_hop()),
            reply,
        },
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("secret-input").is_some());
        assert!(window.try_find("remember-credential").is_none());
    })
    .unwrap();
    let _destination = emit(cx, &transport, password());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("remember-credential").is_some());
    })
    .unwrap();
}
