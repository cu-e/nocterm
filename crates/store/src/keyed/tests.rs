use super::*;
use futures::channel::oneshot;
use gpui::{AppContext as _, TestAppContext};
use std::{cell::RefCell, rc::Rc};

#[gpui::test]
async fn one_hundred_dirty_keys_survive_a_full_ready_queue_and_a_hot_key(cx: &mut TestAppContext) {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let (release, wait) = oneshot::channel();
    let wait = Arc::new(Mutex::new(Some(wait)));
    let log = writes.clone();
    let writer: KeyedWriter<usize, usize> = Arc::new(move |key, value| {
        let wait = wait.lock().unwrap().take();
        let log = log.clone();
        Box::pin(async move {
            if let Some(wait) = wait {
                wait.await.unwrap()
            }
            log.lock().unwrap().push((key, value));
            Ok(())
        })
    });
    let entity = cx.new(|_| KeyedPersisted::new(writer, NonZeroUsize::new(32).unwrap()));
    entity.update(cx, |state, cx| state.update_latest(0, 0, 0, cx).unwrap());
    cx.run_until_parked();
    cx.update(|cx| {
        entity.update(cx, |state, cx| {
            for key in 1..=100 {
                state.update_latest(key, key, key, cx).unwrap();
            }
            for value in 101..=200 {
                state.update_latest(0, value, value, cx).unwrap();
            }
        });
    });
    release.send(()).unwrap();
    cx.run_until_parked();
    let log = writes.lock().unwrap();
    assert_eq!(log.len(), 102);
    assert_eq!(log[0], (0, 0));
    assert_eq!(log.last(), Some(&(0, 200)));
    assert_eq!(
        log[1..101],
        (1..=100).map(|key| (key, key)).collect::<Vec<_>>()
    );
    cx.update(|cx| {
        for key in 0..=100 {
            assert!(!entity.read(cx).is_dirty(&key));
        }
    });
}

#[gpui::test]
fn one_failure_does_not_stop_other_keys_or_busy_retry(cx: &mut TestAppContext) {
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let log = attempts.clone();
    let writer: KeyedWriter<usize, usize> = Arc::new(move |key, value| {
        log.lock().unwrap().push((key, value));
        Box::pin(async move {
            if value == 1 {
                Err(StoreError::PendingWrites)
            } else {
                Ok(())
            }
        })
    });
    let acknowledgements = Rc::new(RefCell::new(Vec::new()));
    let hook = acknowledgements.clone();
    let entity = cx.new(|_| {
        KeyedPersisted::new(writer, NonZeroUsize::new(2).unwrap()).with_completion_hook(
            move |stamp, result, _| {
                hook.borrow_mut()
                    .push((*stamp.key(), *stamp.tag(), result.is_ok()));
                KeyedRetention::Retain
            },
        )
    });
    entity.update(cx, |state, cx| {
        state.update_latest(1, 1, 11, cx).unwrap();
        state.update_latest(2, 2, 22, cx).unwrap();
    });
    cx.run_until_parked();
    assert_eq!(*attempts.lock().unwrap(), [(1, 1), (2, 2)]);
    assert_eq!(*acknowledgements.borrow(), [(1, 11, false), (2, 22, true)]);
    cx.update(|cx| {
        assert!(entity.read(cx).error(&1).is_some());
        assert!(entity.read(cx).is_dirty(&1));
        assert!(!entity.read(cx).is_dirty(&2));
    });
    entity.update(cx, |state, cx| state.update_latest(1, 3, 33, cx).unwrap());
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!entity.read(cx).is_dirty(&1));
        assert!(entity.read(cx).error(&1).is_none());
        assert_eq!(*entity.read(cx).last_success(&1).unwrap().tag(), 33);
    });
}

