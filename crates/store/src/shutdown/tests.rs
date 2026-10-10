use super::*;
use std::sync::{Arc, Mutex};

use crate::{Publication, SnapshotWriter};
use futures::channel::oneshot;
use gpui::{AppContext as _, TestAppContext};

#[gpui::test]
async fn all_captures_precede_concurrent_background_drain(cx: &mut TestAppContext) {
    let captured = Arc::new(Mutex::new(0));
    let started = Arc::new(Mutex::new(0));
    let coordinator = ShutdownCoordinator::default();
    let mut releases = Vec::new();
    for _ in 0..2 {
        let (release, wait) = oneshot::channel();
        releases.push(release);
        let captured = captured.clone();
        let started = started.clone();
        coordinator
            .register_capture(move |_| {
                *captured.lock().unwrap() += 1;
                Box::pin(async move {
                    assert_eq!(*captured.lock().unwrap(), 2);
                    *started.lock().unwrap() += 1;
                    wait.await.unwrap();
                    Ok(())
                })
            })
            .unwrap();
    }
    let work = cx.update(|cx| coordinator.capture_all(cx));
    let task = cx.background_executor.spawn(work);
    cx.run_until_parked();
    assert_eq!(*started.lock().unwrap(), 2);
    for release in releases {
        release.send(()).unwrap();
    }
    let report = task.await;
    assert!(report.failures().is_empty());
    assert!(!report.timed_out());
}

#[gpui::test]
async fn one_shared_budget_expires_with_independent_blocked_participants(cx: &mut TestAppContext) {
    let coordinator = ShutdownCoordinator::default();
    let writer: SnapshotWriter<i32> = Arc::new(|_| Box::pin(futures::future::pending()));
    let first = cx.new(|_| Persisted::new(0, writer.clone(), Publication::Optimistic, 4));
    let second = cx.new(|_| Persisted::new(0, writer, Publication::Optimistic, 4));
    for entity in [&first, &second] {
        coordinator.register(entity).unwrap();
        cx.update(|cx| entity.update(cx, |state, cx| state.update(|v| *v = 1, cx).unwrap()));
    }
    let work = cx.update(|cx| coordinator.capture_with_budget(cx, Duration::from_millis(10)));
    let report = cx.background_executor.spawn(work).await;
    assert!(report.timed_out());
    assert!(matches!(report.failures(), [StoreError::DeadlineExceeded]));
}

#[gpui::test]
fn capture_closes_registration_before_async_drain(cx: &mut TestAppContext) {
    let coordinator = ShutdownCoordinator::default();
    let work = cx.update(|cx| coordinator.capture_all(cx));
    assert!(matches!(
        coordinator.register_capture(|_| Box::pin(async { Ok(()) })),
        Err(StoreError::Closing)
    ));
    cx.background_executor.spawn(work).detach();
    cx.run_until_parked();
}

#[gpui::test]
async fn deadline_report_retains_failures_from_completed_participants(cx: &mut TestAppContext) {
    let coordinator = ShutdownCoordinator::default();
    coordinator
        .register_capture(|_| {
            Box::pin(async {
                Err(StoreError::backend(std::io::Error::from(
                    std::io::ErrorKind::PermissionDenied,
                )))
            })
        })
        .unwrap();
    coordinator
        .register_capture(|_| Box::pin(futures::future::pending()))
        .unwrap();
    let work = cx.update(|cx| coordinator.capture_with_budget(cx, Duration::from_millis(10)));
    let report = cx.background_executor.spawn(work).await;
    assert!(report.timed_out());
    assert!(
        report
            .failures()
            .iter()
            .any(|e| matches!(e, StoreError::Backend(_)))
    );
    assert!(
        report
            .failures()
            .iter()
            .any(|e| matches!(e, StoreError::DeadlineExceeded))
    );
}

#[gpui::test]
async fn accepted_edits_survive_dropping_the_last_external_handle(cx: &mut TestAppContext) {
    let saved = Arc::new(Mutex::new(Vec::new()));
    let observed = saved.clone();
    let writer: SnapshotWriter<i32> = Arc::new(move |value| {
        let observed = observed.clone();
        Box::pin(async move {
            observed.lock().unwrap().push(value);
            Ok(())
        })
    });
    let coordinator = ShutdownCoordinator::default();
    let entity = cx.new(|_| Persisted::new(0, writer, Publication::Durable, 2));
    coordinator.register(&entity).unwrap();
    let receipt = entity.update(cx, |state, cx| {
        state.update(|value| *value = 42, cx).unwrap()
    });
    drop(entity);
    let work = cx.update(|cx| coordinator.capture_all(cx));
    let report = cx.background_executor.spawn(work).await;
    assert!(report.failures().is_empty());
    assert_eq!(receipt.wait().await.unwrap(), 1);
    assert_eq!(*saved.lock().unwrap(), [42]);
}
