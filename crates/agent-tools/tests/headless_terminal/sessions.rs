use crate::headless_support::*;
use nocterm_agent_tools::headless::HeadlessTerminalOptions;
use nocterm_ai::{OpenTerminal, SendInput, TerminalCall};
use nocterm_cloud_model::{
    authority::{AuthorityError, ExecutionAuthority, RunBinding, SessionReceipt},
    identity::*,
    payload::BoundedJson,
    tool::{
        BackgroundCommand, ScopedToolExecutor, ToolEffectStatus, ToolExecutionError,
        ToolExecutionResult,
    },
};
use nocterm_session::{Charset, Event, testing::Gate};

#[test]
fn list_and_open_only_the_original_offered_target_and_receipt() {
    let mut host = Harness::new(Charset::Utf8);
    let handle = RemoteSessionId::from_bytes([9; 16]);
    let server = host
        .tools
        .offer(
            handle,
            host.authority.session().clone(),
            HeadlessTerminalOptions::default(),
        )
        .unwrap();
    let list = host.call(1, TerminalCall::ListTerminals).unwrap();
    assert_eq!(
        list.output().value()["terminals"][0]["terminal_id"],
        host.terminal
    );
    assert_eq!(
        list.output().value()["offline_servers"][0]["server_id"],
        server
    );
    let (session, driver) = nocterm_session::channel(None);
    assert!(host.pool.run_until(driver.emit(Event::Connected)));
    host.opener.queue(host.authority.session().clone(), session);
    let pending = host.start(host.request(
        2,
        TerminalCall::OpenTerminal(OpenTerminal {
            server_id: server.clone(),
        }),
    ));
    host.tick(25);
    let output = host.finish(pending).unwrap();
    assert_eq!(output.output().value()["terminal_id"], server);
    assert_eq!(host.opener.requests().len(), 1);
    let list = host.call(3, TerminalCall::ListTerminals).unwrap();
    assert_eq!(
        list.output().value()["terminals"].as_array().unwrap().len(),
        2
    );
    assert!(
        list.output().value()["offline_servers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        host.call(
            4,
            TerminalCall::OpenTerminal(OpenTerminal {
                server_id: "model-chosen-target".into()
            })
        )
        .is_err()
    );
    assert_eq!(host.opener.requests().len(), 1);
}

#[test]
fn changed_selected_credential_session_epoch_and_run_are_denied_before_io() {
    let mut host = Harness::new(Charset::Utf8);
    let original = host.authority.clone();
    let session = original.session();
    let stale = SessionReceipt::new(
        session.id(),
        session.epoch().next().unwrap(),
        session.target().clone(),
        session.authentication().clone(),
    );
    let stale = ExecutionAuthority::new(
        original.run().clone(),
        stale,
        original.grant(),
        original.grant_epoch(),
    );
    let call = TerminalCall::SendInput(SendInput {
        terminal_id: host.terminal.clone(),
        text: "never".into(),
        press_enter: true,
    });
    let stale_request = request(&stale, 1, call.clone());
    assert!(matches!(
        host.pool.run_until(host.tools.execute(stale_request)),
        Err(ToolExecutionError::Authority(AuthorityError::StaleSession))
    ));
    let changed = SessionReceipt::new(
        session.id(),
        session.epoch(),
        session.target().clone(),
        nocterm_cloud_model::authority::AuthenticationBinding::new(
            session.authentication().key(),
            fingerprint(99),
        ),
    );
    let changed = ExecutionAuthority::new(
        original.run().clone(),
        changed,
        original.grant(),
        original.grant_epoch(),
    );
    let request = request(&changed, 2, call);
    assert!(matches!(
        host.pool.run_until(host.tools.execute(request)),
        Err(ToolExecutionError::Authority(AuthorityError::StaleSession))
    ));
    let foreign = RunBinding::new(
        original.run().scope().clone(),
        original.run().runner(),
        original.run().runner_epoch(),
        CloudChatId::from_bytes([99; 16]),
        original.run().run(),
        original.run().epoch(),
    );
    assert!(matches!(
        host.registry.lock().unwrap().list(&foreign),
        Err(nocterm_agent_tools::headless::HeadlessError::Authority(
            AuthorityError::StaleRun
        ))
    ));
    assert!(host.executor.requests().is_empty());
    assert!(futures::FutureExt::now_or_never(host.driver.next_command()).is_none());
}