#[gpui::test]
async fn frozen_ack_preserves_physical_prefix_and_latest_delete_orders_after_it(
    cx: &mut TestAppContext,
) {
    let (release, wait) = oneshot::channel();
    let wait = Arc::new(Mutex::new(Some(wait)));
    let writes = Arc::new(Mutex::new(Vec::new()));
    let log = writes.clone();
    let writer: KeyedWriter<usize, Option<usize>> = Arc::new(move |key, value| {
        let wait = wait.lock().unwrap().take();
        let log = log.clone();
        Box::pin(async move {
            if let Some(wait) = wait {
                wait.await.unwrap()
            }
            log.lock().unwrap().push((key, value));
            Ok(())
        })
    });
    let entity = cx.new(|_| KeyedPersisted::new(writer, NonZeroUsize::new(1).unwrap()));
    entity.update(cx, |state, cx| {
        state.update_latest(1, Some(1), 11, cx).unwrap()
    });
    cx.run_until_parked();
    entity.update(cx, |state, cx| {
        state.update_latest(1, None, 12, cx).unwrap();
        state.update_latest(2, Some(2), 22, cx).unwrap();
    });
    let captured = entity.update(cx, |state, _| state.capture());
    drop(entity);
    let final_work = cx.background_executor.spawn(captured.run());
    cx.run_until_parked();
    assert!(writes.lock().unwrap().is_empty());
    release.send(()).unwrap();
    let outcomes = final_work.await;
    assert_eq!(
        *writes.lock().unwrap(),
        [(1, Some(1)), (1, None), (2, Some(2))]
    );
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(|outcome| outcome.result().is_ok()));
    assert_eq!(*outcomes[0].stamp().tag(), 12);
}

#[gpui::test]
async fn successful_active_generation_is_not_rewritten_when_capture_cancels_gui_ack(
    cx: &mut TestAppContext,
) {
    let (release, wait) = oneshot::channel();
    let wait = Arc::new(Mutex::new(Some(wait)));
    let writes = Arc::new(Mutex::new(0));
    let log = writes.clone();
    let writer: KeyedWriter<usize, usize> = Arc::new(move |_, _| {
        let wait = wait.lock().unwrap().take();
        let log = log.clone();
        Box::pin(async move {
            if let Some(wait) = wait {
                wait.await.unwrap()
            }
            *log.lock().unwrap() += 1;
            Ok(())
        })
    });
    let entity = cx.new(|_| KeyedPersisted::new(writer, NonZeroUsize::new(1).unwrap()));
    let generation = entity.update(cx, |state, cx| state.update_latest(1, 1, 11, cx).unwrap());
    cx.run_until_parked();
    let captured = entity.update(cx, |state, _| state.capture());
    drop(entity);
    let task = cx.background_executor.spawn(captured.run());
    release.send(()).unwrap();
    let outcomes = task.await;
    assert_eq!(*writes.lock().unwrap(), 1);
    assert_eq!(outcomes[0].stamp().generation(), generation);
    assert!(outcomes[0].result().is_ok());
}

#[gpui::test]
fn an_explicit_retry_reuses_dirty_intent_without_a_new_generation(cx: &mut TestAppContext) {
    let attempts = Arc::new(Mutex::new(0usize));
    let log = attempts.clone();
    let writer: KeyedWriter<usize, usize> = Arc::new(move |_, _| {
        let attempt = {
            let mut attempts = log.lock().unwrap();
            *attempts += 1;
            *attempts
        };
        Box::pin(async move {
            if attempt == 1 {
                Err(StoreError::PendingWrites)
            } else {
                Ok(())
            }
        })
    });
    let entity = cx.new(|_| KeyedPersisted::new(writer, NonZeroUsize::new(1).unwrap()));
    let generation = entity.update(cx, |state, cx| state.update_latest(1, 1, 11, cx).unwrap());
    cx.run_until_parked();
    assert_eq!(*attempts.lock().unwrap(), 1);
    entity.update(cx, |state, cx| state.retry(&1, cx).unwrap());
    cx.run_until_parked();
    assert_eq!(*attempts.lock().unwrap(), 2);
    cx.update(|cx| {
        assert!(!entity.read(cx).is_dirty(&1));
        assert_eq!(
            entity.read(cx).last_success(&1).unwrap().generation(),
            generation
        );
    });
}

