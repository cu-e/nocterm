use super::*;
use futures::{StreamExt as _, future};
use nocterm_cloud_conversation_model::{ConversationDelta, Freshness, RowRole, testing::row};
use std::time::Duration;

type Reader = BoxStream<'static, ConversationUpdate>;
fn never() -> Ended {
    future::pending().boxed().shared()
}
fn update(index: i64) -> ConversationUpdate {
    ConversationUpdate::Freshness(Freshness::Current {
        witnessed_at_ms: index,
    })
}
async fn subscribed(limit: usize) -> (Subscriptions, Reader, Arc<Notify>) {
    subscribed_with_budget(Arc::new(UpdateBudget::new(limit))).await
}
async fn subscribed_with_budget(budget: Arc<UpdateBudget>) -> (Subscriptions, Reader, Arc<Notify>) {
    let changed = Arc::new(Notify::new());
    let mut subscriptions = Subscriptions::new(budget);
    let (subscriber, mut reader) = channel(changed.clone());
    subscriptions
        .subscribe(subscriber, [update(0), update(1)], never())
        .await
        .unwrap();
    reader.next().await.unwrap();
    reader.next().await.unwrap();
    (subscriptions, reader, changed)
}
fn index(update: Option<ConversationUpdate>) -> i64 {
    match update.unwrap() {
        ConversationUpdate::Freshness(Freshness::Current { witnessed_at_ms }) => witnessed_at_ms,
        other => panic!("expected freshness: {other:?}"),
    }
}

#[tokio::test]
async fn a_slow_reader_applies_backpressure_without_losing_the_final_update() {
    let (mut subscriptions, mut reader, _) = subscribed(RETAINED_BYTES).await;
    for value in 0..PENDING_UPDATES {
        subscriptions
            .publish(update(value as i64), never())
            .await
            .unwrap();
    }
    let sending = subscriptions.publish(update(8), never());
    tokio::pin!(sending);
    assert!(futures::poll!(&mut sending).is_pending());
    assert_eq!(index(reader.next().await), 0);
    sending.await.unwrap();
    for value in 1..=8 {
        assert_eq!(index(reader.next().await), value);
    }
}

#[tokio::test]
async fn byte_capacity_blocks_before_message_slots_and_is_released_on_delivery() {
    let limit = update_size::retained(&update(0)) * 2;
    let (mut subscriptions, mut reader, _) = subscribed(limit).await;
    subscriptions.publish(update(10), never()).await.unwrap();
    subscriptions.publish(update(11), never()).await.unwrap();
    let sending = subscriptions.publish(update(12), never());
    tokio::pin!(sending);
    assert!(futures::poll!(&mut sending).is_pending());
    assert_eq!(index(reader.next().await), 10);
    sending.await.unwrap();
    assert_eq!(index(reader.next().await), 11);
    assert_eq!(index(reader.next().await), 12);
}

#[tokio::test]
async fn dropping_the_last_reader_wakes_idle_flow_without_a_network_event() {
    let (mut subscriptions, reader, changed) = subscribed(RETAINED_BYTES).await;
    let idle = changed.notified();
    tokio::pin!(idle);
    idle.as_mut().enable();
    drop(reader);
    tokio::time::timeout(Duration::from_secs(1), idle)
        .await
        .unwrap();
    assert!(subscriptions.is_empty());
}

#[tokio::test]
async fn account_end_interrupts_a_reader_that_stopped_consuming() {
    let (mut subscriptions, _reader, _) = subscribed(RETAINED_BYTES).await;
    for value in 0..PENDING_UPDATES {
        subscriptions
            .publish(update(value as i64), never())
            .await
            .unwrap();
    }
    let (end, ended) = futures::channel::oneshot::channel::<()>();
    let sending = subscriptions.publish(update(99), ended.map(drop).boxed().shared());
    tokio::pin!(sending);
    assert!(futures::poll!(&mut sending).is_pending());
    end.send(()).unwrap();
    assert!(matches!(sending.await, Err(CloudConversationError::Stale)));
}

