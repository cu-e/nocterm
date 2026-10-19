//! Gate cancellation retains the original paid identity without capturing another reader.
use super::*;
use crate::{
    model::retention::RetentionAuthority, testing::InMemoryAgentRepository, thread::ThreadState,
};
use futures::{FutureExt, task::noop_waker};

#[test]
fn cancellation_while_waiting_for_gate_releases_original_identity_and_keeps_reader_idle() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let repository = Arc::new(InMemoryAgentRepository::new(authority.clone()));
    let mut state = ThreadState::new(
        &authority,
        "agent",
        "00000000-0000-0000-0000-000000000001",
        1,
    )
    .unwrap();
    let scope = state.live().role().scope().clone();
    let source = state.reader().clone();
    let gate = HistoryGate::default();
    let held = futures::executor::block_on(gate.acquire());
    let id = state.live_mut().prepare_reader_id().unwrap();
    let mut work = id
        .prepare_load(source.clone(), gate.clone(), repository.clone())
        .unwrap();
    assert!(
        work.poll_unpin(&mut Context::from_waker(&noop_waker()))
            .is_pending()
    );
    drop(source.capture().unwrap());
    assert!(repository.operations().is_empty());
    let admission = state.live().local_admission();
    state
        .live_mut()
        .set_draft(admission, Some("typing during the gate wait"))
        .unwrap();
    let waiting = scope.retained_and_pending_bytes().unwrap();
    drop(work);
    assert!(scope.retained_and_pending_bytes().unwrap() < waiting);
    assert_eq!(
        &*state.live().composer().unwrap().draft().unwrap(),
        "typing during the gate wait"
    );
    drop(source.capture().unwrap());
    assert!(repository.operations().is_empty());
    drop(held);
    drop(futures::executor::block_on(gate.acquire()));
}
