use crate::{Persisted, Publication, SnapshotWriter, StoreError};
use futures::{FutureExt as _, channel::oneshot};
use gpui::{AppContext as _, TestAppContext};
use std::{
    cell::Cell,
    rc::Rc,
    sync::{Arc, Mutex},
};

#[gpui::test]
fn publication_is_independent_of_receipt_consumers_and_precedes_completion(
    cx: &mut TestAppContext,
) {
    let observed = Rc::new(Cell::new(0));
    let hook = observed.clone();
    let entity = cx.new(|_| {
        Persisted::new(
            0,
            Arc::new(|_| Box::pin(async { Ok(()) })),
            Publication::Durable,
            4,
        )
        .with_publication_hook(move |old, new, _| {
            assert_eq!(*old, 0);
            hook.set(*new);
        })
    });
    let receipt =
        cx.update(|cx| entity.update(cx, |state, cx| state.update(|v| *v = 1, cx).unwrap()));
    cx.run_until_parked();
    assert_eq!(observed.get(), 1);
    assert_eq!(futures::executor::block_on(receipt.wait()).unwrap(), 1);
    cx.update(|cx| {
        entity.update(cx, |state, cx| {
            state
                .update_checked(|_| Err(StoreError::PendingWrites), cx)
                .unwrap()
        })
    });
    cx.run_until_parked();
    assert_eq!(observed.get(), 1);
}

#[gpui::test]
async fn physically_successful_active_prefix_survives_cancelled_ack_and_final_failure(
    cx: &mut TestAppContext,
) {
    let (release, blocked) = oneshot::channel();
    let blocked = Arc::new(Mutex::new(Some(blocked)));
    let writer: SnapshotWriter<i32> = Arc::new(move |value| {
        let blocked = blocked.lock().unwrap().take();
        Box::pin(async move {
            if let Some(blocked) = blocked {
                blocked.await.unwrap();
            }
            if value == 2 {
                Err(StoreError::PendingWrites)
            } else {
                Ok(())
            }
        })
    });
    let entity = cx.new(|_| Persisted::new(0, writer, Publication::Durable, 4));
    let active =
        cx.update(|cx| entity.update(cx, |state, cx| state.update(|v| *v = 1, cx).unwrap()));
    cx.run_until_parked();
    let final_receipt =
        cx.update(|cx| entity.update(cx, |state, cx| state.update(|v| *v = 2, cx).unwrap()));
    let captured = cx.update(|cx| entity.update(cx, |state, _| state.capture()));
    drop(entity);
    let task = cx.background_executor.spawn(captured.run());
    release.send(()).unwrap();
    let outcome = task.await;
    assert!(outcome.result().is_err());
    assert_eq!(*outcome.committed().snapshot(), 1);
    assert_eq!(outcome.committed().revision(), 1);
    assert_eq!(active.wait().await.unwrap(), 1);
    assert!(final_receipt.wait().await.is_err());
}

#[gpui::test]
async fn optimistic_saved_prefix_receipt_survives_cancelled_ack_and_failed_final_write(
    cx: &mut TestAppContext,
) {
    let (release, blocked) = oneshot::channel();
    let blocked = Arc::new(Mutex::new(Some(blocked)));
    let writes = Arc::new(Mutex::new(Vec::new()));
    let saved = writes.clone();
    let writer: SnapshotWriter<i32> = Arc::new(move |value| {
        let wait = blocked.lock().unwrap().take();
        let saved = saved.clone();
        Box::pin(async move {
            if let Some(wait) = wait {
                wait.await.unwrap();
            }
            if value == 2 {
                return Err(StoreError::PendingWrites);
            }
            saved.lock().unwrap().push(value);
            Ok(())
        })
    });
    let entity = cx.new(|_| Persisted::new(0, writer, Publication::Optimistic, 4));
    let active = entity.update(cx, |s, cx| s.update(|v| *v = 1, cx).unwrap());
    cx.run_until_parked();
    let final_receipt = entity.update(cx, |s, cx| s.update(|v| *v = 2, cx).unwrap());
    let work = entity.update(cx, |s, _| s.capture());
    drop(entity);
    let task = cx.background_executor.spawn(work.run());
    release.send(()).unwrap();
    let outcome = task.await;
    assert!(matches!(outcome.result(), Err(StoreError::PendingWrites)));
    assert_eq!(*writes.lock().unwrap(), [1]);
    assert_eq!(*outcome.committed().snapshot(), 1);
    assert_eq!(outcome.committed().revision(), 1);
    assert_eq!(active.wait().await.unwrap(), 1);
    assert!(matches!(
        final_receipt.wait().await,
        Err(StoreError::PendingWrites)
    ));
}

