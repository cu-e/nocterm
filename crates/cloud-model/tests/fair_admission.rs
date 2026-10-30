mod support;
use nocterm_cloud_model::{
    admission::*,
    authority::RunBinding,
    identity::{SchedulerId, Timestamp},
};
use support::*;

fn scheduler(limits: AdmissionLimits) -> FairAdmission {
    FairAdmission::new(SchedulerId::from_bytes([1; 16]), limits)
}
fn request(account: u8, chat: u8, id: u8) -> QueuedRun {
    QueuedRun::new(
        run(account, chat, id, 1),
        target(1),
        RunAccess::ReadOnly,
        Timestamp::from_millis(1000),
    )
}
fn lease(state: &mut FairAdmission) -> AdmissionLease {
    state
        .reserve(Timestamp::from_millis(2))
        .unwrap()
        .lease
        .unwrap()
}

#[test]
fn prepared_capacity_is_bounded_and_cannot_run_before_durable_activation() {
    let mut state = scheduler(AdmissionLimits::new(1, 1, 1, 1, 1, 1).unwrap());
    let ticket = state
        .prepare(request(1, 1, 1), Timestamp::from_millis(1))
        .unwrap();
    assert_eq!(state.pending_count(), 1);
    assert_eq!(state.queued_count(), 0);
    assert!(
        state
            .reserve(Timestamp::from_millis(2))
            .unwrap()
            .lease
            .is_none()
    );
    assert!(matches!(
        state.prepare(request(1, 2, 2), Timestamp::from_millis(2)),
        Err(AdmissionError::GlobalCapacity)
    ));
    state.activate(&ticket, Timestamp::from_millis(2)).unwrap();
    assert_eq!(state.pending_count(), 0);
    assert!(matches!(
        state.activate(&ticket, Timestamp::from_millis(2)),
        Err(AdmissionError::StaleLease)
    ));
    assert_eq!(
        lease(&mut state).request().binding(),
        ticket.request().binding()
    );
}

#[test]
fn failed_acceptance_releases_only_its_original_pending_capacity() {
    let mut state = scheduler(AdmissionLimits::new(1, 1, 1, 1, 1, 1).unwrap());
    let old = state
        .prepare(request(1, 1, 1), Timestamp::from_millis(1))
        .unwrap();
    state.abandon(&old).unwrap();
    let current = state
        .prepare(request(1, 1, 2), Timestamp::from_millis(2))
        .unwrap();
    assert!(matches!(
        state.abandon(&old),
        Err(AdmissionError::StaleLease)
    ));
    assert_eq!(state.pending_count(), 1);
    state.activate(&current, Timestamp::from_millis(2)).unwrap();
    assert_eq!(
        lease(&mut state).request().binding(),
        current.request().binding()
    );
}

#[test]
fn stop_can_cancel_a_committed_request_before_its_activation() {
    let mut state = scheduler(AdmissionLimits::default());
    let ticket = state
        .prepare(request(1, 1, 1), Timestamp::from_millis(1))
        .unwrap();
    assert!(state.cancel_queued(ticket.request().binding()).is_some());
    assert!(matches!(
        state.activate(&ticket, Timestamp::from_millis(2)),
        Err(AdmissionError::StaleLease)
    ));
    assert_eq!(state.resident_accounts(), 0);
}

#[test]
fn route_aliases_share_the_same_mutating_target_lane() {
    use nocterm_cloud_model::authority::TargetBinding;
    let mut state = scheduler(AdmissionLimits::new(2, 2, 8, 8, 8, 2).unwrap());
    let original = target(1);
    let alias = TargetBinding::new(
        original.host(),
        fingerprint(77),
        original.principal().clone(),
        original.workspace(),
    );
    for (run, target) in [(run(1, 1, 1, 1), original), (run(1, 2, 2, 1), alias)] {
        state
            .enqueue(
                QueuedRun::new(
                    run,
                    target,
                    RunAccess::Mutating,
                    Timestamp::from_millis(1000),
                ),
                Timestamp::from_millis(1),
            )
            .unwrap();
    }
    let first = lease(&mut state);
    assert!(
        state
            .reserve(Timestamp::from_millis(2))
            .unwrap()
            .lease
            .is_none()
    );
    state.release(&first).unwrap();
    assert!(
        state
            .reserve(Timestamp::from_millis(2))
            .unwrap()
            .lease
            .is_some()
    );
}

