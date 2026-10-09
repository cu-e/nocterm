use super::*;
use futures::FutureExt as _;
use gpui_kit::{TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_session::{
    Auth, ConnectRequest, Event, Reply, Session, SessionDriver, Target, Transport,
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
fn emit(cx: &mut TestAppContext, driver: Arc<SessionDriver>, request: SecretRequest) {
    let (reply, _answer) = Reply::channel();
    cx.background_executor
        .spawn(async move {
            driver
                .emit(Event::Prompt(Prompt::Secret { request, reply }))
                .await;
        })
        .detach();
    cx.run_until_parked();
}
#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn superseded_secret_recreates_masked_empty_field_and_clears_old_text(cx: &mut TestAppContext) {
    let transport = Arc::new(PromptTransport::default());
    let (handle, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        let ui = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_ui::SettingsStore::in_memory(Default::default()),
            cx,
        );
        crate::init(transport.clone(), &ui, cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let view = cx.new(|cx| {
                TerminalView::new(
                    SessionSpec {
                        profile: None,
                        options: Default::default(),
                        title: "test".into(),
                        target: Target::new("me", "host", 22),
                        auth: Auth::Password,
                        credential: None,
                        launch: None,
                    },
                    window,
                    cx,
                )
            });
            window.focus(&view.read(cx).focus_handle(cx), cx);
            view
        })
        .unwrap()
    });
    let driver = transport.0.lock().unwrap().as_ref().unwrap().clone();
    emit(
        cx,
        driver.clone(),
        SecretRequest::Interactive {
            prompt: "Visible code".into(),
            echo: true,
        },
    );
    let old_input = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.input("previous answer", cx);
            view.update(cx, |view, cx| {
                let field = view.secret.as_mut().unwrap();
                field.remember = true;
                assert!(!field.input.read(cx).presentation().is_masked());
                assert_eq!(field.input.read(cx).value(), "previous answer");
                field.input.clone()
            })
        })
        .unwrap();
    emit(
        cx,
        driver.clone(),
        SecretRequest::Password {
            target: Target::new("me", "host", 22),
            retry: false,
        },
    );
    let password_input = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let field = view.read(cx).secret.as_ref().unwrap();
            assert_ne!(field.input.entity_id(), old_input.entity_id());
            assert!(field.input.read(cx).presentation().is_masked());
            assert!(field.input.read(cx).value().is_empty());
            assert!(!field.remember);
            assert!(field.input.read(cx).focus_handle(cx).is_focused(window));
            assert!(old_input.read(cx).value().is_empty());
            field.input.clone()
        })
        .unwrap();
    // Even the same question is a new authentication attempt.
    emit(
        cx,
        driver,
        SecretRequest::Password {
            target: Target::new("me", "host", 22),
            retry: false,
        },
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let field = view.read(cx).secret.as_ref().unwrap();
        assert_ne!(field.input.entity_id(), password_input.entity_id());
        window.input("fresh password", cx);
        window.press("ctrl-z", cx);
        assert_ne!(
            view.read(cx)
                .secret
                .as_ref()
                .unwrap()
                .input
                .read(cx)
                .value(),
            "previous answer"
        );
    })
    .unwrap();
}

fn host_fixture(
    cx: &mut TestAppContext,
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
            nocterm_ui::SettingsStore::in_memory(Default::default()),
            cx,
        );
        crate::init(transport.clone(), &ui, cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                TerminalView::new(
                    SessionSpec {
                        profile: None,
                        options: Default::default(),
                        title: "test".into(),
                        target: Target::new("me", "host", 22),
                        auth: Auth::Password,
                        credential: None,
                        launch: None,
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
    let (handle, view, transport) = host_fixture(cx);
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
    let (handle, view, transport) = host_fixture(cx);
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