#[gpui::test]
async fn optimistic_saved_prefix_receipt_keeps_own_revision_when_final_write_succeeds(
    cx: &mut TestAppContext,
) {
    let (release, blocked) = oneshot::channel();
    let blocked = Arc::new(Mutex::new(Some(blocked)));
    let writer: SnapshotWriter<i32> = Arc::new(move |_| {
        let wait = blocked.lock().unwrap().take();
        Box::pin(async move {
            if let Some(wait) = wait {
                wait.await.unwrap();
            }
            Ok(())
        })
    });
    let entity = cx.new(|_| Persisted::new(0, writer, Publication::Optimistic, 4));
    let active = entity.update(cx, |s, cx| s.update(|v| *v = 1, cx).unwrap());
    cx.run_until_parked();
    let final_receipt = entity.update(cx, |s, cx| s.update(|v| *v = 2, cx).unwrap());
    let work = entity.update(cx, |s, _| s.capture());
    drop(entity);
    let task = cx.background_executor.spawn(work.run());
    release.send(()).unwrap();
    let outcome = task.await;
    outcome.result().unwrap();
    assert_eq!(*outcome.committed().snapshot(), 2);
    assert_eq!(outcome.committed().revision(), 2);
    assert_eq!(active.wait().await.unwrap(), 1);
    assert_eq!(final_receipt.wait().await.unwrap(), 2);
}

#[gpui::test]
async fn projected_failed_baseline_preserves_saved_prefix_before_dependency_settles(
    cx: &mut TestAppContext,
) {
    let producer = cx.new(|_| {
        Persisted::new(
            0,
            Arc::new(|_| Box::pin(async { Ok(()) })),
            Publication::Durable,
            1,
        )
    });
    let (release, blocked) = oneshot::channel();
    let blocked = Arc::new(Mutex::new(Some(blocked)));
    let writer: SnapshotWriter<i32> = Arc::new(move |value| {
        let wait = blocked.lock().unwrap().take();
        Box::pin(async move {
            if let Some(wait) = wait {
                wait.await.unwrap();
            }
            if value == 2 {
                Err(StoreError::PendingWrites)
            } else {
                Ok(())
            }
        })
    });
    let receiver = cx.new(|_| Persisted::new(0, writer, Publication::Optimistic, 4));
    let active = receiver.update(cx, |s, cx| s.update(|v| *v = 1, cx).unwrap());
    cx.run_until_parked();
    let final_receipt = receiver.update(cx, |s, cx| s.update(|v| *v = 2, cx).unwrap());
    let producer_work = producer.update(cx, |s, _| s.capture());
    let receiver_work = receiver.update(cx, |s, _| s.capture());
    let (producer_work, feed) = producer_work.run_observed();
    let receiver_work = receiver_work.run_projected(feed, |_, _| Ok(None));
    drop((producer, receiver));
    let receiver_task = cx.background_executor.spawn(receiver_work);
    release.send(()).unwrap();
    cx.run_until_parked();
    assert!(
        matches!(active.wait().now_or_never(), Some(Ok(1))),
        "the physically saved prefix receipt must settle while the dependency is pending"
    );
    drop(producer_work);
    let outcome = receiver_task.await;
    assert!(matches!(outcome.result(), Err(StoreError::PendingWrites)));
    assert_eq!(*outcome.committed().snapshot(), 1);
    assert_eq!(outcome.committed().revision(), 1);
    assert!(matches!(
        final_receipt.wait().await,
        Err(StoreError::PendingWrites)
    ));
}