#[test]
fn expiration_during_open_closes_new_session_instead_of_attaching_it() {
    let mut host = Harness::new(Charset::Utf8);
    let server = host
        .tools
        .offer(
            RemoteSessionId::from_bytes([9; 16]),
            host.authority.session().clone(),
            HeadlessTerminalOptions::default(),
        )
        .unwrap();
    let gate = Gate::pending();
    let waiting = gate.clone();
    host.opener.suspend(async move { waiting.wait().await });
    let (session, driver) = nocterm_session::channel(None);
    host.opener.queue(host.authority.session().clone(), session);
    let pending = host.start(host.request(
        1,
        TerminalCall::OpenTerminal(OpenTerminal { server_id: server }),
    ));
    assert_eq!(host.opener.requests().len(), 1);
    host.checks.refuse(ToolExecutionError::Expired);
    gate.release();
    host.pump();
    assert!(matches!(
        host.finish(pending),
        Err(ToolExecutionError::Expired)
    ));
    assert!(futures::FutureExt::now_or_never(driver.closed()).is_some());
    assert_eq!(
        host.registry
            .lock()
            .unwrap()
            .list(host.authority.run())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn expiration_while_waiting_for_connection_closes_the_attached_session() {
    let mut host = Harness::new(Charset::Utf8);
    let server = host
        .tools
        .offer(
            RemoteSessionId::from_bytes([9; 16]),
            host.authority.session().clone(),
            HeadlessTerminalOptions::default(),
        )
        .unwrap();
    let (session, driver) = nocterm_session::channel(None);
    host.opener.queue(host.authority.session().clone(), session);
    let pending = host.start(host.request(
        1,
        TerminalCall::OpenTerminal(OpenTerminal { server_id: server }),
    ));
    assert_eq!(
        host.registry
            .lock()
            .unwrap()
            .list(host.authority.run())
            .unwrap()
            .len(),
        2
    );
    host.checks.refuse(ToolExecutionError::Expired);
    host.tick(25);
    assert!(matches!(
        host.finish(pending),
        Err(ToolExecutionError::Expired)
    ));
    assert!(
        futures::FutureExt::now_or_never(driver.closed()).is_some(),
        "an expired connection attempt must close its already attached transport"
    );
    assert_eq!(
        host.registry
            .lock()
            .unwrap()
            .list(host.authority.run())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn opener_cannot_substitute_a_different_session_receipt() {
    let mut host = Harness::new(Charset::Utf8);
    let server = host
        .tools
        .offer(
            RemoteSessionId::from_bytes([9; 16]),
            host.authority.session().clone(),
            HeadlessTerminalOptions::default(),
        )
        .unwrap();
    let original = host.authority.session();
    let different = SessionReceipt::new(
        RemoteSessionId::from_bytes([99; 16]),
        original.epoch(),
        original.target().clone(),
        original.authentication().clone(),
    );
    let (session, driver) = nocterm_session::channel(None);
    host.opener.queue(different, session);
    assert!(matches!(
        host.call(
            1,
            TerminalCall::OpenTerminal(OpenTerminal { server_id: server })
        ),
        Err(ToolExecutionError::StaleSession)
    ));
    assert!(futures::FutureExt::now_or_never(driver.closed()).is_some());
}

#[test]
fn result_classification_cannot_rebind_another_intent_or_background_command() {
    let host = Harness::new(Charset::Utf8);
    let one = host.request(1, host.exec("one"));
    let two = host.request(2, host.exec("two"));
    let result = ToolExecutionResult::completed(
        &one,
        BoundedJson::new(serde_json::json!({"accepted":true})).unwrap(),
        None,
    );
    assert!(
        result
            .clone()
            .with_effect_status(&two, ToolEffectStatus::Unknown)
            .is_err()
    );
    let background = BackgroundCommand::new(
        two.permit().clone(),
        host.terminal.clone(),
        "command".into(),
    )
    .unwrap();
    assert!(
        result
            .clone()
            .with_effect_status(
                &one,
                ToolEffectStatus::Background(std::sync::Arc::new(background))
            )
            .is_err()
    );
    let wrong_terminal = BackgroundCommand::new(
        one.permit().clone(),
        "another-terminal".into(),
        "command".into(),
    )
    .unwrap();
    assert!(
        result
            .with_effect_status(
                &one,
                ToolEffectStatus::Background(std::sync::Arc::new(wrong_terminal))
            )
            .is_err()
    );
}

#[test]
fn oversized_geometry_is_refused_before_allocating_a_terminal() {
    let host = Harness::new(Charset::Utf8);
    let (session, driver) = nocterm_session::channel(None);
    let invalid = HeadlessTerminalOptions {
        size: nocterm_session::PtySize {
            cols: u16::MAX,
            rows: u16::MAX,
            pixel_width: 0,
            pixel_height: 0,
        },
        charset: Charset::Utf8,
    };
    let result = host.registry.lock().unwrap().attach(
        RemoteSessionId::from_bytes([9; 16]),
        host.authority.session().clone(),
        session,
        invalid,
        host.timer.clone(),
    );
    assert!(matches!(
        result,
        Err(nocterm_agent_tools::headless::HeadlessError::InvalidGeometry)
    ));
    assert!(futures::FutureExt::now_or_never(driver.closed()).is_some());
}

#[test]
fn refused_session_task_closes_and_removes_the_new_native_session() {
    let mut host = Harness::new(Charset::Utf8);
    let server = host
        .tools
        .offer(
            RemoteSessionId::from_bytes([9; 16]),
            host.authority.session().clone(),
            HeadlessTerminalOptions::default(),
        )
        .unwrap();
    let (session, driver) = nocterm_session::channel(None);
    host.opener.queue(host.authority.session().clone(), session);
    host.tasks.refuse(ToolExecutionError::Capacity);
    assert!(matches!(
        host.call(
            1,
            TerminalCall::OpenTerminal(OpenTerminal { server_id: server })
        ),
        Err(ToolExecutionError::Capacity)
    ));
    assert!(futures::FutureExt::now_or_never(driver.closed()).is_some());
    assert_eq!(
        host.registry
            .lock()
            .unwrap()
            .list(host.authority.run())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn dropping_an_open_waiter_closes_candidate_and_interrupts_its_idle_event_task() {
    use futures::FutureExt as _;
    let host = Harness::new(Charset::Utf8);
    let server = host
        .tools
        .offer(
            RemoteSessionId::from_bytes([9; 16]),
            host.authority.session().clone(),
            HeadlessTerminalOptions::default(),
        )
        .unwrap();
    let (session, driver) = nocterm_session::channel(None);
    host.opener.queue(host.authority.session().clone(), session);
    let request = host.request(
        1,
        TerminalCall::OpenTerminal(OpenTerminal { server_id: server }),
    );
    let mut waiter = host.tools.execute(request);
    assert!(waiter.as_mut().now_or_never().is_none());
    assert_eq!(
        host.registry
            .lock()
            .unwrap()
            .list(host.authority.run())
            .unwrap()
            .len(),
        2
    );
    let mut tasks = host.tasks.take();
    assert_eq!(tasks.len(), 1);
    let mut watcher = tasks.pop().unwrap();
    assert!(watcher.as_mut().now_or_never().is_none());
    drop(waiter);
    assert!(driver.closed().now_or_never().is_some());
    assert_eq!(
        host.registry
            .lock()
            .unwrap()
            .list(host.authority.run())
            .unwrap()
            .len(),
        1
    );
    assert!(
        matches!(watcher.now_or_never(), Some(Ok(()))),
        "retirement does not wait for a transport event"
    );
}

#[test]
fn a_stale_open_waiter_never_removes_a_replacement_under_the_same_handle() {
    use futures::FutureExt as _;
    let host = Harness::new(Charset::Utf8);
    let handle = RemoteSessionId::from_bytes([9; 16]);
    let server = host
        .tools
        .offer(
            handle,
            host.authority.session().clone(),
            HeadlessTerminalOptions::default(),
        )
        .unwrap();
    let (session, original) = nocterm_session::channel(None);
    host.opener.queue(host.authority.session().clone(), session);
    let mut waiter = host.tools.execute(host.request(
        1,
        TerminalCall::OpenTerminal(OpenTerminal {
            server_id: server.clone(),
        }),
    ));
    assert!(waiter.as_mut().now_or_never().is_none());
    host.registry.lock().unwrap().remove(&server).unwrap();
    let (session, replacement) = nocterm_session::channel(None);
    let task = host
        .registry
        .lock()
        .unwrap()
        .attach(
            handle,
            host.authority.session().clone(),
            session,
            HeadlessTerminalOptions::default(),
            host.timer.clone(),
        )
        .unwrap();
    drop(waiter);
    assert!(original.closed().now_or_never().is_some());
    assert!(replacement.closed().now_or_never().is_none());
    assert_eq!(
        host.registry
            .lock()
            .unwrap()
            .list(host.authority.run())
            .unwrap()
            .len(),
        2
    );
    drop(task);
    host.registry.lock().unwrap().remove(&server).unwrap();
}

#[test]
fn connection_timeout_and_revocation_close_the_original_candidate() {
    for revoked in [false, true] {
        let mut host = Harness::new(Charset::Utf8);
        let server = host
            .tools
            .offer(
                RemoteSessionId::from_bytes([9; 16]),
                host.authority.session().clone(),
                HeadlessTerminalOptions::default(),
            )
            .unwrap();
        let (session, driver) = nocterm_session::channel(None);
        host.opener.queue(host.authority.session().clone(), session);
        let pending = host.start(host.request(
            1,
            TerminalCall::OpenTerminal(OpenTerminal { server_id: server }),
        ));
        if revoked {
            host.checks.refuse(ToolExecutionError::Revoked);
            host.tick(25);
        } else {
            host.tick(30_000);
        }
        let result = host.finish(pending);
        assert!(matches!(
            (&result, revoked),
            (Err(ToolExecutionError::Revoked), true)
                | (Err(ToolExecutionError::OutcomeUnknown), false)
        ));
        assert!(futures::FutureExt::now_or_never(driver.closed()).is_some());
        assert_eq!(
            host.registry
                .lock()
                .unwrap()
                .list(host.authority.run())
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn a_closed_login_requiring_fresh_consent_reports_needs_unlock_and_cleans_its_attempt() {
    use nocterm_session::{CloseReason, ConnectionFailure, ConnectionFailureKind};
    let mut host = Harness::new(Charset::Utf8);
    let server = host
        .tools
        .offer(
            RemoteSessionId::from_bytes([9; 16]),
            host.authority.session().clone(),
            HeadlessTerminalOptions::default(),
        )
        .unwrap();
    let (session, driver) = nocterm_session::channel(None);
    let failed = ConnectionFailure::new(
        ConnectionFailureKind::NeedsUnlock,
        std::io::Error::from(std::io::ErrorKind::PermissionDenied),
    );
    assert!(
        host.pool
            .run_until(driver.emit(Event::Closed(CloseReason::Failed(failed.into()))))
    );
    host.opener.queue(host.authority.session().clone(), session);
    let request = host.request(
        2,
        TerminalCall::OpenTerminal(OpenTerminal { server_id: server }),
    );
    let pending = host.start(request);
    host.tick(25);
    let error = host.finish(pending).unwrap_err();
    assert!(error.is_needs_unlock());
    let ToolExecutionError::Execution(cause) = error else {
        panic!("native typed cause was discarded")
    };
    assert!(
        matches!(cause.downcast_ref::<nocterm_session::SessionError>(),Some(nocterm_session::SessionError::Connection(error)) if error.kind()==ConnectionFailureKind::NeedsUnlock)
    );
    assert!(
        futures::FutureExt::now_or_never(driver.closed()).is_some(),
        "closed failure must not retain a pending physical auth session"
    );
    assert_eq!(
        host.registry
            .lock()
            .unwrap()
            .list(host.authority.run())
            .unwrap()
            .len(),
        1,
        "only the original unrelated session remains"
    );
}

#[test]
fn closed_native_denials_unsupported_and_source_failures_keep_their_typed_cause() {
    use nocterm_session::{
        CloseReason, ConnectionFailure, ConnectionFailureKind, ConnectionSourceError, SessionError,
    };
    let cases = [
        ConnectionFailure::new(
            ConnectionFailureKind::Denied,
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        )
        .into(),
        ConnectionFailure::new(
            ConnectionFailureKind::Unsupported,
            std::io::Error::from(std::io::ErrorKind::Unsupported),
        )
        .into(),
        SessionError::Source(ConnectionSourceError::Revoked),
    ];
    for failure in cases {
        let mut host = Harness::new(Charset::Utf8);
        let server = host
            .tools
            .offer(
                RemoteSessionId::from_bytes([9; 16]),
                host.authority.session().clone(),
                HeadlessTerminalOptions::default(),
            )
            .unwrap();
        let (session, driver) = nocterm_session::channel(None);
        assert!(
            host.pool
                .run_until(driver.emit(Event::Closed(CloseReason::Failed(failure.clone()))))
        );
        host.opener.queue(host.authority.session().clone(), session);
        let request = host.request(
            2,
            TerminalCall::OpenTerminal(OpenTerminal { server_id: server }),
        );
        let pending = host.start(request);
        host.tick(25);
        let error = host.finish(pending).unwrap_err();
        assert!(!error.is_needs_unlock());
        let ToolExecutionError::Execution(cause) = error else {
            panic!("native failure became a stale-session placeholder")
        };
        assert_eq!(cause.downcast_ref::<SessionError>(), Some(&failure));
        assert!(futures::FutureExt::now_or_never(driver.closed()).is_some());
    }
}
