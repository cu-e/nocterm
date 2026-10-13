use super::*;
use gpui::{AppContext as _, TestAppContext};
use std::error::Error as _;

fn recording(writes: &Arc<Mutex<Vec<i32>>>) -> SnapshotWriter<i32> {
    let writes = writes.clone();
    Arc::new(move |value| {
        let writes = writes.clone();
        Box::pin(async move {
            writes.lock().unwrap().push(value);
            Ok(())
        })
    })
}

#[gpui::test]
fn optimistic_changes_are_visible_immediately_and_coalesce(cx: &mut TestAppContext) {
    let writes = Arc::default();
    let entity = cx.new(|_| Persisted::new(0, recording(&writes), Publication::Optimistic, 10));
    let receipts = cx.update(|cx| {
        (1..=3)
            .map(|value| {
                entity.update(cx, |state, cx| {
                    state.update(move |state| *state = value, cx).unwrap()
                })
            })
            .collect::<Vec<_>>()
    });
    cx.update(|cx| assert_eq!(*entity.read(cx).visible(), 3));
    cx.run_until_parked();
    assert_eq!(*writes.lock().unwrap(), [3]);
    for receipt in receipts {
        assert_eq!(futures::executor::block_on(receipt.wait()).unwrap(), 3);
    }
}

#[gpui::test]
async fn shutdown_waits_for_inflight_and_outlives_the_entity(cx: &mut TestAppContext) {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let (release, blocked) = oneshot::channel();
    let blocked = Arc::new(Mutex::new(Some(blocked)));
    let writer: SnapshotWriter<i32> = {
        let writes = writes.clone();
        Arc::new(move |value| {
            let blocked = blocked.lock().unwrap().take();
            let writes = writes.clone();
            Box::pin(async move {
                if let Some(blocked) = blocked {
                    blocked.await.unwrap();
                }
                writes.lock().unwrap().push(value);
                Ok(())
            })
        })
    };
    let entity = cx.new(|_| Persisted::new(0, writer, Publication::Optimistic, 10));
    let first =
        cx.update(|cx| entity.update(cx, |state, cx| state.update(|v| *v = 1, cx).unwrap()));
    cx.run_until_parked();
    let second =
        cx.update(|cx| entity.update(cx, |state, cx| state.update(|v| *v = 2, cx).unwrap()));
    let final_work = cx.update(|cx| entity.update(cx, |state, _| state.capture_shutdown()));
    drop(entity);
    let final_task = cx.background_executor.spawn(final_work);
    cx.run_until_parked();
    assert!(writes.lock().unwrap().is_empty());
    release.send(()).unwrap();
    final_task.await.unwrap();
    assert_eq!(*writes.lock().unwrap(), [1, 2]);
    // This receipt belongs to the physically saved prefix, not the final write.
    assert_eq!(first.wait().await.unwrap(), 1);
    assert_eq!(second.wait().await.unwrap(), 2);
}

#[gpui::test]
fn failed_optimistic_write_keeps_desired_and_retry_advances_durable(cx: &mut TestAppContext) {
    let attempts = Arc::new(Mutex::new(0));
    let writer: SnapshotWriter<i32> = Arc::new(move |_| {
        let mut count = attempts.lock().unwrap();
        *count += 1;
        let fail = *count == 1;
        Box::pin(async move {
            if fail {
                Err(StoreError::backend(nocterm_core::PersistError::Io {
                    path: "state.toml".into(),
                    source: std::io::Error::from(std::io::ErrorKind::StorageFull),
                }))
            } else {
                Ok(())
            }
        })
    });
    let entity = cx.new(|_| Persisted::new(0, writer, Publication::Optimistic, 10));
    let receipt =
        cx.update(|cx| entity.update(cx, |state, cx| state.update(|v| *v = 1, cx).unwrap()));
    cx.run_until_parked();
    let error = futures::executor::block_on(receipt.wait()).unwrap_err();
    let backend = error.source().unwrap().source().unwrap();
    assert!(
        backend
            .downcast_ref::<nocterm_core::PersistError>()
            .is_some()
    );
    assert_eq!(
        backend
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::StorageFull
    );
    cx.update(|cx| {
        assert_eq!(*entity.read(cx).visible(), 1);
        assert_eq!(*entity.read(cx).durable(), 0);
        assert!(entity.read(cx).error().is_some());
        entity.update(cx, |state, cx| state.update(|v| *v = 1, cx).unwrap());
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(*entity.read(cx).durable(), 1);
        assert!(entity.read(cx).error().is_none());
        assert!(!entity.read(cx).is_dirty());
    });
}