#[gpui::test]
async fn projected_baseline_receipt_and_active_prefix_do_not_wait_for_final_producer(
    cx: &mut TestAppContext,
) {
    let (release_active, active_blocked) = oneshot::channel();
    let (release_final, final_blocked) = oneshot::channel();
    let blockers = Arc::new(Mutex::new(vec![final_blocked, active_blocked]));
    let writer: SnapshotWriter<i32> = Arc::new(move |_| {
        let blocked = blockers.lock().unwrap().pop().unwrap();
        Box::pin(async move {
            blocked.await.unwrap();
            Ok(())
        })
    });
    let producer = cx.new(|_| Persisted::new(0, writer, Publication::Durable, 4));
    cx.update(|cx| producer.update(cx, |s, cx| s.update(|v| *v = 1, cx).unwrap()));
    cx.run_until_parked();
    cx.update(|cx| producer.update(cx, |s, cx| s.update(|v| *v = 2, cx).unwrap()));
    let writes = Arc::new(Mutex::new(Vec::new()));
    let saved = writes.clone();
    let receiver = cx.new(|_| {
        Persisted::new(
            0,
            Arc::new(move |v| {
                saved.lock().unwrap().push(v);
                Box::pin(async { Ok(()) })
            }),
            Publication::Optimistic,
            4,
        )
    });
    let baseline_receipt =
        cx.update(|cx| receiver.update(cx, |s, cx| s.update(|v| *v = 10, cx).unwrap()));
    let producer_work = cx.update(|cx| producer.update(cx, |s, _| s.capture()));
    let receiver_work = cx.update(|cx| receiver.update(cx, |s, _| s.capture()));
    let (producer_work, feed) = producer_work.run_observed();
    let receiver_work = receiver_work.run_projected(feed, |_, committed| {
        Ok((*committed.snapshot() != 0).then(|| 10 + committed.snapshot()))
    });
    drop((producer, receiver));
    let producer_task = cx.background_executor.spawn(producer_work);
    let receiver_task = cx.background_executor.spawn(receiver_work);
    cx.run_until_parked();
    assert_eq!(*writes.lock().unwrap(), [10]);
    assert_eq!(baseline_receipt.wait().await.unwrap(), 1);
    release_active.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(*writes.lock().unwrap(), [10, 11]);
    release_final.send(()).unwrap();
    producer_task.await.into_result().unwrap();
    receiver_task.await.into_result().unwrap();
    assert_eq!(*writes.lock().unwrap(), [10, 11, 12]);
}

#[gpui::test]
async fn failed_baseline_is_covered_by_prefix_success(cx: &mut TestAppContext) {
    let producer = cx.new(|_| {
        Persisted::new(
            1,
            Arc::new(|_| Box::pin(async { Ok(()) })),
            Publication::Durable,
            1,
        )
    });
    let receiver = cx.new(|_| {
        Persisted::new(
            0,
            Arc::new(|v| {
                Box::pin(async move {
                    if v == 10 {
                        Err(StoreError::PendingWrites)
                    } else {
                        Ok(())
                    }
                })
            }),
            Publication::Optimistic,
            1,
        )
    });
    let receipt = cx.update(|cx| receiver.update(cx, |s, cx| s.update(|v| *v = 10, cx).unwrap()));
    let p = cx.update(|cx| producer.update(cx, |s, _| s.capture()));
    let r = cx.update(|cx| receiver.update(cx, |s, _| s.capture()));
    let (p, feed) = p.run_observed();
    let r = r.run_projected(feed, |_, committed| Ok(Some(10 + committed.snapshot())));
    let p = cx.background_executor.spawn(p);
    let r = cx.background_executor.spawn(r);
    p.await.into_result().unwrap();
    assert_eq!(*r.await.committed().snapshot(), 11);
    assert_eq!(receipt.wait().await.unwrap(), 1);
}

