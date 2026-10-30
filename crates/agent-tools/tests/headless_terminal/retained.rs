use crate::headless_support::*;
use futures::{FutureExt as _, task::LocalSpawnExt as _};
use nocterm_ai::{CancelCommand, ReadCommand, RunCommand, SendInput, TerminalCall};
use nocterm_cloud_model::{
    authority::RunBinding,
    identity::{RemoteSessionId, RunEpoch, RunId},
    tool::{BackgroundCommandOutcome, ScopedToolExecutor, ToolExecutionError},
};
use nocterm_session::{Charset, Command, ExecExit};
use std::sync::Arc;

fn live(terminal: &str) -> TerminalCall {
    TerminalCall::RunCommand(RunCommand {
        terminal_id: terminal.into(),
        command: "echo retained".into(),
        timeout_ms: None,
        idle_ms: None,
    })
}
fn read_command(terminal: &str, command: &str) -> TerminalCall {
    TerminalCall::ReadCommand(ReadCommand {
        terminal_id: terminal.into(),
        command_id: command.into(),
        yield_ms: None,
    })
}
#[test]
fn next_prompt_keeps_one_physical_screen_and_old_view_cannot_close_it() {
    let (mut host, retained, _) = Harness::retained(Charset::Utf8);
    host.emit(b"before\r\n\x1b]133;A\x07$ ");
    let original = host.authority.clone();
    let old_terminal = host.terminal.clone();
    let old_request = host.request(
        1,
        TerminalCall::SendInput(SendInput {
            terminal_id: old_terminal,
            text: "old".into(),
            press_enter: false,
        }),
    );
    let (old_tools, old_registry) = host.next_run(&retained, 9);
    old_tools.detach();
    assert!(
        old_registry
            .lock()
            .unwrap()
            .remove(old_request.call().terminal_id().unwrap())
            .is_err()
    );
    drop(old_tools);
    drop(old_registry);
    assert_eq!(host.authority.session(), original.session());
    assert!(
        host.tools
            .execute(old_request)
            .now_or_never()
            .unwrap()
            .is_err()
    );
    host.emit(b"after");
    assert_eq!(host.read(2, None)["text"], "before\n$ after");
    let pending = host.start(host.request(3, live(&host.terminal)));
    assert_eq!(
        host.pool.run_until(host.driver.next_command()),
        Some(Command::Input(b"echo retained\r".to_vec()))
    );
    host.emit(b"\x1b]133;C\x07done\r\n\x1b]133;A\x07");
    host.tick(100);
    assert_eq!(
        host.finish(pending).unwrap().output().value()["completed"],
        true
    );
    retained.close();
    assert!(host.driver.closed().now_or_never().is_some());
    assert!(host.call(4, host.exec("closed")).is_err());
}
#[test]
fn admitted_background_survives_completed_view_with_original_ids_and_shared_budget() {
    let (mut host, retained, _) = Harness::retained(Charset::Utf8);
    let sink = host.executor.queue_stream("long");
    let started = host.call(1, host.exec("long")).unwrap();
    let command = started.output().value()["command_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let original = host.journal.observations()[0].0.clone();
    let (old_tools, old_registry) = host.next_run(&retained, 9);
    old_tools.detach();
    drop(old_tools);
    drop(old_registry);
    assert!(host.pool.run_until(sink.send(b"retained output".to_vec())));
    host.pump();
    let read = host
        .call(2, read_command(&host.terminal, &command))
        .unwrap();
    assert_eq!(read.output().value()["stdout"], "retained output");
    sink.finish(ExecExit {
        status: Some(7),
        stderr: "original stderr".into(),
    });
    drop(sink);
    host.pump();
    assert_eq!(
        host.journal.observations(),
        [(original, Some(BackgroundCommandOutcome::ObservedExit(7)))]
    );
    assert_eq!(
        host.call(3, read_command(&host.terminal, &command))
            .unwrap()
            .output()
            .value()["exit_status"],
        7
    );
}
#[test]
fn original_background_revoke_stops_observation_without_retargeting_new_run() {
    let (mut host, retained, observations) = Harness::retained(Charset::Utf8);
    let sink = host.executor.queue_stream("long");
    host.call(1, host.exec("long")).unwrap();
    let (old_tools, _) = host.next_run(&retained, 9);
    old_tools.detach();
    observations.refuse(ToolExecutionError::Expired);
    host.tick(50);
    assert!(sink.is_closed());
    assert_eq!(
        host.journal.observations()[0].1,
        Some(BackgroundCommandOutcome::Unknown)
    );
    assert!(host.driver.closed().now_or_never().is_none());
}
#[test]
fn new_run_cancels_only_the_same_physical_original_command() {
    let (mut host, retained, _) = Harness::retained(Charset::Utf8);
    let sink = host.executor.queue_stream("long");
    let result = host.call(1, host.exec("long")).unwrap();
    let command = result.output().value()["command_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let original = host.journal.observations()[0].0.clone();
    let (old_tools, _) = host.next_run(&retained, 9);
    old_tools.detach();
    host.call(
        2,
        TerminalCall::CancelCommand(CancelCommand {
            terminal_id: host.terminal.clone(),
            command_id: command,
        }),
    )
    .unwrap();
    host.pump();
    assert!(sink.is_closed());
    assert_eq!(
        host.journal.observations(),
        [(original, Some(BackgroundCommandOutcome::Unknown))]
    );
}
#[test]
fn old_live_waiter_drop_cannot_release_new_live_owner() {
    assert_old_drop_keeps_new_live_owner(9);
    assert_old_drop_keeps_new_live_owner(3);
}
fn assert_old_drop_keeps_new_live_owner(next_run: u8) {
    let (mut host, retained, _) = Harness::retained(Charset::Utf8);
    host.emit(b"\x1b]133;A\x07");
    let old = host.request(1, live(&host.terminal));
    let mut observer = host.tools.execute(old);
    assert!(observer.as_mut().now_or_never().is_none());
    host.emit(b"\x1b]133;C\x07old done\r\n\x1b]133;A\x07");
    let (old_tools, _) = host.next_run(&retained, next_run);
    old_tools.detach();
    // Reuse intent bytes across different runs, then the SAME RunBinding with
    // a new grant/runtime. Caller coordinates alone cannot own a live lease.
    let pending = host.start(host.request(1, live(&host.terminal)));
    drop(observer);
    let input = TerminalCall::SendInput(SendInput {
        terminal_id: host.terminal.clone(),
        text: "cannot interleave".into(),
        press_enter: false,
    });
    assert!(host.call(3, input).is_err());
    host.emit(b"\x1b]133;C\x07new done\r\n\x1b]133;A\x07");
    host.tick(100);
    assert_eq!(
        host.finish(pending).unwrap().output().value()["completed"],
        true
    );
}
#[test]
fn retained_views_reject_foreign_chat_and_receipt_before_native_io() {
    let (host, retained, _) = Harness::retained(Charset::Utf8);
    let original = host.authority.run();
    let foreign = RunBinding::new(
        original.scope().clone(),
        original.runner(),
        original.runner_epoch(),
        nocterm_cloud_model::identity::CloudChatId::from_bytes([99; 16]),
        RunId::from_bytes([9; 16]),
        RunEpoch::new(1).unwrap(),
    );
    assert!(retained.view(foreign).is_err());
    // Identical IDs on another hub/account-key partition cannot borrow this PTY.
    use nocterm_cloud_model::identity::{AccountIdentity, AccountScope, HubIdentity};
    let scope = original.scope();
    for foreign_scope in [
        AccountScope::new(
            HubIdentity::new(scope.hub().instance().clone(), fingerprint(99)),
            scope.account().clone(),
        ),
        AccountScope::new(
            scope.hub().clone(),
            AccountIdentity::new(scope.account().id().clone(), fingerprint(99)),
        ),
    ] {
        let foreign = RunBinding::new(
            foreign_scope,
            original.runner(),
            original.runner_epoch(),
            original.chat(),
            original.run(),
            original.epoch(),
        );
        assert!(retained.view(foreign).is_err());
    }
    let mut view = retained.view(original.clone()).unwrap();
    let receipt = host.authority.session();
    let different = nocterm_cloud_model::authority::SessionReceipt::new(
        RemoteSessionId::from_bytes([99; 16]),
        receipt.epoch(),
        receipt.target().clone(),
        receipt.authentication().clone(),
    );
    assert!(
        view.attach_retained(RemoteSessionId::from_bytes([9; 16]), &different)
            .is_err()
    );
    assert!(host.executor.requests().is_empty());
}

#[test]
fn completed_view_cannot_start_an_unpolled_original_background_dispatch() {
    let (mut host, retained, _) = Harness::retained(Charset::Utf8);
    let request = host.request(1, host.exec("never"));
    host.pool.run_until(host.tools.execute(request)).unwrap();
    assert!(host.executor.requests().is_empty());
    let (old_tools, _) = host.next_run(&retained, 9);
    old_tools.detach();
    host.pump();
    assert!(host.executor.requests().is_empty());
    assert_eq!(
        host.journal.observations()[0].1,
        Some(BackgroundCommandOutcome::Unknown)
    );
}
#[test]
fn retained_job_budget_and_new_grants_are_not_reset_at_the_next_prompt() {
    let (mut host, retained, _) = Harness::retained(Charset::Utf8);
    let sinks: Vec<_> = (1..=4)
        .map(|id| {
            let sink = host.executor.queue_stream("long");
            host.call(id, host.exec("long")).unwrap();
            sink
        })
        .collect();
    let (old_tools, _) = host.next_run(&retained, 9);
    old_tools.detach();
    assert!(matches!(
        host.call(5, host.exec("fifth")),
        Err(ToolExecutionError::Capacity)
    ));
    assert_eq!(host.executor.requests().len(), 4);
    host.checks.refuse(ToolExecutionError::Expired);
    assert!(matches!(
        host.call(
            6,
            TerminalCall::SendInput(SendInput {
                terminal_id: host.terminal.clone(),
                text: "forbidden".into(),
                press_enter: true
            })
        ),
        Err(ToolExecutionError::Expired)
    ));
    assert!(host.driver.next_command().now_or_never().is_none());
    drop(sinks);
}
#[test]
fn dropping_the_last_retained_owner_closes_physical_sessions_even_with_live_views() {
    let (host, retained, _) = Harness::retained(Charset::Utf8);
    drop(retained);
    assert!(host.driver.closed().now_or_never().is_some());
}

#[test]
fn retained_factory_attaches_without_login_and_completion_preserves_the_physical_owner() {
    use nocterm_agent_tools::headless::RetainedToolsFactory;
    use nocterm_cloud_model::{
        testing::ScriptedSessionHandles,
        tool::{RunRetirement, RunToolRuntimeFactory},
    };
    use std::sync::Arc;
    let (mut host, retained, _) = Harness::retained(Charset::Utf8);
    let factory = RetainedToolsFactory::new(
        retained,
        host.timer.clone(),
        host.journal.clone(),
        host.tasks.clone(),
        host.opener.clone(),
        Arc::new(ScriptedSessionHandles::new([
            RemoteSessionId::from_bytes([9; 16]),
            RemoteSessionId::from_bytes([10; 16]),
        ])),
    );
    let runtime = host
        .pool
        .run_until(factory.attach(
            host.authority.run().clone(),
            host.authority.session().clone(),
            host.checks.clone(),
        ))
        .unwrap();
    let result = host
        .pool
        .run_until(runtime.execute(host.request(1, TerminalCall::ListTerminals)))
        .unwrap();
    assert_eq!(
        result.output().value()["terminals"][0]["terminal_id"],
        "t-09090909090909090909090909090909"
    );
    let server = result.output().value()["terminals"][0]["server_id"]
        .as_str()
        .unwrap();
    for id in [2, 3] {
        let opened = host
            .pool
            .run_until(runtime.execute(host.request(
                id,
                TerminalCall::OpenTerminal(nocterm_ai::OpenTerminal {
                    server_id: server.into(),
                }),
            )))
            .unwrap();
        assert_eq!(
            opened.output().value()["terminal_id"],
            result.output().value()["terminals"][0]["terminal_id"]
        );
    }
    assert!(host.tasks.take().is_empty());
    assert!(host.opener.requests().is_empty());
    host.pool
        .run_until(runtime.retire(RunRetirement::Completed))
        .unwrap();
    assert!(runtime.check_current().is_err());
    assert!(host.driver.closed().now_or_never().is_none());
    host.pool
        .run_until(host.driver.emit(nocterm_session::Event::Closed(
            nocterm_session::CloseReason::ClosedByUser,
        )));
    host.pump();
    assert!(matches!(
        host.pool.run_until(factory.attach(
            host.authority.run().clone(),
            host.authority.session().clone(),
            host.checks.clone()
        )),
        Err(ToolExecutionError::NeedsUnlock)
    ));
}

fn pending_open(
    host: &mut Harness,
    handle: u8,
) -> (
    futures::future::BoxFuture<
        'static,
        Result<nocterm_cloud_model::tool::ToolExecutionResult, ToolExecutionError>,
    >,
    nocterm_session::SessionDriver,
) {
    use nocterm_agent_tools::headless::HeadlessTerminalOptions;
    let server = host
        .tools
        .offer(
            RemoteSessionId::from_bytes([handle; 16]),
            host.authority.session().clone(),
            HeadlessTerminalOptions::default(),
        )
        .unwrap();
    let (session, driver) = nocterm_session::channel(None);
    host.opener.queue(host.authority.session().clone(), session);
    let request = host.request(
        handle,
        TerminalCall::OpenTerminal(nocterm_ai::OpenTerminal { server_id: server }),
    );
    let mut wait = host.tools.execute(request);
    assert!(wait.as_mut().now_or_never().is_none());
    host.pump();
    (wait, driver)
}

#[test]
fn original_owner_drop_cannot_retire_a_published_pending_connection() {
    for publish in [false, true] {
        let (mut host, retained, _) = Harness::retained(Charset::Utf8);
        retained.remove(host.authority.session()).unwrap();
        let (wait, driver) = pending_open(&mut host, 10);
        let mut unready = retained.view(host.authority.run().clone()).unwrap();
        assert!(
            unready
                .attach_retained(
                    RemoteSessionId::from_bytes([11; 16]),
                    host.authority.session()
                )
                .is_err()
        );
        if !publish {
            drop(wait);
            host.pump();
            assert!(driver.closed().now_or_never().is_some());
            // The original receipt can be admitted again: cancelled staging
            // released the retained entry instead of leaving a closed orphan.
            let (retry, retried_driver) = pending_open(&mut host, 12);
            drop(retry);
            host.pump();
            assert!(retried_driver.closed().now_or_never().is_some());
            continue;
        }
        host.pool
            .run_until(driver.emit(nocterm_session::Event::Connected));
        host.pump(); // Connected is consumed, but the old 25ms waiter is not polled.
        let (old_tools, old_registry) = host.next_run(&retained, 9);
        old_tools.detach();
        drop(wait);
        drop(old_tools);
        drop(old_registry);
        host.pump();
        assert!(driver.closed().now_or_never().is_none());
        host.pool
            .run_until(driver.emit(nocterm_session::Event::Output(b"adopted".to_vec())));
        host.pump();
        assert_eq!(host.read(20, None)["text"], "adopted");
        host.call(
            21,
            TerminalCall::SendInput(SendInput {
                terminal_id: host.terminal.clone(),
                text: "new owner".into(),
                press_enter: true,
            }),
        )
        .unwrap();
        assert_eq!(
            host.pool.run_until(driver.next_command()),
            Some(Command::Input(b"new owner\r".to_vec()))
        );
    }
}

#[test]
fn original_owner_retirement_cancels_its_job_only_with_identical_run_binding() {
    let (mut host, retained, observations) = Harness::retained(Charset::Utf8);
    let old_sink = host.executor.queue_stream("old");
    host.call(1, host.exec("old")).unwrap();
    let old_command = host.journal.observations()[0].0.clone();
    let (old_tools, _) = host.next_run(&retained, 3); // Same original RunBinding.
    observations
        .allow_background(host.authority.clone())
        .unwrap();
    let new_sink = host.executor.queue_stream("new");
    let initial = host.call(2, host.exec("new")).unwrap();
    let command = initial.output().value()["command_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let new_command = host.journal.observations()[1].0.clone();
    assert_eq!(
        old_command.dispatch().intent().authority().run(),
        new_command.dispatch().intent().authority().run()
    );
    old_tools.revoke();
    host.pump();
    assert!(old_sink.is_closed());
    assert!(!new_sink.is_closed());
    assert!(
        host.pool
            .run_until(new_sink.send(b"new job still drains".to_vec()))
    );
    host.pump();
    new_sink.finish(ExecExit {
        status: Some(7),
        stderr: String::new(),
    });
    drop(new_sink);
    host.pump();
    assert_eq!(
        host.journal.observations(),
        [
            (old_command, Some(BackgroundCommandOutcome::Unknown)),
            (new_command, Some(BackgroundCommandOutcome::ObservedExit(7))),
        ]
    );
    let read = host
        .call(3, read_command(&host.terminal, &command))
        .unwrap();
    assert_eq!(read.output().value()["stdout"], "new job still drains");
    assert_eq!(read.output().value()["exit_status"], 7);
}

#[test]
fn duplicate_command_intent_preserves_the_original_job_and_observation() {
    let (mut host, retained, _) = Harness::retained(Charset::Utf8);
    let sink = host.executor.queue_stream("original");
    let result = host.call(1, host.exec("original")).unwrap();
    let command = result.output().value()["command_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let original = host.journal.observations()[0].0.clone();
    // The same runtime preserves its cached idempotent dispatch receipt.
    let repeated = host.call(1, host.exec("original")).unwrap();
    assert_eq!(repeated.output().value()["command_id"], command);
    // A fresh runtime with the same RunBinding also cannot replace the entry
    // before durable retention refuses the altered original command.
    let (old_tools, _) = host.next_run(&retained, 3);
    old_tools.detach();
    assert!(matches!(
        host.call(1, host.exec("replacement")),
        Err(ToolExecutionError::ArgumentMismatch)
    ));
    assert_eq!(host.executor.requests().len(), 1);
    assert_eq!(host.journal.observations(), [(original.clone(), None)]);
    assert!(!sink.is_closed());
    assert!(host.pool.run_until(sink.send(b"original output".to_vec())));
    host.pump();
    let read = host
        .call(2, read_command(&host.terminal, &command))
        .unwrap();
    assert_eq!(read.output().value()["stdout"], "original output");
    assert_eq!(read.output().value()["state"], "running");
    sink.finish(ExecExit {
        status: Some(7),
        stderr: "original stderr".into(),
    });
    drop(sink);
    host.pump();
    assert_eq!(
        host.journal.observations(),
        [(original, Some(BackgroundCommandOutcome::ObservedExit(7)))]
    );
    let read = host
        .call(3, read_command(&host.terminal, &command))
        .unwrap();
    assert_eq!(read.output().value()["exit_status"], 7);
    assert_eq!(read.output().value()["stderr"], "original stderr");
}

#[test]
fn initial_session_connected_requires_explicit_publication_then_reports_original_loss() {
    use nocterm_agent_tools::headless::{
        HeadlessTerminalOptions, InitialSessionCandidate, SessionLifecycleEvent,
    };
    use nocterm_session::{CloseReason, Event};
    let (mut host, retained, _) = Harness::retained(Charset::Utf8);
    retained.remove(host.authority.session()).unwrap();
    let (session, driver) = nocterm_session::channel(None);
    let (candidate, mut observations, task) = InitialSessionCandidate::prepare(
        host.registry.clone(),
        RemoteSessionId::from_bytes([31; 16]),
        host.authority.session().clone(),
        session,
        HeadlessTerminalOptions::default(),
        host.timer.clone(),
        Arc::new(()),
    )
    .unwrap();
    let terminal = task.terminal().to_owned();
    let generation = candidate.generation().clone();
    host.pool.spawner().spawn_local(task.into_run()).unwrap();
    assert!(observations.next().now_or_never().is_none());
    host.pool.run_until(driver.emit(Event::Connected));
    host.pump();
    let connected = observations
        .next()
        .now_or_never()
        .expect("lifecycle fact pending")
        .unwrap();
    assert!(matches!(
        connected.event(),
        SessionLifecycleEvent::Connected
    ));
    assert_eq!(connected.receipt(), candidate.receipt());
    assert_eq!(connected.generation(), &generation);
    let mut other = retained.view(host.authority.run().clone()).unwrap();
    assert!(
        other
            .attach_retained(RemoteSessionId::from_bytes([32; 16]), candidate.receipt())
            .is_err()
    );
    let input = || {
        TerminalCall::SendInput(SendInput {
            terminal_id: terminal.clone(),
            text: "admitted".into(),
            press_enter: false,
        })
    };
    assert!(host.call(40, input()).is_err());
    candidate.publish().unwrap();
    host.call(41, input()).unwrap();
    assert_eq!(
        host.pool.run_until(driver.next_command()),
        Some(Command::Input(b"admitted".to_vec()))
    );
    other
        .attach_retained(
            RemoteSessionId::from_bytes([32; 16]),
            host.authority.session(),
        )
        .unwrap();
    assert!(driver.closed().now_or_never().is_none());
    host.pool.run_until(driver.emit(Event::Connected));
    host.pool
        .run_until(driver.emit(Event::Output(b"output is not a lifecycle fact".to_vec())));
    host.pool
        .run_until(driver.emit(Event::Closed(CloseReason::ClosedByUser)));
    host.pump();
    let lost = observations
        .next()
        .now_or_never()
        .expect("lifecycle fact pending")
        .unwrap();
    assert_eq!(lost.receipt(), connected.receipt());
    assert_eq!(lost.generation(), &generation);
    assert!(matches!(lost.event(), SessionLifecycleEvent::Lost { .. }));
    assert!(matches!(observations.next().now_or_never(), Some(None)));
}

#[test]
fn initial_session_pending_drop_and_unpolled_reader_release_only_the_original_candidate() {
    use nocterm_agent_tools::headless::{
        HeadlessTerminalOptions, InitialSessionCandidate, SessionLifecycleEvent,
    };
    for order in 0..3 {
        let (host, retained, _) = Harness::retained(Charset::Utf8);
        retained.remove(host.authority.session()).unwrap();
        let resource = Arc::new(());
        let weak = Arc::downgrade(&resource);
        let (session, driver) = nocterm_session::channel(None);
        let (candidate, mut observations, task) = InitialSessionCandidate::prepare(
            host.registry.clone(),
            RemoteSessionId::from_bytes([33; 16]),
            host.authority.session().clone(),
            session,
            HeadlessTerminalOptions::default(),
            host.timer.clone(),
            resource,
        )
        .unwrap();
        let original = candidate.generation().clone();
        match order {
            0 => {
                drop(candidate);
                drop(task);
            }
            1 => {
                drop(task);
                drop(candidate);
            }
            _ => {
                assert!(candidate.publish().is_err());
                drop(task);
            }
        }
        assert!(driver.closed().now_or_never().is_some());
        let lost = observations
            .next()
            .now_or_never()
            .expect("lifecycle fact pending")
            .unwrap();
        assert_eq!(lost.generation(), &original);
        assert!(matches!(lost.event(), SessionLifecycleEvent::Lost { .. }));
        assert!(matches!(observations.next().now_or_never(), Some(None)));
        assert!(
            weak.upgrade().is_none(),
            "observation cannot retain the physical resource"
        );
        let (session, replacement) = nocterm_session::channel(None);
        let (candidate, _, task) = InitialSessionCandidate::prepare(
            host.registry.clone(),
            RemoteSessionId::from_bytes([33; 16]),
            host.authority.session().clone(),
            session,
            HeadlessTerminalOptions::default(),
            host.timer.clone(),
            Arc::new(()),
        )
        .unwrap();
        assert_ne!(candidate.generation(), &original);
        assert!(replacement.closed().now_or_never().is_none());
        drop(candidate);
        drop(task);
    }
}

#[test]
fn initial_session_reader_abort_reports_loss_and_closed_view_cannot_publish() {
    for close_view in [false, true] {
        use nocterm_agent_tools::headless::{
            HeadlessTerminalOptions, InitialSessionCandidate, SessionLifecycleEvent,
        };
        use nocterm_session::Event;
        let (mut host, retained, _) = Harness::retained(Charset::Utf8);
        retained.remove(host.authority.session()).unwrap();
        let (session, driver) = nocterm_session::channel(None);
        let (candidate, mut observations, task) = InitialSessionCandidate::prepare(
            host.registry.clone(),
            RemoteSessionId::from_bytes([34; 16]),
            host.authority.session().clone(),
            session,
            HeadlessTerminalOptions::default(),
            host.timer.clone(),
            Arc::new(()),
        )
        .unwrap();
        let generation = candidate.generation().clone();
        let (reader, abort) = futures::future::abortable(task.into_run());
        host.pool.spawner().spawn_local(reader.map(|_| ())).unwrap();
        host.pool.run_until(driver.emit(Event::Connected));
        host.pump();
        assert!(matches!(
            observations
                .next()
                .now_or_never()
                .expect("lifecycle fact pending")
                .unwrap()
                .event(),
            SessionLifecycleEvent::Connected
        ));
        if close_view {
            host.registry.lock().unwrap().close();
            assert!(candidate.publish().is_err());
        } else {
            candidate.publish().unwrap();
        }
        abort.abort();
        host.pump();
        let lost = observations
            .next()
            .now_or_never()
            .expect("lifecycle fact pending")
            .unwrap();
        assert_eq!(lost.generation(), &generation);
        assert!(matches!(lost.event(), SessionLifecycleEvent::Lost { .. }));
        assert!(driver.closed().now_or_never().is_some());
        assert!(matches!(observations.next().now_or_never(), Some(None)));
    }
}