#[gpui::test]
async fn a_failed_final_key_keeps_the_exact_successful_physical_prefix(cx: &mut TestAppContext) {
    let (release, wait) = oneshot::channel();
    let wait = Arc::new(Mutex::new(Some(wait)));
    let writes = Arc::new(Mutex::new(Vec::new()));
    let log = writes.clone();
    let writer: KeyedWriter<usize, usize> = Arc::new(move |key, value| {
        let wait = wait.lock().unwrap().take();
        let log = log.clone();
        Box::pin(async move {
            if let Some(wait) = wait {
                wait.await.unwrap();
            }
            if value == 2 {
                return Err(StoreError::PendingWrites);
            }
            log.lock().unwrap().push((key, value));
            Ok(())
        })
    });
    let entity = cx.new(|_| KeyedPersisted::new(writer, NonZeroUsize::new(1).unwrap()));
    let first = entity.update(cx, |state, cx| state.update_latest(1, 1, 11, cx).unwrap());
    cx.run_until_parked();
    let final_generation = entity.update(cx, |state, cx| {
        let final_generation = state.update_latest(1, 2, 12, cx).unwrap();
        state.update_latest(2, 3, 22, cx).unwrap();
        final_generation
    });
    let captured = entity.update(cx, |state, _| state.capture());
    drop(entity);
    let task = cx.background_executor.spawn(captured.run());
    release.send(()).unwrap();
    let outcomes = task.await;
    assert_eq!(*writes.lock().unwrap(), [(1, 1), (2, 3)]);
    assert!(outcomes[0].result().is_err());
    assert_eq!(outcomes[0].stamp().generation(), final_generation);
    let prefix = outcomes[0].committed().unwrap();
    assert_eq!(prefix.generation(), first);
    assert_eq!(*prefix.tag(), 11);
    assert!(outcomes[1].result().is_ok());
    assert_eq!(*outcomes[1].committed().unwrap().tag(), 22);
}

#[gpui::test]
fn closed_clean_keys_release_payloads_but_failed_dirty_keys_survive_until_retry(
    cx: &mut TestAppContext,
) {
    let failing = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let failure = failing.clone();
    let writer: KeyedWriter<usize, Arc<Vec<u8>>> = Arc::new(move |_, _| {
        let fail = failure.load(std::sync::atomic::Ordering::SeqCst);
        Box::pin(async move {
            if fail {
                Err(StoreError::PendingWrites)
            } else {
                Ok(())
            }
        })
    });
    let entity = cx.new(|_| {
        KeyedPersisted::new(writer, NonZeroUsize::new(1).unwrap())
            .with_completion_hook(|_, _, _| KeyedRetention::ForgetClean)
    });
    let payload = Arc::new(vec![1; 1024]);
    let retained = Arc::downgrade(&payload);
    entity.update(cx, |state, cx| {
        state.update_latest(1, payload, 11, cx).unwrap()
    });
    cx.run_until_parked();
    assert!(
        retained.upgrade().is_some(),
        "failed closed payload was lost"
    );
    assert!(!entity.update(cx, |state, _| state.forget_clean(&1)));
    failing.store(false, std::sync::atomic::Ordering::SeqCst);
    entity.update(cx, |state, cx| state.retry(&1, cx).unwrap());
    cx.run_until_parked();
    assert!(
        retained.upgrade().is_none(),
        "successful closed payload was retained"
    );
    cx.update(|cx| {
        assert!(!entity.read(cx).is_dirty(&1));
        assert!(entity.read(cx).last_success(&1).is_none());
        assert!(entity.read(cx).error(&1).is_none());
    });
}

#[gpui::test]
fn retained_value_observation_includes_active_and_coalesced_payloads_until_publication(
    cx: &mut TestAppContext,
) {
    let (release, wait) = oneshot::channel();
    let wait = Arc::new(Mutex::new(Some(wait)));
    let writer: KeyedWriter<usize, Arc<usize>> = Arc::new(move |_, _| {
        let wait = wait.lock().unwrap().take();
        Box::pin(async move {
            if let Some(wait) = wait {
                wait.await.unwrap();
            }
            Ok(())
        })
    });
    let entity = cx.new(|_| KeyedPersisted::new(writer, NonZeroUsize::new(1).unwrap()));
    entity.update(cx, |state, cx| {
        state.update_latest(1, Arc::new(10), (), cx).unwrap()
    });
    cx.run_until_parked();
    entity.update(cx, |state, cx| {
        state.update_latest(1, Arc::new(20), (), cx).unwrap();
        state.update_latest(2, Arc::new(30), (), cx).unwrap();
    });
    cx.update(|cx| {
        let mut retained = Vec::new();
        entity
            .read(cx)
            .visit_retained_values(|value| retained.push(**value));
        retained.sort_unstable();
        assert_eq!(retained, [10, 20, 30]);
    });
    release.send(()).unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let mut retained = Vec::new();
        entity
            .read(cx)
            .visit_retained_values(|value| retained.push(**value));
        assert!(retained.is_empty());
    });
}
