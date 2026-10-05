use super::*;
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
fn superseded_secret_recreates_masked_empty_field_and_clears_old_text(cx: &mut TestAppContext) {
    let transport = Arc::new(PromptTransport::default());
    let (handle, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_ui::SettingsStore::in_memory(Default::default()),
            cx,
        );
        crate::init(transport.clone(), cx);
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