#[gpui::test]
fn durable_edits_do_not_inherit_a_failed_predecessor(cx: &mut TestAppContext) {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let writer: SnapshotWriter<i32> = {
        let writes = writes.clone();
        Arc::new(move |v| {
            writes.lock().unwrap().push(v);
            Box::pin(async move {
                if v == 1 {
                    Err(StoreError::backend(std::io::Error::from(
                        std::io::ErrorKind::PermissionDenied,
                    )))
                } else {
                    Ok(())
                }
            })
        })
    };
    let entity = cx.new(|_| Persisted::new(0, writer, Publication::Durable, 10));
    cx.update(|cx| {
        entity.update(cx, |state, cx| state.update(|v| *v += 1, cx).unwrap());
        entity.update(cx, |state, cx| state.update(|v| *v += 2, cx).unwrap());
        assert_eq!(*entity.read(cx).visible(), 0);
    });
    cx.run_until_parked();
    assert_eq!(*writes.lock().unwrap(), [1, 2]);
    cx.update(|cx| assert_eq!(*entity.read(cx).visible(), 2));
}

#[gpui::test]
fn full_or_closed_admission_changes_no_visible_state(cx: &mut TestAppContext) {
    let writes = Arc::default();
    let entity = cx.new(|_| Persisted::new(0, recording(&writes), Publication::Optimistic, 1));
    cx.update(|cx| {
        entity.update(cx, |state, cx| {
            state.update(|v| *v = 1, cx).unwrap();
            assert!(matches!(
                state.update(|v| *v = 2, cx),
                Err(StoreError::Full)
            ));
            assert_eq!(*state.visible(), 1);
            let work = state.capture_shutdown();
            assert!(matches!(
                state.update(|v| *v = 3, cx),
                Err(StoreError::Closing)
            ));
            cx.background_executor().spawn(work).detach();
        })
    });
    cx.run_until_parked();
    assert_eq!(*writes.lock().unwrap(), [1]);
}

#[gpui::test]
fn unchanged_clean_edits_do_not_write(cx: &mut TestAppContext) {
    let writes = Arc::default();
    let entity = cx.new(|_| Persisted::new(0, recording(&writes), Publication::Optimistic, 10));
    let receipt =
        cx.update(|cx| entity.update(cx, |state, cx| state.update(|v| *v = 0, cx).unwrap()));
    assert_eq!(futures::executor::block_on(receipt.wait()).unwrap(), 0);
    let work = cx.update(|cx| entity.update(cx, |state, _| state.capture_shutdown()));
    cx.background_executor.spawn(work).detach();
    cx.run_until_parked();
    assert!(writes.lock().unwrap().is_empty());
}

#[gpui::test]
async fn blocked_writer_coalesces_a_thousand_changes_into_one_followup(cx: &mut TestAppContext) {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let (release, wait) = oneshot::channel();
    let wait = Arc::new(Mutex::new(Some(wait)));
    let writer: SnapshotWriter<i32> = {
        let writes = writes.clone();
        Arc::new(move |value| {
            let wait = wait.lock().unwrap().take();
            let writes = writes.clone();
            Box::pin(async move {
                if let Some(wait) = wait {
                    wait.await.unwrap();
                }
                writes.lock().unwrap().push(value);
                Ok(())
            })
        })
    };
    let entity = cx.new(|_| Persisted::new(0, writer, Publication::Optimistic, 4096));
    cx.update(|cx| entity.update(cx, |state, cx| state.update(|v| *v = 1, cx).unwrap()));
    cx.run_until_parked();
    for value in 2..=1001 {
        cx.update(|cx| {
            entity.update(cx, |state, cx| {
                state.update(move |v| *v = value, cx).unwrap()
            })
        });
    }
    cx.run_until_parked();
    assert!(writes.lock().unwrap().is_empty());
    release.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(*writes.lock().unwrap(), [1, 1001]);
    cx.update(|cx| assert_eq!(*entity.read(cx).durable(), 1001));
}