#[gpui::test]
async fn later_projection_failure_keeps_successful_baseline_receipt(cx: &mut TestAppContext) {
    let producer = cx.new(|_| {
        Persisted::new(
            1,
            Arc::new(|_| Box::pin(async { Ok(()) })),
            Publication::Durable,
            1,
        )
    });
    let receiver = cx.new(|_| {
        Persisted::new(
            0,
            Arc::new(|value| {
                Box::pin(async move {
                    if value == 10 {
                        Ok(())
                    } else {
                        Err(StoreError::PendingWrites)
                    }
                })
            }),
            Publication::Optimistic,
            1,
        )
    });
    let receipt = cx.update(|cx| receiver.update(cx, |s, cx| s.update(|v| *v = 10, cx).unwrap()));
    let p = cx.update(|cx| producer.update(cx, |s, _| s.capture()));
    let r = cx.update(|cx| receiver.update(cx, |s, _| s.capture()));
    let (p, feed) = p.run_observed();
    let r = r.run_projected(feed, |_, _| Ok(Some(11)));
    let p = cx.background_executor.spawn(p);
    let r = cx.background_executor.spawn(r);
    p.await.into_result().unwrap();
    let outcome = r.await;
    assert!(outcome.result().is_err());
    assert_eq!(*outcome.committed().snapshot(), 10);
    assert_eq!(receipt.wait().await.unwrap(), 1);
}

#[gpui::test]
async fn dropped_dependency_reports_interruption_without_revoking_saved_baseline(
    cx: &mut TestAppContext,
) {
    let producer = cx.new(|_| {
        Persisted::new(
            0,
            Arc::new(|_| Box::pin(async { Ok(()) })),
            Publication::Durable,
            1,
        )
    });
    let receiver = cx.new(|_| {
        Persisted::new(
            0,
            Arc::new(|_| Box::pin(async { Ok(()) })),
            Publication::Optimistic,
            1,
        )
    });
    let receipt = cx.update(|cx| receiver.update(cx, |s, cx| s.update(|v| *v = 10, cx).unwrap()));
    let p = cx.update(|cx| producer.update(cx, |s, _| s.capture()));
    let r = cx.update(|cx| receiver.update(cx, |s, _| s.capture()));
    let (p, feed) = p.run_observed();
    drop(p);
    let r = r.run_projected(feed, |_, _| Ok(Some(11)));
    let outcome = cx.background_executor.spawn(r).await;
    assert!(matches!(
        outcome.result(),
        Err(StoreError::CompletionDropped)
    ));
    assert_eq!(*outcome.committed().snapshot(), 10);
    assert_eq!(receipt.wait().await.unwrap(), 1);
}

#[gpui::test]
async fn old_normal_and_blocked_prefix_cannot_overwrite_final_projection(cx: &mut TestAppContext) {
    let producer = cx.new(|_| {
        Persisted::new(
            0,
            Arc::new(|_| Box::pin(async { Ok(()) })),
            Publication::Durable,
            4,
        )
    });
    cx.update(|cx| producer.update(cx, |s, cx| s.update(|v| *v = 1, cx).unwrap()));
    let (release_old, old_blocked) = oneshot::channel();
    let (release_prefix, prefix_blocked) = oneshot::channel();
    let blockers = Arc::new(Mutex::new(std::collections::HashMap::from([
        (5, old_blocked),
        (11, prefix_blocked),
    ])));
    let writes = Arc::new(Mutex::new(Vec::new()));
    let saved = writes.clone();
    let writer: SnapshotWriter<i32> = Arc::new(move |value| {
        let blocked = blockers.lock().unwrap().remove(&value);
        let saved = saved.clone();
        Box::pin(async move {
            if let Some(blocked) = blocked {
                blocked.await.unwrap();
            }
            saved.lock().unwrap().push(value);
            Ok(())
        })
    });
    let receiver = cx.new(|_| Persisted::new(0, writer, Publication::Optimistic, 4));
    cx.update(|cx| receiver.update(cx, |s, cx| s.update(|v| *v = 5, cx).unwrap()));
    cx.run_until_parked();
    let receipt = cx.update(|cx| receiver.update(cx, |s, cx| s.update(|v| *v = 10, cx).unwrap()));
    let p = cx.update(|cx| producer.update(cx, |s, _| s.capture()));
    let r = cx.update(|cx| receiver.update(cx, |s, _| s.capture()));
    let (p, feed) = p.run_observed();
    let r = r.run_projected(feed, |phase, _| {
        Ok(Some(match phase {
            super::CommitPhase::Prefix => 11,
            super::CommitPhase::Final => 12,
        }))
    });
    let p = cx.background_executor.spawn(p);
    let r = cx.background_executor.spawn(r);
    cx.run_until_parked();
    assert!(writes.lock().unwrap().is_empty());
    release_old.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(*writes.lock().unwrap(), [5, 10]);
    assert_eq!(receipt.wait().await.unwrap(), 2);
    release_prefix.send(()).unwrap();
    p.await.into_result().unwrap();
    let outcome = r.await;
    outcome.result().unwrap();
    assert_eq!(*outcome.committed().snapshot(), 12);
    assert_eq!(*writes.lock().unwrap(), [5, 10, 11, 12]);
}

