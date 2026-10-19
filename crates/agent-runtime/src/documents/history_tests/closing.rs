use super::*;
use nocterm_ai::testing::ScriptedCommands;

pub(super) fn connected_client(
    runtime: &Entity<Runtime>,
    connector: &ScriptedConnector,
    commands: &ScriptedCommands,
    cx: &mut TestAppContext,
) -> Rc<ProbeClient> {
    let client = ProbeClient::new(cx, snapshot());
    connector.push_scripted(commands);
    runtime.update(cx, |runtime, cx| {
        runtime.register_document(client.clone(), cx);
        runtime.connect(
            client.clone(),
            client.chat.borrow().id().to_owned(),
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
        lease.session = Some(nocterm_ai::model::ids::SessionId::new("session"));
    }
    events.clear();
    drop(lease);
}
#[gpui::test]
fn closing_cleanup_handle_is_retained_on_failure_and_released_on_success(cx: &mut TestAppContext) {
    for fails in [true, false] {
        let connector = Arc::new(ScriptedConnector::default());
        let (runtime, _directory, _repository) = fixture_with_connector(cx, connector.clone());
        let commands = ScriptedCommands::default();
        if fails {
            commands.fail_shutdown();
        }
        let retained = commands.witness();
        let client = connected_client(&runtime, &connector, &commands, cx);
        release_client(&client, false);
        cx.run_until_parked();
        assert_eq!(commands.shutdowns(), 1);
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
        commands.release_handle();
        assert_eq!(
            retained.retained(),
            fails,
            "failed cleanup must retain process ownership"
        );
    }
}
#[gpui::test]
fn a_connections_event_source_is_released_before_its_close_is_reported(cx: &mut TestAppContext) {
    let connector = Arc::new(ScriptedConnector::default());
    let (runtime, _directory, _repository) = fixture_with_connector(cx, connector.clone());
    let commands = ScriptedCommands::default();
    let events = commands.events_witness();
    let client = connected_client(&runtime, &connector, &commands, cx);
    assert!(
        events.retained(),
        "a live connection keeps its event source"
    );
    release_client(&client, false);
    cx.run_until_parked();
    assert!(
        client
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, SessionEvent::SessionClosed))
    );
    assert!(
        !events.retained(),
        "the event source must be gone when the close is reported"
    );
}
#[gpui::test]
fn actual_quit_starts_active_and_already_closing_cleanup_without_waiting_for_close_ack(
    cx: &mut TestAppContext,
) {
    let connector = Arc::new(ScriptedConnector::default());
    let (runtime, _directory, _repository) = fixture_with_connector(cx, connector.clone());
    let closing = ScriptedCommands::default();
    let active = ScriptedCommands::default();
    active.hang_shutdown();
    let first = connected_client(&runtime, &connector, &closing, cx);
    release_client(&first, true);
    cx.run_until_parked();
    let _second = connected_client(&runtime, &connector, &active, cx);
    assert_eq!(closing.closes(), 1);
    assert_eq!(closing.shutdowns(), 0);
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
        active.shutdowns(),
        1,
        "slow active cleanup must have started"
    );
    assert_eq!(
        closing.shutdowns(),
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
    let commands = ScriptedCommands::default();
    let first = connected_client(&runtime, &connector, &commands, cx);
    let second = connected_client(&runtime, &connector, &commands, cx);
    let accepted = [&first, &second].map(|client| {
        client
            .events
            .borrow()
            .iter()
            .find_map(|event| match event {
                SessionEvent::Connected { commands, .. } => Some(commands.clone()),
                _ => None,
            })
            .expect("both connections must have accepted the supplied command handle")
    });
    assert!(accepted[0].same_connection(&accepted[1]));
    assert_eq!(connector.working_directories().len(), 2);
    assert_eq!(commands.shutdowns(), 0);
    runtime.update(cx, |runtime, cx| runtime.shutdown(cx)).await;
    assert_eq!(
        commands.shutdowns(),
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
    let commands = ScriptedCommands::default();
    let release = commands.gate_shutdown();
    let client = connected_client(&runtime, &connector, &commands, cx);
    release_client(&client, false);
    cx.run_until_parked();
    assert_eq!(commands.shutdowns(), 1);
    let drain = runtime.update(cx, |runtime, cx| runtime.shutdown(cx));
    cx.run_until_parked();
    let calls_during_capture = commands.shutdowns();
    release.send(()).unwrap();
    drain.await;
    cx.run_until_parked();
    assert_eq!(
        calls_during_capture, 1,
        "capture must join the existing graceful cleanup instead of invoking it twice",
    );
    assert_eq!(commands.shutdowns(), 1);
}

#[gpui::test]
async fn a_close_ack_after_explicit_shutdown_cannot_start_process_cleanup_again(
    cx: &mut TestAppContext,
) {
    let connector = Arc::new(ScriptedConnector::default());
    let (runtime, _directory, _repository) = fixture_with_connector(cx, connector.clone());
    let commands = ScriptedCommands::default();
    let release = commands.gate_close();
    let client = connected_client(&runtime, &connector, &commands, cx);
    release_client(&client, true);
    cx.run_until_parked();
    assert_eq!(commands.closes(), 1);
    assert_eq!(commands.shutdowns(), 0);
    runtime.update(cx, |runtime, cx| runtime.shutdown(cx)).await;
    assert_eq!(
        commands.shutdowns(),
        1,
        "explicit shutdown must start cleanup without waiting for session close",
    );
    // Final ownership may cancel the ordinary close future; either outcome must
    // leave the one completed process cleanup intact.
    let _ = release.send(());
    cx.run_until_parked();
    assert_eq!(
        commands.shutdowns(),
        1,
        "the retained-App close continuation must not repeat completed process cleanup",
    );
}

#[gpui::test]
async fn final_capture_retries_a_previously_failed_normal_cleanup_once(cx: &mut TestAppContext) {
    let connector = Arc::new(ScriptedConnector::default());
    let (runtime, _directory, _repository) = fixture_with_connector(cx, connector.clone());
    let commands = ScriptedCommands::default();
    commands.fail_shutdown();
    let client = connected_client(&runtime, &connector, &commands, cx);
    release_client(&client, false);
    cx.run_until_parked();
    assert_eq!(commands.shutdowns(), 1);
    assert!(
        client
            .events
            .borrow()
            .iter()
            .any(|event| { matches!(event, SessionEvent::CleanupUnconfirmed(_)) })
    );
    runtime.update(cx, |runtime, cx| runtime.shutdown(cx)).await;
    assert_eq!(
        commands.shutdowns(),
        2,
        "final capture must retry the retained failed normal attempt exactly once",
    );
    runtime.update(cx, |runtime, cx| runtime.shutdown(cx)).await;
    assert_eq!(commands.shutdowns(), 2);
}

#[gpui::test]
async fn a_late_close_joins_the_failed_final_retry_without_a_third_native_attempt(
    cx: &mut TestAppContext,
) {
    let connector = Arc::new(ScriptedConnector::default());
    let (runtime, _directory, _repository) = fixture_with_connector(cx, connector.clone());
    let commands = ScriptedCommands::default();
    commands.fail_shutdown();
    let first = connected_client(&runtime, &connector, &commands, cx);
    let second = connected_client(&runtime, &connector, &commands, cx);
    release_client(&first, false);
    cx.run_until_parked();
    assert_eq!(commands.shutdowns(), 1);
    let release = commands.gate_close();
    release_client(&second, true);
    cx.run_until_parked();
    assert_eq!(commands.closes(), 1);
    assert_eq!(commands.shutdowns(), 1);
    runtime.update(cx, |runtime, cx| runtime.shutdown(cx)).await;
    assert_eq!(commands.shutdowns(), 2);
    let _ = release.send(());
    cx.run_until_parked();
    assert_eq!(
        commands.shutdowns(),
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

#[gpui::test]
async fn stale_closing_capture_preserves_the_newer_writer_owned_candidate(cx: &mut TestAppContext) {
    let (runtime, _directory, repository) = fixture(cx);
    let old = snapshot();
    let client = ProbeClient::new(cx, old.clone());
    let latest = redraft(&old, "writer owned newest");
    runtime.update(cx, |runtime, cx| {
        runtime.register_document(client.clone(), cx);
        assert!(
            client
                .save(&runtime.chats.publication(), &latest, 2, cx)
                .unwrap()
        );
    });
    let final_work = runtime.update(cx, |runtime, cx| runtime.capture_shutdown(cx));
    final_work.await.unwrap();
    assert_eq!(
        repository.transcript(old.id()).unwrap().composer().draft,
        draft(&latest)
    );
}

#[gpui::test]
fn a_detached_capture_acknowledges_its_original_client_outside_the_runtime_borrow(
    cx: &mut TestAppContext,
) {
    let (runtime, _directory, repository) = fixture(cx);
    let client = ProbeClient::new(cx, snapshot());
    let callback_runtime = runtime.clone();
    let reentered = Rc::new(Cell::new(false));
    let observed = reentered.clone();
    *client.on_admitted.borrow_mut() = Some(Box::new(move |id, cx| {
        callback_runtime.update(cx, |runtime, cx| runtime.unregister_document(id, cx));
        observed.set(true);
    }));
    runtime.update(cx, |runtime, cx| {
        runtime.register_document(client.clone(), cx);
        assert!(
            runtime
                .capture_and_detach_document(client.id(), cx)
                .unwrap()
        );
        assert!(
            !reentered.get(),
            "admission delivery is deferred until the Runtime borrow ends"
        );
    });
    cx.run_until_parked();
    assert!(reentered.get());
    assert!(
        client
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, SessionEvent::SaveAdmitted { revision: 1 }))
    );
    assert_eq!(
        repository
            .transcript(snapshot().id())
            .unwrap()
            .composer()
            .draft,
        draft(&snapshot())
    );
}
#[gpui::test]
fn an_unaccepted_startup_orphan_is_shut_down_exactly_once(cx: &mut TestAppContext) {
    let connector = Arc::new(ScriptedConnector::default());
    let (runtime, _directory, _repository) = fixture_with_connector(cx, connector.clone());
    let commands = ScriptedCommands::default();
    let (open, gate) = futures::channel::oneshot::channel::<()>();
    let script = commands.clone();
    connector.push_managed(move |request| {
        let lifetime = request.lifetime();
        let connected = script.connect(request)?;
        nocterm_ai::history::source::ManagedSourceFuture::new(lifetime, async move {
            let _ = gate.await;
            connected.await
        })
    });
    let client = ProbeClient::new(cx, snapshot());
    runtime.update(cx, |runtime, cx| {
        runtime.register_document(client.clone(), cx);
        runtime.connect(
            client.clone(),
            client.chat.borrow().id().to_owned(),
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
    release_client(&client, false);
    cx.run_until_parked();
    open.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(commands.shutdowns(), 1);
    cx.run_until_parked();
    assert_eq!(commands.shutdowns(), 1);
}
