use super::*;
use futures::{channel::oneshot, future::BoxFuture};
use nocterm_ai::{AgentCommands, AgentError, CloseSessionOutcome};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct BlockedCommands {
    closes: AtomicUsize,
    shutdowns: AtomicUsize,
    slow_shutdown: bool,
    fail_shutdown: bool,
    close_gate: Mutex<Option<oneshot::Receiver<()>>>,
    shutdown_gate: Mutex<Option<oneshot::Receiver<()>>>,
}
impl BlockedCommands {
    fn new(slow_shutdown: bool) -> Arc<Self> {
        Arc::new(Self {
            closes: AtomicUsize::new(0),
            shutdowns: AtomicUsize::new(0),
            slow_shutdown,
            fail_shutdown: false,
            close_gate: Mutex::new(None),
            shutdown_gate: Mutex::new(None),
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
        if let Some(gate) = self.close_gate.lock().unwrap().take() {
            Box::pin(async move {
                gate.await.unwrap();
                Ok(CloseSessionOutcome::Closed)
            })
        } else {
            Box::pin(futures::future::pending())
        }
    }
    fn shutdown(&self) {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
    }
    fn shutdown_gracefully(&self) -> BoxFuture<'static, Result<(), AgentError>> {
        self.shutdown();
        if let Some(gate) = self.shutdown_gate.lock().unwrap().take() {
            Box::pin(async move {
                gate.await.unwrap();
                Ok(())
            })
        } else if self.slow_shutdown {
            Box::pin(futures::future::pending())
        } else if self.fail_shutdown {
            Box::pin(async { Err(AgentError::Io("cleanup was not confirmed".into())) })
        } else {
            Box::pin(async { Ok(()) })
        }
    }
}

fn connected_client(
    runtime: &Entity<Runtime>,
    connector: &ScriptedConnector,
    commands: Arc<dyn AgentCommands>,
    cx: &mut TestAppContext,
) -> Rc<ProbeClient> {
    let client = ProbeClient::new(cx, snapshot());
    connector.push(Ok(nocterm_ai::AgentConnection {
        commands,
        events: async_channel::unbounded().1,
        info: nocterm_ai::AgentInfo {
            name: "test".into(),
            version: "1".into(),
            capabilities: Default::default(),
            auth_methods: Vec::new(),
        },
    }));
    runtime.update(cx, |runtime, cx| {
        runtime.register_document(client.clone(), cx);
        runtime.connect(
            client.clone(),
            client.chat.borrow().id.clone(),
            AgentLaunch {
                id: "codex".into(),
                name: "test".into(),
                command: "unused".into(),
                args: Vec::new(),
                env: Default::default(),
                inherit_env: Vec::new(),
            },
            cx,
        );
    });
    cx.run_until_parked();
    client
}
fn release_client(client: &ProbeClient, session: bool) {
    let mut events = client.events.borrow_mut();
    let index = events
        .iter()
        .position(|event| matches!(event, SessionEvent::Leased(_)))
        .expect("public Leased event");
    let SessionEvent::Leased(mut lease) = events.remove(index) else {
        unreachable!()
    };
    if session {
        lease.accept_session("session".into(), None);
    }
    events.clear();
    drop(lease);
}
#[gpui::test]
fn closing_cleanup_handle_is_retained_on_failure_and_released_on_success(cx: &mut TestAppContext) {
    for fails in [true, false] {
        let connector = Arc::new(ScriptedConnector::default());
        let (runtime, _directory, _repository) = fixture_with_connector(cx, connector.clone());
        let mut commands = BlockedCommands::new(false);
        Arc::get_mut(&mut commands).unwrap().fail_shutdown = fails;
        let retained = Arc::downgrade(&commands);
        let client = connected_client(&runtime, &connector, commands.clone(), cx);
        release_client(&client, false);
        cx.run_until_parked();
        assert_eq!(commands.shutdowns.load(Ordering::SeqCst), 1);
        let events = client.events.borrow();
        assert_eq!(
            events
                .iter()
                .any(|event| matches!(event, SessionEvent::CleanupUnconfirmed(_))),
            fails
        );
        assert_eq!(
            events
                .iter()
                .any(|event| matches!(event, SessionEvent::SessionClosed)),
            !fails
        );
        drop(events);
        drop(commands);
        assert_eq!(
            retained.upgrade().is_some(),
            fails,
            "failed cleanup must retain process ownership"
        );
    }
}
#[gpui::test]
fn actual_quit_starts_active_and_already_closing_cleanup_without_waiting_for_close_ack(
    cx: &mut TestAppContext,
) {
    let connector = Arc::new(ScriptedConnector::default());
    let (runtime, _directory, _repository) = fixture_with_connector(cx, connector.clone());
    let closing = BlockedCommands::new(false);
    let active = BlockedCommands::new(true);
    let first = connected_client(&runtime, &connector, closing.clone(), cx);
    release_client(&first, true);
    cx.run_until_parked();
    let _second = connected_client(&runtime, &connector, active.clone(), cx);
    assert_eq!(closing.closes.load(Ordering::SeqCst), 1);
    assert_eq!(closing.shutdowns.load(Ordering::SeqCst), 0);
    cx.update(|cx| {
        let shutdown = nocterm_store::ShutdownCoordinator::default();
        shutdown
            .register_capture(move |cx| {
                runtime.update(cx, |runtime, cx| runtime.capture_shutdown(cx))
            })
            .unwrap();
        cx.on_app_quit(move |cx| {
            let captured = shutdown.capture_all(cx);
            cx.background_executor().spawn(async move {
                captured.await;
            })
        })
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

#[gpui::test]
async fn shutdown_cleans_a_shared_command_handle_from_two_active_connections_once(
    cx: &mut TestAppContext,
) {
    let connector = Arc::new(ScriptedConnector::default());
    let (runtime, _directory, _repository) = fixture_with_connector(cx, connector.clone());
    let commands = BlockedCommands::new(false);
    let first = connected_client(&runtime, &connector, commands.clone(), cx);
    let second = connected_client(&runtime, &connector, commands.clone(), cx);
    for client in [&first, &second] {
        assert!(
            client.events.borrow().iter().any(|event| {
                matches!(event, SessionEvent::Connected { commands: accepted, .. }
                if Arc::ptr_eq(accepted, &(commands.clone() as Arc<dyn AgentCommands>)))
            }),
            "both connections must have accepted the supplied command handle"
        );
    }
    assert_eq!(connector.requests().len(), 2);
    assert_eq!(commands.shutdowns.load(Ordering::SeqCst), 0);
    runtime.update(cx, |runtime, cx| runtime.shutdown(cx)).await;
    assert_eq!(
        commands.shutdowns.load(Ordering::SeqCst),
        1,
        "one owned process handle must be cleaned once even when two connections share it",
    );
}

#[gpui::test]
async fn capture_joins_graceful_cleanup_already_running_without_starting_it_again(
    cx: &mut TestAppContext,
) {
    let connector = Arc::new(ScriptedConnector::default());
    let (runtime, _directory, _repository) = fixture_with_connector(cx, connector.clone());
    let commands = BlockedCommands::new(false);
    let (release, gate) = oneshot::channel();
    *commands.shutdown_gate.lock().unwrap() = Some(gate);
    let client = connected_client(&runtime, &connector, commands.clone(), cx);
    release_client(&client, false);
    cx.run_until_parked();
    assert_eq!(commands.shutdowns.load(Ordering::SeqCst), 1);
    let drain = runtime.update(cx, |runtime, cx| runtime.shutdown(cx));
    cx.run_until_parked();
    let calls_during_capture = commands.shutdowns.load(Ordering::SeqCst);
    release.send(()).unwrap();
    drain.await;
    cx.run_until_parked();
    assert_eq!(
        calls_during_capture, 1,
        "capture must join the existing graceful cleanup instead of invoking it twice",
    );
    assert_eq!(commands.shutdowns.load(Ordering::SeqCst), 1);
}

#[gpui::test]
async fn a_close_ack_after_explicit_shutdown_cannot_start_process_cleanup_again(
    cx: &mut TestAppContext,
) {
    let connector = Arc::new(ScriptedConnector::default());
    let (runtime, _directory, _repository) = fixture_with_connector(cx, connector.clone());
    let commands = BlockedCommands::new(false);
    let (release, gate) = oneshot::channel();
    *commands.close_gate.lock().unwrap() = Some(gate);
    let client = connected_client(&runtime, &connector, commands.clone(), cx);
    release_client(&client, true);
    cx.run_until_parked();
    assert_eq!(commands.closes.load(Ordering::SeqCst), 1);
    assert_eq!(commands.shutdowns.load(Ordering::SeqCst), 0);
    runtime.update(cx, |runtime, cx| runtime.shutdown(cx)).await;
    assert_eq!(
        commands.shutdowns.load(Ordering::SeqCst),
        1,
        "explicit shutdown must start cleanup without waiting for session close",
    );
    // Final ownership may cancel the ordinary close future; either outcome must
    // leave the one completed process cleanup intact.
    let _ = release.send(());
    cx.run_until_parked();
    assert_eq!(
        commands.shutdowns.load(Ordering::SeqCst),
        1,
        "the retained-App close continuation must not repeat completed process cleanup",
    );
}

#[gpui::test]
async fn final_capture_retries_a_previously_failed_normal_cleanup_once(cx: &mut TestAppContext) {
    let connector = Arc::new(ScriptedConnector::default());
    let (runtime, _directory, _repository) = fixture_with_connector(cx, connector.clone());
    let mut commands = BlockedCommands::new(false);
    Arc::get_mut(&mut commands).unwrap().fail_shutdown = true;
    let client = connected_client(&runtime, &connector, commands.clone(), cx);
    release_client(&client, false);
    cx.run_until_parked();
    assert_eq!(commands.shutdowns.load(Ordering::SeqCst), 1);
    assert!(
        client
            .events
            .borrow()
            .iter()
            .any(|event| { matches!(event, SessionEvent::CleanupUnconfirmed(_)) })
    );
    runtime.update(cx, |runtime, cx| runtime.shutdown(cx)).await;
    assert_eq!(
        commands.shutdowns.load(Ordering::SeqCst),
        2,
        "final capture must retry the retained failed normal attempt exactly once",
    );
    runtime.update(cx, |runtime, cx| runtime.shutdown(cx)).await;
    assert_eq!(commands.shutdowns.load(Ordering::SeqCst), 2);
}

#[gpui::test]
async fn a_late_close_joins_the_failed_final_retry_without_a_third_native_attempt(
    cx: &mut TestAppContext,
) {
    let connector = Arc::new(ScriptedConnector::default());
    let (runtime, _directory, _repository) = fixture_with_connector(cx, connector.clone());
    let mut commands = BlockedCommands::new(false);
    Arc::get_mut(&mut commands).unwrap().fail_shutdown = true;
    let first = connected_client(&runtime, &connector, commands.clone(), cx);
    let second = connected_client(&runtime, &connector, commands.clone(), cx);
    release_client(&first, false);
    cx.run_until_parked();
    assert_eq!(commands.shutdowns.load(Ordering::SeqCst), 1);
    let (release, gate) = oneshot::channel();
    *commands.close_gate.lock().unwrap() = Some(gate);
    release_client(&second, true);
    cx.run_until_parked();
    assert_eq!(commands.closes.load(Ordering::SeqCst), 1);
    assert_eq!(commands.shutdowns.load(Ordering::SeqCst), 1);
    runtime.update(cx, |runtime, cx| runtime.shutdown(cx)).await;
    assert_eq!(commands.shutdowns.load(Ordering::SeqCst), 2);
    let _ = release.send(());
    cx.run_until_parked();
    assert_eq!(
        commands.shutdowns.load(Ordering::SeqCst),
        2,
        "late ordinary close must join the failed final retry, never launch a third attempt",
    );
    assert!(
        second
            .events
            .borrow()
            .iter()
            .any(|event| { matches!(event, SessionEvent::CleanupUnconfirmed(_)) })
    );
}