#[gpui::test]
async fn durable_shutdown_captures_all_edits_into_one_atomic_snapshot(cx: &mut TestAppContext) {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let writer: SnapshotWriter<i32> = {
        let writes = writes.clone();
        Arc::new(move |value| {
            let writes = writes.clone();
            Box::pin(async move {
                writes.lock().unwrap().push(value);
                if value == 1 {
                    Err(StoreError::backend(std::io::Error::from(
                        std::io::ErrorKind::PermissionDenied,
                    )))
                } else {
                    Ok(())
                }
            })
        })
    };
    let entity = cx.new(|_| Persisted::new(0, writer, Publication::Durable, 4));
    let (first, second, final_work) = cx.update(|cx| {
        entity.update(cx, |state, cx| {
            let first = state.update(|v| *v += 1, cx).unwrap();
            let second = state.update(|v| *v += 2, cx).unwrap();
            (first, second, state.capture_shutdown())
        })
    });
    drop(entity);
    cx.background_executor.spawn(final_work).await.unwrap();
    assert_eq!(first.wait().await.unwrap(), 2);
    assert_eq!(second.wait().await.unwrap(), 2);
    assert_eq!(*writes.lock().unwrap(), [3]);
}

#[gpui::test]
async fn final_snapshot_restores_a_reverted_value_after_an_inflight_write(cx: &mut TestAppContext) {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let (release, wait) = oneshot::channel();
    let wait = Arc::new(Mutex::new(Some(wait)));
    let writer: SnapshotWriter<i32> = {
        let writes = writes.clone();
        Arc::new(move |value| {
            let wait = wait.lock().unwrap().take();
            let writes = writes.clone();
            Box::pin(async move {
                if let Some(wait) = wait {
                    wait.await.unwrap();
                }
                writes.lock().unwrap().push(value);
                Ok(())
            })
        })
    };
    let entity = cx.new(|_| Persisted::new(0, writer, Publication::Optimistic, 4));
    cx.update(|cx| entity.update(cx, |state, cx| state.update(|v| *v = 1, cx).unwrap()));
    cx.run_until_parked();
    let work = cx.update(|cx| {
        entity.update(cx, |state, cx| {
            state.update(|v| *v = 0, cx).unwrap();
            state.capture_shutdown()
        })
    });
    release.send(()).unwrap();
    cx.background_executor.spawn(work).await.unwrap();
    assert_eq!(*writes.lock().unwrap(), [1, 0]);
}

#[gpui::test]
async fn shutdown_applies_local_edit_callbacks_before_returning_background_work(
    cx: &mut TestAppContext,
) {
    let writes = Arc::default();
    let applied = std::rc::Rc::new(std::cell::Cell::new(false));
    let entity = cx.new(|_| Persisted::new(0, recording(&writes), Publication::Durable, 4));
    let edit_applied = applied.clone();
    let work = cx.update(|cx| {
        entity.update(cx, |state, cx| {
            state
                .update(
                    move |value| {
                        edit_applied.set(true);
                        *value = 7;
                    },
                    cx,
                )
                .unwrap();
            state.capture_shutdown()
        })
    });
    assert!(applied.get());
    drop(entity);
    cx.background_executor.spawn(work).await.unwrap();
    assert_eq!(*writes.lock().unwrap(), [7]);
}

#[gpui::test]
fn dropping_final_work_completes_receipts_with_a_typed_interruption(cx: &mut TestAppContext) {
    let writes = Arc::default();
    let entity = cx.new(|_| Persisted::new(0, recording(&writes), Publication::Optimistic, 4));
    let (receipt, work) = cx.update(|cx| {
        entity.update(cx, |state, cx| {
            let receipt = state.update(|v| *v = 1, cx).unwrap();
            (receipt, state.capture_shutdown())
        })
    });
    drop(work);
    assert!(matches!(
        futures::executor::block_on(receipt.wait()),
        Err(StoreError::CompletionDropped)
    ));
    assert!(writes.lock().unwrap().is_empty());
}

#[gpui::test]
async fn latest_desired_saturates_without_consuming_receipt_queue_capacity(
    cx: &mut TestAppContext,
) {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let (release, wait) = oneshot::channel();
    let wait = Arc::new(Mutex::new(Some(wait)));
    let writer: SnapshotWriter<i32> = {
        let writes = writes.clone();
        Arc::new(move |value| {
            let wait = wait.lock().unwrap().take();
            let writes = writes.clone();
            Box::pin(async move {
                if let Some(wait) = wait {
                    wait.await.unwrap();
                }
                writes.lock().unwrap().push(value);
                Ok(())
            })
        })
    };
    let entity = cx.new(|_| Persisted::new(0, writer, Publication::Optimistic, 1));
    cx.update(|cx| entity.update(cx, |state, cx| state.update_latest(|v| *v = 1, cx).unwrap()));
    cx.run_until_parked();
    let receipt =
        cx.update(|cx| entity.update(cx, |state, cx| state.update(|v| *v = 2, cx).unwrap()));
    cx.update(|cx| {
        entity.update(cx, |state, cx| {
            assert!(matches!(
                state.update(|v| *v = 3, cx),
                Err(StoreError::Full)
            ));
            for value in 3..=10001 {
                state.update_latest(|v| *v = value, cx).unwrap();
            }
            assert_eq!(*state.visible(), 10001);
        })
    });
    release.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(receipt.wait().await.unwrap(), 10001);
    assert_eq!(*writes.lock().unwrap(), [1, 10001]);
    let work = cx.update(|cx| {
        entity.update(cx, |state, cx| {
            let work = state.capture_shutdown();
            assert!(matches!(
                state.update_latest(|v| *v = 7, cx),
                Err(StoreError::Closing)
            ));
            work
        })
    });
    cx.background_executor.spawn(work).await.unwrap();
}

