use super::*;
use futures::future::BoxFuture;
use nocterm_ai::{AgentCommands, AgentError, CloseSessionOutcome};
use std::sync::atomic::{AtomicUsize, Ordering};

struct BlockedCommands {
    closes: AtomicUsize,
    shutdowns: AtomicUsize,
    slow_shutdown: bool,
    fail_shutdown: bool,
}
impl BlockedCommands {
    fn new(slow_shutdown: bool) -> Arc<Self> {
        Arc::new(Self {
            closes: AtomicUsize::new(0),
            shutdowns: AtomicUsize::new(0),
            slow_shutdown,
            fail_shutdown: false,
        })
    }
}
fn unused<T: 'static>() -> BoxFuture<'static, Result<T, AgentError>> {
    Box::pin(async { Err(AgentError::Io("unused test operation".into())) })
}
impl AgentCommands for BlockedCommands {
    fn new_session(
        &self,
        _: acp::NewSessionRequest,
    ) -> BoxFuture<'static, Result<acp::NewSessionResponse, AgentError>> {
        unused()
    }
    fn prompt(
        &self,
        _: acp::PromptRequest,
    ) -> BoxFuture<'static, Result<acp::PromptResponse, AgentError>> {
        unused()
    }
    fn cancel(&self, _: acp::SessionId) {}
    fn set_mode(
        &self,
        _: acp::SetSessionModeRequest,
    ) -> BoxFuture<'static, Result<(), AgentError>> {
        unused()
    }
    fn set_config_option(
        &self,
        _: acp::SetSessionConfigOptionRequest,
    ) -> BoxFuture<'static, Result<Vec<acp::SessionConfigOption>, AgentError>> {
        unused()
    }
    fn authenticate(&self, _: acp::AuthMethodId) -> BoxFuture<'static, Result<(), AgentError>> {
        unused()
    }
    fn close_session(
        &self,
        _: acp::SessionId,
    ) -> BoxFuture<'static, Result<CloseSessionOutcome, AgentError>> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        Box::pin(futures::future::pending())
    }
    fn shutdown(&self) {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
    }
    fn shutdown_gracefully(&self) -> BoxFuture<'static, Result<(), AgentError>> {
        self.shutdown();
        if self.slow_shutdown {
            Box::pin(futures::future::pending())
        } else if self.fail_shutdown {
            Box::pin(async { Err(AgentError::Io("cleanup was not confirmed".into())) })
        } else {
            Box::pin(async { Ok(()) })
        }
    }
}

#[gpui::test]
fn closing_cleanup_handle_is_retained_on_failure_and_released_on_success(cx: &mut TestAppContext) {
    for fails in [true, false] {
        let (runtime, directory) = fixture(cx);
        let mut commands = BlockedCommands::new(false);
        Arc::get_mut(&mut commands).unwrap().fail_shutdown = fails;
        runtime.update(cx, |runtime, cx| {
            runtime
                .connections
                .insert(1, connection(commands.clone(), directory.path().into()));
            runtime.release_lease(
                lease::ReleasedLease {
                    owner: cx.entity_id(),
                    session: None,
                    commands: Some(commands.clone()),
                    registration: None,
                    connection_key: 1,
                },
                cx,
            );
        });
        cx.run_until_parked();
        runtime.read_with(cx, |runtime, _| {
            assert_eq!(runtime.closing_commands.contains_key(&1), fails);
            assert_eq!(runtime.closing.contains_key(&1), fails);
        });
        assert_eq!(commands.shutdowns.load(Ordering::SeqCst), 1);
    }
}
fn connection(commands: Arc<dyn AgentCommands>, workdir: PathBuf) -> Connection {
    Connection {
        chat_id: "closing chat".into(),
        launch: AgentLaunch {
            id: "test".into(),
            name: "test".into(),
            command: "unused".into(),
            args: Vec::new(),
            env: Default::default(),
            inherit_env: Vec::new(),
        },
        workdir,
        isolated: false,
        commands: Some(commands),
        info: None,
        users: Default::default(),
        serial: 1,
        _events: None,
        _connecting: None,
        cancellation: Default::default(),
        startup_completion: None,
    }
}

#[gpui::test]
fn actual_quit_starts_active_and_already_closing_cleanup_without_waiting_for_close_ack(
    cx: &mut TestAppContext,
) {
    let (runtime, directory) = fixture(cx);
    let closing = BlockedCommands::new(false);
    let active = BlockedCommands::new(true);
    runtime.update(cx, |runtime, cx| {
        runtime
            .connections
            .insert(1, connection(closing.clone(), directory.path().into()));
        runtime.release_lease(
            lease::ReleasedLease {
                owner: cx.entity_id(),
                session: Some("session".into()),
                commands: Some(closing.clone()),
                registration: None,
                connection_key: 1,
            },
            cx,
        );
        runtime
            .connections
            .insert(2, connection(active.clone(), directory.path().into()));
    });
    cx.run_until_parked();
    assert_eq!(closing.closes.load(Ordering::SeqCst), 1);
    assert_eq!(closing.shutdowns.load(Ordering::SeqCst), 0);
    runtime.read_with(cx, |runtime, _| {
        assert_eq!(runtime.closing_commands.len(), 1)
    });
    cx.update(|cx| {
        cx.on_app_quit(move |cx| runtime.update(cx, |runtime, cx| runtime.shutdown(cx)))
            .detach();
    });
    cx.quit();
    assert_eq!(
        active.shutdowns.load(Ordering::SeqCst),
        1,
        "slow active cleanup must have started"
    );
    assert_eq!(
        closing.shutdowns.load(Ordering::SeqCst),
        1,
        "close's FIFO barrier must not delay quit cleanup"
    );
}
