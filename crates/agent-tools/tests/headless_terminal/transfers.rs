use crate::headless_support::*;
use futures::task::LocalSpawnExt;
use nocterm_agent_tools::headless::{HeadlessError, HeadlessTerminalOptions, RetainedToolsFactory};
use nocterm_ai::{CancelTransfer, ReadTransfer, TerminalCall, TransferFiles};
use nocterm_cloud_model::{
    authority::{AuthenticationBinding, ExecutionAuthority, RunBinding, SessionReceipt},
    identity::{GrantEpoch, GrantId, RemoteSessionId, RunEpoch, RunId, SessionEpoch},
    testing::{ScriptedSessionHandles, ScriptedToolAuthority},
    tool::{
        RunRetirement, RunToolRuntimeFactory, ToolEffectStatus, ToolExecutionError,
        ToolExecutionResult,
    },
};
use nocterm_session::{Charset, Event, SessionDriver, testing::InMemoryFs};
use nocterm_transfers::{
    CollisionPolicy, OwnedTransferError, TransferDirection, TransferError, TransferEvent,
    TransferId, TransferRequests, TransferService, TransferState, testing::FakeTransfers,
};
use std::sync::Arc;

fn files(host: &Harness, sources: &[&str], destination: &str) -> TransferFiles {
    TransferFiles {
        source_terminal_id: host.terminal.clone(),
        sources: sources.iter().map(|source| (*source).to_owned()).collect(),
        destination_terminal_id: host.terminal.clone(),
        destination: destination.into(),
        overwrite: false,
        yield_ms: Some(0),
    }
}
fn transfer_id(result: &ToolExecutionResult) -> TransferId {
    result.output().value()["transfer_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}
fn complete(fake: &FakeTransfers, id: TransferId) {
    fake.advance(id, TransferEvent::DiscoveryComplete).unwrap();
    fake.advance(id, TransferEvent::WorkersDrained).unwrap();
}
fn refusal(error: &ToolExecutionError) -> &OwnedTransferError {
    let ToolExecutionError::Execution(cause) = error else {
        panic!("expected a native failure, got {error:?}");
    };
    let Some(HeadlessError::Transfer(inner)) = cause.downcast_ref::<HeadlessError>() else {
        panic!("expected a transfer failure");
    };
    inner.as_ref()
}
fn read(id: TransferId, yield_ms: Option<u64>) -> TerminalCall {
    TerminalCall::ReadTransfer(ReadTransfer {
        transfer_id: id.to_string(),
        yield_ms,
    })
}

#[test]
fn transfer_files_relays_within_the_runs_session_and_reports_it_observed() {
    let fake = Arc::new(FakeTransfers::default());
    let mut host = Harness::transferring(&fake, true);
    let call = files(&host, &["/srv/a", "/srv/b"], "/srv/in");
    let result = host.call(1, TerminalCall::TransferFiles(call)).unwrap();
    assert_eq!(result.effect_status(), &ToolEffectStatus::Observed);
    let report = result.output().value();
    assert_eq!(report["state"], "queued");
    assert_eq!(report["direction"], "relay");
    assert_eq!(report["destination"], "/srv/in");
    let request = fake.request(transfer_id(&result)).unwrap();
    assert_eq!(request.direction(), TransferDirection::Relay);
    assert_eq!(request.collisions(), CollisionPolicy::Skip);
}

#[test]
fn overwrite_replaces_existing_destination_files() {
    let fake = Arc::new(FakeTransfers::default());
    let mut host = Harness::transferring(&fake, true);
    let mut call = files(&host, &["/srv/a"], "/srv/in");
    call.overwrite = true;
    let result = host.call(1, TerminalCall::TransferFiles(call)).unwrap();
    let request = fake.request(transfer_id(&result)).unwrap();
    assert_eq!(request.collisions(), CollisionPolicy::Replace);
}

#[test]
fn transfer_files_waits_for_completion_within_the_default_yield() {
    let fake = Arc::new(FakeTransfers::default());
    let mut host = Harness::transferring(&fake, true);
    let mut call = files(&host, &["/srv/a"], "/srv/in");
    call.yield_ms = None;
    let request = host.request(1, TerminalCall::TransferFiles(call));
    let mut pending = host.start(request);
    host.tick(900);
    assert!(pending.try_recv().unwrap().is_none());
    complete(&fake, fake.snapshot()[0].id);
    host.tick(25);
    let result = host.finish(pending).unwrap();
    assert_eq!(result.output().value()["state"], "completed");
}

#[test]
fn transfer_files_returns_the_running_report_when_the_yield_elapses() {
    let fake = Arc::new(FakeTransfers::default());
    let mut host = Harness::transferring(&fake, true);
    let mut call = files(&host, &["/srv/a"], "/srv/in");
    call.yield_ms = Some(200);
    let request = host.request(1, TerminalCall::TransferFiles(call));
    let pending = host.start(request);
    host.tick(200);
    let result = host.finish(pending).unwrap();
    assert_eq!(result.output().value()["state"], "queued");
}

#[test]
fn read_transfer_waits_while_the_transfer_runs() {
    let fake = Arc::new(FakeTransfers::default());
    let mut host = Harness::transferring(&fake, true);
    let call = files(&host, &["/srv/a"], "/srv/in");
    let started = host.call(1, TerminalCall::TransferFiles(call)).unwrap();
    let id = transfer_id(&started);
    let request = host.request(2, read(id, Some(500)));
    let mut pending = host.start(request);
    host.tick(100);
    assert!(pending.try_recv().unwrap().is_none());
    complete(&fake, id);
    host.tick(25);
    let result = host.finish(pending).unwrap();
    assert_eq!(result.output().value()["state"], "completed");
    assert_eq!(result.effect_status(), &ToolEffectStatus::Observed);
}

#[test]
fn cancel_transfer_stops_the_transfer_and_reports_it() {
    let fake = Arc::new(FakeTransfers::default());
    let mut host = Harness::transferring(&fake, true);
    let call = files(&host, &["/srv/a"], "/srv/in");
    let started = host.call(1, TerminalCall::TransferFiles(call)).unwrap();
    let id = transfer_id(&started);
    let cancel = TerminalCall::CancelTransfer(CancelTransfer {
        transfer_id: id.to_string(),
    });
    let result = host.call(2, cancel).unwrap();
    assert_eq!(result.output().value()["state"], "cancelled");
    assert_eq!(result.effect_status(), &ToolEffectStatus::Observed);
    assert_eq!(fake.progress(id).unwrap().state, TransferState::Cancelled);
}

#[test]
fn only_transfers_this_chat_started_can_be_read() {
    let fake = Arc::new(FakeTransfers::default());
    let mut host = Harness::transferring(&fake, true);
    let malformed = TerminalCall::ReadTransfer(ReadTransfer {
        transfer_id: "nope".into(),
        yield_ms: None,
    });
    let error = host.call(1, malformed).unwrap_err();
    assert!(matches!(
        refusal(&error),
        OwnedTransferError::Transfer(TransferError::InvalidId)
    ));
    let unknown = TransferId::from_bytes([7; 16]).unwrap();
    let error = host.call(2, read(unknown, None)).unwrap_err();
    assert!(matches!(refusal(&error), OwnedTransferError::NotOwned));
}

#[test]
fn a_fifth_running_transfer_is_refused_for_capacity() {
    let fake = Arc::new(FakeTransfers::default());
    let mut host = Harness::transferring(&fake, true);
    for id in 1..=4 {
        let call = files(&host, &["/srv/a"], "/srv/in");
        host.call(id, TerminalCall::TransferFiles(call)).unwrap();
    }
    let call = files(&host, &["/srv/a"], "/srv/in");
    let error = host.call(5, TerminalCall::TransferFiles(call)).unwrap_err();
    assert!(matches!(error, ToolExecutionError::Capacity));
    assert_eq!(fake.snapshot().len(), 4);
}

#[test]
fn a_destination_inside_a_source_is_refused_before_anything_starts() {
    let fake = Arc::new(FakeTransfers::default());
    let mut host = Harness::transferring(&fake, true);
    let call = files(&host, &["/srv/data"], "/srv/data/nested");
    let error = host.call(1, TerminalCall::TransferFiles(call)).unwrap_err();
    assert!(matches!(
        refusal(&error),
        OwnedTransferError::Transfer(TransferError::DestinationInsideSource)
    ));
    assert!(fake.snapshot().is_empty());
}

#[test]
fn transfers_without_a_book_or_a_file_system_are_unsupported() {
    let mut host = Harness::new(Charset::Utf8);
    let call = files(&host, &["/srv/a"], "/srv/in");
    let error = host.call(1, TerminalCall::TransferFiles(call)).unwrap_err();
    assert!(matches!(error, ToolExecutionError::UnsupportedCapability));
    let error = host.call(2, read(TransferId::from_bytes([7; 16]).unwrap(), None));
    assert!(matches!(
        error,
        Err(ToolExecutionError::UnsupportedCapability)
    ));
    let fake = Arc::new(FakeTransfers::default());
    let mut host = Harness::transferring(&fake, false);
    let call = files(&host, &["/srv/a"], "/srv/in");
    let error = host.call(1, TerminalCall::TransferFiles(call)).unwrap_err();
    assert!(matches!(error, ToolExecutionError::UnsupportedCapability));
    assert!(fake.snapshot().is_empty());
}

/// A second connected session of the same run view, under a different receipt.
fn other_session(host: &mut Harness) -> (String, SessionDriver) {
    let receipt = SessionReceipt::new(
        RemoteSessionId::from_bytes([40; 16]),
        SessionEpoch::new(1).unwrap(),
        host.authority.session().target().clone(),
        AuthenticationBinding::new(fingerprint(8), fingerprint(9)),
    );
    let (session, driver) = nocterm_session::channel(None);
    let task = host
        .registry
        .lock()
        .unwrap()
        .attach(
            RemoteSessionId::from_bytes([41; 16]),
            receipt,
            session.with_fs(Some(Arc::new(InMemoryFs::default()))),
            HeadlessTerminalOptions::default(),
            host.timer.clone(),
        )
        .unwrap();
    let terminal = task.terminal().to_owned();
    host.pool.spawner().spawn_local(task.into_run()).unwrap();
    host.pool.run_until(driver.emit(Event::Connected));
    host.pool.run_until_stalled();
    (terminal, driver)
}

#[test]
fn a_terminal_of_another_session_is_stale_for_either_endpoint() {
    let fake = Arc::new(FakeTransfers::default());
    let mut host = Harness::transferring(&fake, true);
    let (other, _driver) = other_session(&mut host);
    let mut call = files(&host, &["/srv/a"], "/srv/in");
    call.destination_terminal_id = other.clone();
    let error = host.call(1, TerminalCall::TransferFiles(call)).unwrap_err();
    assert!(matches!(error, ToolExecutionError::StaleSession));
    let mut call = files(&host, &["/srv/a"], "/srv/in");
    call.source_terminal_id = other;
    let error = host.call(2, TerminalCall::TransferFiles(call)).unwrap_err();
    assert!(matches!(error, ToolExecutionError::StaleSession));
    assert!(fake.snapshot().is_empty());
}

#[test]
fn revoking_the_run_cancels_its_running_transfers() {
    let fake = Arc::new(FakeTransfers::default());
    let mut host = Harness::transferring(&fake, true);
    let call = files(&host, &["/srv/a"], "/srv/in");
    let started = host.call(1, TerminalCall::TransferFiles(call)).unwrap();
    host.tools.revoke();
    let id = transfer_id(&started);
    assert_eq!(fake.progress(id).unwrap().state, TransferState::Cancelled);
}

#[test]
fn dropping_an_unretained_run_cancels_its_running_transfers() {
    let fake = Arc::new(FakeTransfers::default());
    let mut host = Harness::transferring(&fake, true);
    let call = files(&host, &["/srv/a"], "/srv/in");
    let started = host.call(1, TerminalCall::TransferFiles(call)).unwrap();
    drop(host.tools);
    let id = transfer_id(&started);
    assert_eq!(fake.progress(id).unwrap().state, TransferState::Cancelled);
}

fn next_run(host: &Harness, run_id: u8) -> (ExecutionAuthority, Arc<ScriptedToolAuthority>) {
    let current = host.authority.run();
    let run = RunBinding::new(
        current.scope().clone(),
        current.runner(),
        current.runner_epoch(),
        current.chat(),
        RunId::from_bytes([run_id; 16]),
        RunEpoch::new(1).unwrap(),
    );
    let authority = ExecutionAuthority::new(
        run,
        host.authority.session().clone(),
        GrantId::from_bytes([run_id; 16]),
        GrantEpoch::new(1).unwrap(),
    );
    let checks = Arc::new(ScriptedToolAuthority::new(authority.clone()));
    (authority, checks)
}

#[test]
fn run_views_of_one_chat_share_transfers_and_chat_close_cancels_them() {
    let fake = Arc::new(FakeTransfers::default());
    let (mut host, retained, _) = Harness::retained_files();
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
    )
    .with_transfers(fake.clone());
    let first = host
        .pool
        .run_until(factory.attach(
            host.authority.run().clone(),
            host.authority.session().clone(),
            host.checks.clone(),
        ))
        .unwrap();
    let terminal = "t-09090909090909090909090909090909".to_owned();
    let mut call = files(&host, &["/srv/a"], "/srv/in");
    call.source_terminal_id = terminal.clone();
    call.destination_terminal_id = terminal;
    let started = host
        .pool
        .run_until(first.execute(host.request(1, TerminalCall::TransferFiles(call))))
        .unwrap();
    let id = transfer_id(&started);
    let (authority, checks) = next_run(&host, 4);
    let second = host
        .pool
        .run_until(factory.attach(authority.run().clone(), authority.session().clone(), checks))
        .unwrap();
    let seen = host
        .pool
        .run_until(second.execute(request(&authority, 2, read(id, None))))
        .unwrap();
    assert_eq!(seen.output().value()["state"], "queued");
    factory.cancel_transfers();
    let seen = host
        .pool
        .run_until(second.execute(request(&authority, 3, read(id, None))))
        .unwrap();
    assert_eq!(seen.output().value()["state"], "cancelled");
}

#[test]
fn revoking_a_retained_run_cancels_the_chats_running_transfers() {
    let fake = Arc::new(FakeTransfers::default());
    let (mut host, retained, _) = Harness::retained_files();
    let factory = RetainedToolsFactory::new(
        retained,
        host.timer.clone(),
        host.journal.clone(),
        host.tasks.clone(),
        host.opener.clone(),
        Arc::new(ScriptedSessionHandles::new([RemoteSessionId::from_bytes(
            [9; 16],
        )])),
    )
    .with_transfers(fake.clone());
    let runtime = host
        .pool
        .run_until(factory.attach(
            host.authority.run().clone(),
            host.authority.session().clone(),
            host.checks.clone(),
        ))
        .unwrap();
    let terminal = "t-09090909090909090909090909090909".to_owned();
    let mut call = files(&host, &["/srv/a"], "/srv/in");
    call.source_terminal_id = terminal.clone();
    call.destination_terminal_id = terminal;
    let started = host
        .pool
        .run_until(runtime.execute(host.request(1, TerminalCall::TransferFiles(call))))
        .unwrap();
    host.pool
        .run_until(runtime.retire(RunRetirement::Completed))
        .unwrap();
    let id = transfer_id(&started);
    assert_eq!(fake.progress(id).unwrap().state, TransferState::Queued);
    host.pool
        .run_until(runtime.retire(RunRetirement::Revoked))
        .unwrap();
    assert_eq!(fake.progress(id).unwrap().state, TransferState::Cancelled);
}