#[gpui::test]
async fn rejected_checked_edits_discard_partial_changes_without_disk_work(cx: &mut TestAppContext) {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let observed = writes.clone();
    let writer: SnapshotWriter<i32> = Arc::new(move |value| {
        let observed = observed.clone();
        Box::pin(async move {
            observed.lock().unwrap().push(value);
            Ok(())
        })
    });
    let entity = cx.new(|_| Persisted::new(0, writer, Publication::Durable, 4));
    let rejected = entity.update(cx, |state, cx| {
        state
            .update_checked(
                |value| {
                    *value = 99;
                    Err(StoreError::edit(std::io::Error::from(
                        std::io::ErrorKind::InvalidInput,
                    )))
                },
                cx,
            )
            .unwrap()
    });
    let error = rejected.wait().await.unwrap_err();
    assert!(matches!(error, StoreError::Edit(_)));
    let original = error.source().unwrap().source().unwrap();
    assert_eq!(
        original.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert!(writes.lock().unwrap().is_empty());
    let accepted = entity.update(cx, |state, cx| {
        state.update(|value| *value += 1, cx).unwrap()
    });
    accepted.wait().await.unwrap();
    assert_eq!(entity.read_with(cx, |state, _| *state.visible()), 1);
    assert_eq!(*writes.lock().unwrap(), [1]);
}

#[gpui::test]
async fn shutdown_rejected_checked_edits_never_write_the_original_snapshot(
    cx: &mut TestAppContext,
) {
    let calls = Arc::new(Mutex::new(0));
    let observed = calls.clone();
    let writer: SnapshotWriter<i32> = Arc::new(move |_| {
        let observed = observed.clone();
        Box::pin(async move {
            *observed.lock().unwrap() += 1;
            Ok(())
        })
    });
    let entity = cx.new(|_| Persisted::new(0, writer, Publication::Durable, 4));
    let rejected = entity.update(cx, |state, cx| {
        state
            .update_checked(
                |value| {
                    *value = 99;
                    Err(StoreError::edit(std::io::Error::from(
                        std::io::ErrorKind::InvalidInput,
                    )))
                },
                cx,
            )
            .unwrap()
    });
    let work = entity.update(cx, |state, _| state.capture_shutdown());
    cx.background_executor.spawn(work).await.unwrap();
    let error = rejected.wait().await.unwrap_err();
    assert!(matches!(error, StoreError::Edit(_)));
    let original = error.source().unwrap().source().unwrap();
    assert_eq!(
        original.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(*calls.lock().unwrap(), 0);
}

#[gpui::test]
async fn captured_backend_failure_retains_concrete_and_native_causes(cx: &mut TestAppContext) {
    let writer: SnapshotWriter<i32> = Arc::new(|_| {
        Box::pin(async {
            Err(StoreError::backend(nocterm_core::PersistError::Io {
                path: "captured.toml".into(),
                source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            }))
        })
    });
    let entity = cx.new(|_| Persisted::new(0, writer, Publication::Durable, 4));
    let (receipt, work) = entity.update(cx, |state, cx| {
        let receipt = state.update(|value| *value = 1, cx).unwrap();
        (receipt, state.capture_shutdown())
    });
    drop(entity);
    let error = cx.background_executor.spawn(work).await.unwrap_err();
    let receipt_error = receipt.wait().await.unwrap_err();
    let backend = error.source().unwrap().source().unwrap();
    assert!(
        backend
            .downcast_ref::<nocterm_core::PersistError>()
            .is_some()
    );
    let receipt_backend = receipt_error.source().unwrap().source().unwrap();
    assert!(std::ptr::eq(backend, receipt_backend));
    assert_eq!(
        backend
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
}