#[gpui::test]
async fn optimistic_failed_prefix_receipt_uses_successful_final_covering_revision(
    cx: &mut TestAppContext,
) {
    let (release, blocked) = oneshot::channel();
    let blocked = Arc::new(Mutex::new(Some(blocked)));
    let saved = Arc::new(Mutex::new(Vec::new()));
    let written = saved.clone();
    let writer: SnapshotWriter<i32> = Arc::new(move |value| {
        let wait = blocked.lock().unwrap().take();
        let written = written.clone();
        Box::pin(async move {
            if let Some(wait) = wait {
                wait.await.unwrap();
            }
            if value == 1 {
                return Err(StoreError::PendingWrites);
            }
            written.lock().unwrap().push(value);
            Ok(())
        })
    });
    let entity = cx.new(|_| Persisted::new(0, writer, Publication::Optimistic, 4));
    let active = entity.update(cx, |s, cx| s.update(|v| *v = 1, cx).unwrap());
    cx.run_until_parked();
    let final_receipt = entity.update(cx, |s, cx| s.update(|v| *v = 2, cx).unwrap());
    let captured = entity.update(cx, |s, _| s.capture());
    drop(entity);
    let task = cx.background_executor.spawn(captured.run());
    release.send(()).unwrap();
    let outcome = task.await;
    outcome.result().unwrap();
    assert_eq!(*saved.lock().unwrap(), [2]);
    assert_eq!(*outcome.committed().snapshot(), 2);
    assert_eq!(outcome.committed().revision(), 2);
    assert_eq!(active.wait().await.unwrap(), 2);
    assert_eq!(final_receipt.wait().await.unwrap(), 2);
}

#[gpui::test]
async fn projected_successful_final_covers_failed_prefix_before_dependency_settles(
    cx: &mut TestAppContext,
) {
    let producer = cx.new(|_| {
        Persisted::new(
            0,
            Arc::new(|_| Box::pin(async { Ok(()) })),
            Publication::Durable,
            1,
        )
    });
    let (release, blocked) = oneshot::channel();
    let blocked = Arc::new(Mutex::new(Some(blocked)));
    let saved = Arc::new(Mutex::new(Vec::new()));
    let written = saved.clone();
    let writer: SnapshotWriter<i32> = Arc::new(move |value| {
        let wait = blocked.lock().unwrap().take();
        let written = written.clone();
        Box::pin(async move {
            if let Some(wait) = wait {
                wait.await.unwrap();
            }
            if value == 1 {
                return Err(StoreError::PendingWrites);
            }
            written.lock().unwrap().push(value);
            Ok(())
        })
    });
    let receiver = cx.new(|_| Persisted::new(0, writer, Publication::Optimistic, 4));
    let active = receiver.update(cx, |s, cx| s.update(|v| *v = 1, cx).unwrap());
    cx.run_until_parked();
    let final_receipt = receiver.update(cx, |s, cx| s.update(|v| *v = 2, cx).unwrap());
    let producer_work = producer.update(cx, |s, _| s.capture());
    let receiver_work = receiver.update(cx, |s, _| s.capture());
    let (producer_work, feed) = producer_work.run_observed();
    drop((producer, receiver));
    let task = cx
        .background_executor
        .spawn(receiver_work.run_projected(feed, |_, _| Ok(None)));
    release.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(*saved.lock().unwrap(), [2]);
    assert!(matches!(active.wait().now_or_never(), Some(Ok(2))));
    assert!(matches!(final_receipt.wait().now_or_never(), Some(Ok(2))));
    drop(producer_work);
    let outcome = task.await;
    assert!(matches!(
        outcome.result(),
        Err(StoreError::CompletionDropped)
    ));
    assert_eq!(*outcome.committed().snapshot(), 2);
    assert_eq!(outcome.committed().revision(), 2);
}