#[test]
fn accounts_share_slots_round_robin_without_allocating_workers_for_idle_scopes() {
    let mut state = scheduler(AdmissionLimits::default());
    assert_eq!(state.resident_accounts(), 0);
    for candidate in [request(1, 1, 1), request(1, 2, 2), request(2, 1, 1)] {
        state.enqueue(candidate, Timestamp::from_millis(1)).unwrap();
    }
    let first = lease(&mut state);
    let second = lease(&mut state);
    assert_eq!(first.request().binding().scope(), &scope(1));
    assert_eq!(second.request().binding().scope(), &scope(2));
    assert_eq!(state.active_count(), 2);
    state.release(&first).unwrap();
    state.release(&second).unwrap();
    let third = lease(&mut state);
    state.release(&third).unwrap();
    assert_eq!(state.resident_accounts(), 0);
}

#[test]
fn one_active_run_per_chat_does_not_block_another_chat_in_the_same_account() {
    let mut state = scheduler(AdmissionLimits::new(2, 2, 8, 8, 4, 2).unwrap());
    for candidate in [request(1, 1, 1), request(1, 1, 2), request(1, 2, 3)] {
        state.enqueue(candidate, Timestamp::from_millis(1)).unwrap();
    }
    let first = lease(&mut state);
    let second = lease(&mut state);
    assert_ne!(
        first.request().binding().chat(),
        second.request().binding().chat()
    );
    assert!(
        state
            .reserve(Timestamp::from_millis(2))
            .unwrap()
            .lease
            .is_none()
    );
    state.release(&first).unwrap();
    assert_eq!(
        lease(&mut state).request().binding().run(),
        request(1, 1, 2).binding().run()
    );
}

#[test]
fn global_account_chat_and_resident_limits_refuse_before_unbounded_queue_growth() {
    let mut state = scheduler(AdmissionLimits::new(2, 1, 3, 2, 1, 2).unwrap());
    state
        .enqueue(request(1, 1, 1), Timestamp::from_millis(1))
        .unwrap();
    assert_eq!(
        state.enqueue(request(1, 1, 2), Timestamp::from_millis(1)),
        Err(AdmissionError::ChatCapacity)
    );
    state
        .enqueue(request(1, 2, 2), Timestamp::from_millis(1))
        .unwrap();
    assert_eq!(
        state.enqueue(request(1, 3, 3), Timestamp::from_millis(1)),
        Err(AdmissionError::AccountCapacity)
    );
    state
        .enqueue(request(2, 1, 1), Timestamp::from_millis(1))
        .unwrap();
    assert_eq!(
        state.enqueue(request(3, 1, 1), Timestamp::from_millis(1)),
        Err(AdmissionError::AccountCapacity)
    );
    assert_eq!(
        state.enqueue(request(2, 2, 2), Timestamp::from_millis(1)),
        Err(AdmissionError::GlobalCapacity)
    );
    assert_eq!(state.queued_count(), 3);
}

#[test]
fn account_writer_lanes_serialize_chats_while_readers_share_the_target() {
    let mut state = scheduler(AdmissionLimits::default());
    for (account, chat, access) in [
        (1, 1, RunAccess::Mutating),
        (1, 2, RunAccess::Mutating),
        (2, 1, RunAccess::ReadOnly),
    ] {
        state
            .enqueue(
                QueuedRun::new(
                    run(account, chat, 1, 1),
                    target(1),
                    access,
                    Timestamp::from_millis(1000),
                ),
                Timestamp::from_millis(1),
            )
            .unwrap();
    }
    let writer = lease(&mut state);
    assert_eq!(writer.request().access(), RunAccess::Mutating);
    let reader = lease(&mut state);
    assert_eq!(reader.request().access(), RunAccess::ReadOnly);
    assert!(
        state
            .reserve(Timestamp::from_millis(2))
            .unwrap()
            .lease
            .is_none()
    );
    state.release(&writer).unwrap();
    assert_eq!(
        lease(&mut state).request().binding().chat(),
        request(1, 2, 1).binding().chat()
    );
}

