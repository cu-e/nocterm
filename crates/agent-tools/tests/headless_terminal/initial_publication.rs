use crate::headless_support::*;
use futures::{FutureExt as _, task::LocalSpawnExt as _};
use nocterm_agent_tools::headless::{
    HeadlessTerminalOptions, InitialSessionCandidate, SessionLifecycleEvent,
};
use nocterm_cloud_model::{
    authority::RunBinding,
    identity::{RemoteSessionId, RunEpoch, RunId},
};
use nocterm_session::{Charset, Event};
use std::sync::Arc;

#[test]
fn initial_session_published_guard_closes_until_original_run_accepts_it() {
    for accepted in [false, true] {
        let (mut host, retained, _) = Harness::retained(Charset::Utf8);
        retained.remove(host.authority.session()).unwrap();
        let (session, driver) = nocterm_session::channel(None);
        let (candidate, mut observations, task) = InitialSessionCandidate::prepare(
            host.registry.clone(),
            RemoteSessionId::from_bytes([41; 16]),
            host.authority.session().clone(),
            session,
            HeadlessTerminalOptions::default(),
            host.timer.clone(),
            Arc::new(()),
        )
        .unwrap();
        host.pool.spawner().spawn_local(task.into_run()).unwrap();
        host.pool.run_until(driver.emit(Event::Connected));
        host.pump();
        let connected = observations.recv().now_or_never().unwrap().unwrap();
        assert!(matches!(
            connected.event(),
            SessionLifecycleEvent::Connected
        ));
        let published = candidate.publish_owned().unwrap();
        assert_eq!(published.receipt(), connected.receipt());
        assert_eq!(published.generation(), connected.generation());
        assert!(driver.closed().now_or_never().is_none());
        if accepted {
            published.accept();
        } else {
            drop(published);
        }
        host.pump();
        assert_eq!(driver.closed().now_or_never().is_some(), !accepted);
        if !accepted {
            assert!(matches!(
                observations.recv().now_or_never().unwrap().unwrap().event(),
                SessionLifecycleEvent::Lost { .. }
            ));
        }
    }
}

#[test]
fn initial_session_old_published_guard_does_not_remove_a_new_physical_generation() {
    let (mut host, retained, _) = Harness::retained(Charset::Utf8);
    retained.remove(host.authority.session()).unwrap();
    let (session, original_driver) = nocterm_session::channel(None);
    let (candidate, mut observations, task) = InitialSessionCandidate::prepare(
        host.registry.clone(),
        RemoteSessionId::from_bytes([42; 16]),
        host.authority.session().clone(),
        session,
        HeadlessTerminalOptions::default(),
        host.timer.clone(),
        Arc::new(()),
    )
    .unwrap();
    host.pool.spawner().spawn_local(task.into_run()).unwrap();
    host.pool.run_until(original_driver.emit(Event::Connected));
    host.pump();
    assert!(matches!(
        observations.recv().now_or_never().unwrap().unwrap().event(),
        SessionLifecycleEvent::Connected
    ));
    let old = candidate.publish_owned().unwrap();
    retained.remove(host.authority.session()).unwrap();
    host.pump();
    assert!(original_driver.closed().now_or_never().is_some());
    let original_loss = observations.recv().now_or_never().unwrap().unwrap();
    let (session, replacement_driver) = nocterm_session::channel(None);
    let (replacement, mut observations, task) = InitialSessionCandidate::prepare(
        host.registry.clone(),
        RemoteSessionId::from_bytes([43; 16]),
        host.authority.session().clone(),
        session,
        HeadlessTerminalOptions::default(),
        host.timer.clone(),
        Arc::new(()),
    )
    .unwrap();
    assert_ne!(replacement.generation(), old.generation());
    host.pool.spawner().spawn_local(task.into_run()).unwrap();
    host.pool
        .run_until(replacement_driver.emit(Event::Connected));
    host.pump();
    assert!(matches!(
        observations.recv().now_or_never().unwrap().unwrap().event(),
        SessionLifecycleEvent::Connected
    ));
    replacement.publish_owned().unwrap().accept();
    assert!(!retained.retire_observed(&original_loss));
    drop(old);
    host.pump();
    assert!(replacement_driver.closed().now_or_never().is_none());
    let mut view = retained.view(host.authority.run().clone()).unwrap();
    view.attach_retained(
        RemoteSessionId::from_bytes([44; 16]),
        host.authority.session(),
    )
    .unwrap();
}

#[test]
fn initial_session_old_guard_cannot_close_the_same_physical_after_another_view_accepts_it() {
    let (mut host, retained, _) = Harness::retained(Charset::Utf8);
    retained.remove(host.authority.session()).unwrap();
    let (session, driver) = nocterm_session::channel(None);
    let (candidate, mut observations, task) = InitialSessionCandidate::prepare(
        host.registry.clone(),
        RemoteSessionId::from_bytes([45; 16]),
        host.authority.session().clone(),
        session,
        HeadlessTerminalOptions::default(),
        host.timer.clone(),
        Arc::new(()),
    )
    .unwrap();
    host.pool.spawner().spawn_local(task.into_run()).unwrap();
    host.pool.run_until(driver.emit(Event::Connected));
    host.pump();
    assert!(matches!(
        observations.recv().now_or_never().unwrap().unwrap().event(),
        SessionLifecycleEvent::Connected
    ));
    let old = candidate.publish_owned().unwrap();
    let original = host.authority.run();
    let next_run = RunBinding::new(
        original.scope().clone(),
        original.runner(),
        original.runner_epoch(),
        original.chat(),
        RunId::from_bytes([48; 16]),
        RunEpoch::new(2).unwrap(),
    );
    let mut next = retained.view(next_run.clone()).unwrap();
    next.attach_retained(RemoteSessionId::from_bytes([46; 16]), old.receipt())
        .unwrap();
    drop(old);
    host.pump();
    assert!(driver.closed().now_or_never().is_none());
    assert!(observations.recv().now_or_never().is_none());
    let mut later = retained.view(next_run).unwrap();
    later
        .attach_retained(
            RemoteSessionId::from_bytes([47; 16]),
            host.authority.session(),
        )
        .unwrap();
    retained.remove(host.authority.session()).unwrap();
    host.pump();
    assert!(driver.closed().now_or_never().is_some());
}

#[test]
fn initial_session_observed_loss_releases_the_original_retained_slot() {
    let (mut host, retained, _) = Harness::retained(Charset::Utf8);
    retained.remove(host.authority.session()).unwrap();
    let (session, driver) = nocterm_session::channel(None);
    let (candidate, mut observations, task) = InitialSessionCandidate::prepare(
        host.registry.clone(),
        RemoteSessionId::from_bytes([51; 16]),
        host.authority.session().clone(),
        session,
        HeadlessTerminalOptions::default(),
        host.timer.clone(),
        Arc::new(()),
    )
    .unwrap();
    host.pool.spawner().spawn_local(task.into_run()).unwrap();
    host.pool.run_until(driver.emit(Event::Connected));
    host.pump();
    let connected = observations.recv().now_or_never().unwrap().unwrap();
    assert!(!retained.retire_observed(&connected));
    candidate.publish_owned().unwrap().accept();
    host.pool
        .run_until(driver.emit(Event::Closed(nocterm_session::CloseReason::ClosedByUser)));
    host.pump();
    let lost = observations.recv().now_or_never().unwrap().unwrap();
    assert!(retained.retire_observed(&lost));
    assert!(!retained.retire_observed(&lost));
    assert!(retained.remove(host.authority.session()).is_err());
}