#[tokio::test]
async fn closing_a_reader_releases_a_byte_wait_without_other_consumers_progressing() {
    let limit = update_size::retained(&update(0)) * 2;
    let (mut subscriptions, reader, _) = subscribed(limit).await;
    subscriptions.publish(update(1), never()).await.unwrap();
    subscriptions.publish(update(2), never()).await.unwrap();
    let sending = subscriptions.publish(update(3), never());
    tokio::pin!(sending);
    assert!(futures::poll!(&mut sending).is_pending());
    drop(reader);
    tokio::time::timeout(Duration::from_secs(1), sending)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn an_oversized_delta_is_refused_and_the_prior_queue_is_preserved() {
    let (mut subscriptions, mut reader, _) = subscribed(4096).await;
    subscriptions.publish(update(1), never()).await.unwrap();
    let delta = ConversationUpdate::Transcript(ConversationDelta {
        changed_from: Some(0),
        rows: vec![row(RowRole::Assistant, None, &"x".repeat(4096))],
        ..Default::default()
    });
    assert!(matches!(
        subscriptions.publish(delta, never()).await,
        Err(CloudConversationError::Capacity)
    ));
    assert_eq!(index(reader.next().await), 1);
    subscriptions.publish(update(2), never()).await.unwrap();
    assert_eq!(index(reader.next().await), 2);
}

#[tokio::test]
async fn readers_share_one_payload_reservation_until_the_last_reader_consumes_it() {
    let limit = update_size::retained(&update(0)) * 2;
    let (mut subscriptions, mut first, changed) = subscribed(limit).await;
    let (subscriber, mut second) = channel(changed);
    subscriptions
        .subscribe(subscriber, [update(0), update(1)], never())
        .await
        .unwrap();
    second.next().await.unwrap();
    second.next().await.unwrap();
    subscriptions.publish(update(10), never()).await.unwrap();
    subscriptions.publish(update(11), never()).await.unwrap();
    assert_eq!(index(first.next().await), 10);
    assert_eq!(index(first.next().await), 11);
    let sending = subscriptions.publish(update(12), never());
    tokio::pin!(sending);
    assert!(futures::poll!(&mut sending).is_pending());
    assert_eq!(index(second.next().await), 10);
    sending.await.unwrap();
    assert_eq!(index(second.next().await), 11);
    assert_eq!(index(second.next().await), 12);
    assert_eq!(index(first.next().await), 12);
}

#[tokio::test]
async fn an_initial_snapshot_that_does_not_fit_reports_capacity_then_ends() {
    let changed = Arc::new(Notify::new());
    let mut subscriptions = Subscriptions::new(Arc::new(UpdateBudget::new(4096)));
    let (subscriber, mut reader) = channel(changed);
    let initial = ConversationUpdate::Transcript(ConversationDelta {
        changed_from: Some(0),
        rows: vec![row(RowRole::Assistant, None, &"x".repeat(4096))],
        ..Default::default()
    });
    subscriptions
        .subscribe(subscriber, [initial, update(0)], never())
        .await
        .unwrap();
    assert!(matches!(
        reader.next().await,
        Some(ConversationUpdate::Refused(
            CloudConversationError::Capacity
        ))
    ));
    assert!(reader.next().await.is_none());
    assert!(subscriptions.is_empty());
}

#[tokio::test]
async fn an_interrupted_wait_resumes_a_partially_sent_delta_without_duplicates() {
    let (mut subscriptions, mut first, changed) = subscribed(RETAINED_BYTES).await;
    let (subscriber, mut second) = channel(changed);
    subscriptions
        .subscribe(subscriber, [update(0), update(1)], never())
        .await
        .unwrap();
    second.next().await.unwrap();
    second.next().await.unwrap();
    for value in 0..PENDING_UPDATES {
        subscriptions
            .publish(update(value as i64), never())
            .await
            .unwrap();
        assert_eq!(index(first.next().await), value as i64);
    }
    subscriptions.begin_publish(update(99), never());
    // The first reader received the delta; the second reader is still full.
    // Dropping this wait models a control branch winning the actor's select.
    {
        let advancing = subscriptions.advance();
        tokio::pin!(advancing);
        assert!(futures::poll!(&mut advancing).is_pending());
    }
    assert_eq!(index(first.next().await), 99);
    assert_eq!(index(second.next().await), 0);
    subscriptions.advance().await.unwrap();
    assert!(first.next().now_or_never().is_none());
    for value in 1..PENDING_UPDATES {
        assert_eq!(index(second.next().await), value as i64);
    }
    assert_eq!(index(second.next().await), 99);
}

#[tokio::test]
async fn deferred_subscribers_are_bounded_before_their_snapshots_are_built() {
    let changed = Arc::new(Notify::new());
    let mut subscriptions = Subscriptions::new(Arc::new(UpdateBudget::default()));
    let mut readers = Vec::new();
    for _ in 0..WAITING_SUBSCRIBERS {
        let (subscriber, reader) = channel(changed.clone());
        subscriptions.defer(subscriber);
        readers.push(reader);
    }
    let (subscriber, mut refused) = channel(changed);
    subscriptions.defer(subscriber);
    assert!(matches!(
        refused.next().await,
        Some(ConversationUpdate::Refused(
            CloudConversationError::Capacity
        ))
    ));
    assert!(refused.next().await.is_none());
    for reader in &mut readers {
        assert!(reader.next().now_or_never().is_none());
    }
    let subscriber = subscriptions.waiting().unwrap();
    subscriptions.begin_subscribe(subscriber, [update(3), update(4)], never());
    subscriptions.advance().await.unwrap();
    assert_eq!(index(readers[0].next().await), 3);
    assert_eq!(index(readers[0].next().await), 4);
}

#[tokio::test]
async fn a_terminal_error_follows_the_already_accepted_delta() {
    let (mut subscriptions, mut reader, _) = subscribed(RETAINED_BYTES).await;
    for value in 0..PENDING_UPDATES {
        subscriptions
            .publish(update(value as i64), never())
            .await
            .unwrap();
    }
    subscriptions.begin_publish(update(99), never());
    {
        let advancing = subscriptions.advance();
        tokio::pin!(advancing);
        assert!(futures::poll!(&mut advancing).is_pending());
    }
    let finishing = subscriptions.finish(CloudConversationError::Capacity, never());
    tokio::pin!(finishing);
    assert!(futures::poll!(&mut finishing).is_pending());
    for value in 0..PENDING_UPDATES {
        assert_eq!(index(reader.next().await), value as i64);
    }
    finishing.await;
    assert_eq!(index(reader.next().await), 99);
    assert!(matches!(
        reader.next().await,
        Some(ConversationUpdate::Refused(
            CloudConversationError::Capacity
        ))
    ));
}

#[tokio::test]
async fn a_headless_flow_does_not_reject_an_undelivered_oversized_transcript() {
    let changed = Arc::new(Notify::new());
    let mut subscriptions = Subscriptions::new(Arc::new(UpdateBudget::new(4096)));
    let (subscriber, reader) = channel(changed);
    subscriptions
        .subscribe(subscriber, [update(0), update(1)], never())
        .await
        .unwrap();
    drop(reader);
    let delta = ConversationUpdate::Transcript(ConversationDelta {
        changed_from: Some(0),
        rows: vec![row(RowRole::Assistant, None, &"x".repeat(4096))],
        ..Default::default()
    });
    subscriptions.publish(delta, never()).await.unwrap();
}

#[tokio::test]
async fn closing_a_reader_finishes_its_byte_wait_even_when_the_actor_takes_the_drop_wake() {
    let budget = Arc::new(UpdateBudget::new(update_size::retained(&update(0)) * 2));
    let (mut other, mut other_reader, _) = subscribed_with_budget(budget.clone()).await;
    let (mut subscriptions, reader, changed) = subscribed_with_budget(budget).await;
    for value in [10, 11] {
        other.publish(update(value), never()).await.unwrap();
    }
    let actor_wake = changed.notified();
    tokio::pin!(actor_wake);
    actor_wake.as_mut().enable();
    subscriptions.begin_publish(update(99), never());
    {
        let sending = subscriptions.advance();
        tokio::pin!(sending);
        assert!(futures::poll!(&mut sending).is_pending());
        drop(reader);
        actor_wake.await;
        tokio::time::timeout(Duration::from_secs(1), sending)
            .await
            .expect("closing the target must not wait for an unrelated chat's byte budget")
            .unwrap();
    }
    assert!(subscriptions.is_empty());
    // The other reader held the full budget until the closed delivery finished.
    assert_eq!(index(other_reader.next().await), 10);
    assert_eq!(index(other_reader.next().await), 11);
}