#[test]
fn expired_queue_entries_are_reported_but_active_work_stays_reserved_until_cleanup() {
    let mut state = scheduler(AdmissionLimits::new(1, 1, 2, 2, 2, 2).unwrap());
    let candidate = |account| {
        QueuedRun::new(
            run(account, 1, 1, 1),
            target(1),
            RunAccess::ReadOnly,
            Timestamp::from_millis(10),
        )
    };
    state
        .enqueue(candidate(1), Timestamp::from_millis(1))
        .unwrap();
    let active = lease(&mut state);
    state
        .enqueue(candidate(2), Timestamp::from_millis(2))
        .unwrap();
    let decision = state.reserve(Timestamp::from_millis(10)).unwrap();
    assert_eq!(decision.expired.len(), 1);
    assert!(decision.lease.is_none());
    assert_eq!(state.active_count(), 1);
    assert_eq!(state.expired_active(Timestamp::from_millis(10)).len(), 1);
    state.release(&active).unwrap();
    assert_eq!(state.resident_accounts(), 0);
}

#[test]
fn stale_or_cross_scheduler_release_never_frees_a_replacement_reservation() {
    let mut first = scheduler(AdmissionLimits::default());
    let candidate = request(1, 1, 1);
    first
        .enqueue(candidate.clone(), Timestamp::from_millis(1))
        .unwrap();
    let old = lease(&mut first);
    first.release(&old).unwrap();
    first
        .enqueue(candidate.clone(), Timestamp::from_millis(2))
        .unwrap();
    let current = lease(&mut first);
    assert_eq!(first.release(&old), Err(AdmissionError::StaleLease));
    assert_eq!(first.active_count(), 1);
    let mut other =
        FairAdmission::new(SchedulerId::from_bytes([2; 16]), AdmissionLimits::default());
    other.enqueue(candidate, Timestamp::from_millis(1)).unwrap();
    let replacement = lease(&mut other);
    assert_eq!(other.release(&current), Err(AdmissionError::StaleLease));
    assert_eq!(other.active_count(), 1);
    other.release(&replacement).unwrap();
}

#[test]
fn full_queues_and_clock_failure_do_not_block_explicit_stop_and_resource_release() {
    let mut state = scheduler(AdmissionLimits::new(1, 1, 1, 1, 1, 1).unwrap());
    state
        .enqueue(request(1, 1, 1), Timestamp::from_millis(1))
        .unwrap();
    let active = lease(&mut state);
    let queued = request(1, 2, 2);
    state
        .enqueue(queued.clone(), Timestamp::from_millis(3))
        .unwrap();
    assert_eq!(
        state.enqueue(request(1, 3, 3), Timestamp::from_millis(2)),
        Err(AdmissionError::ClockRollback)
    );
    assert!(state.cancel_queued(queued.binding()).is_some());
    state.release(&active).unwrap();
    assert_eq!(state.active_count(), 0);
    assert_eq!(
        state.enqueue(request(1, 3, 3), Timestamp::from_millis(4)),
        Err(AdmissionError::ClockRollback)
    );
}

#[test]
fn duplicate_and_changed_requests_do_not_consume_more_queue_capacity() {
    let mut state = scheduler(AdmissionLimits::default());
    let original = request(1, 1, 1);
    state
        .enqueue(original.clone(), Timestamp::from_millis(1))
        .unwrap();
    assert_eq!(
        state.enqueue(original.clone(), Timestamp::from_millis(1)),
        Err(AdmissionError::Duplicate)
    );
    let changed = QueuedRun::new(
        original.binding().clone(),
        target(2),
        original.access(),
        original.expires(),
    );
    assert_eq!(
        state.enqueue(changed, Timestamp::from_millis(1)),
        Err(AdmissionError::Conflict)
    );
    assert_eq!(state.queued_count(), 1);
    let foreign: RunBinding = run(2, 1, 1, 1);
    assert!(state.cancel_queued(&foreign).is_none());
    assert_eq!(state.queued_count(), 1);
}

#[test]
fn different_account_authorities_do_not_share_a_mutation_lock() {
    let mut state = scheduler(AdmissionLimits::default());
    for account in [1, 2] {
        state
            .enqueue(
                QueuedRun::new(
                    run(account, 1, 1, 1),
                    target(1),
                    RunAccess::Mutating,
                    Timestamp::from_millis(1000),
                ),
                Timestamp::from_millis(1),
            )
            .unwrap();
    }
    let first = lease(&mut state);
    let second = lease(&mut state);
    assert_ne!(
        first.request().binding().scope(),
        second.request().binding().scope()
    );
}
