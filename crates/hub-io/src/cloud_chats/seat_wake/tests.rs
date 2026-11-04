use std::sync::atomic::{AtomicUsize, Ordering};

use futures::FutureExt as _;
use nocterm_ai::conversation::ConversationAction;
use nocterm_cloud_model::projection::testing::{ProjectionFixture, ScriptedConversationSource};

use super::*;

/// Counts the wakes and answers each with `result`.
struct CountingSeat {
    wakes: AtomicUsize,
    result: Result<(), ProjectionError>,
}

impl SeatWake for CountingSeat {
    fn wake(&self) -> BoxFuture<'static, Result<(), ProjectionError>> {
        self.wakes.fetch_add(1, Ordering::SeqCst);
        let result = self.result.clone();
        async move { result }.boxed()
    }
}

fn lease() -> AllocationLease {
    AllocationLease::new(64 * 1024, 1, ())
}

fn waking(
    result: Result<(), ProjectionError>,
) -> (
    Arc<ScriptedConversationSource>,
    Arc<CountingSeat>,
    Arc<dyn CapturedConversationSource>,
) {
    let inner = Arc::new(ScriptedConversationSource::new(
        ProjectionFixture::new().binding().clone(),
    ));
    let seat = Arc::new(CountingSeat {
        wakes: AtomicUsize::new(0),
        result,
    });
    let source = WakingSource::wrap(inner.clone(), seat.clone());
    (inner, seat, source)
}

/// Prepares and stages `action` through `source`, as the chat does before it delivers.
fn staged(source: &Arc<dyn CapturedConversationSource>, action: ConversationAction) -> CommandId {
    let coordinates = ConversationCoordinates {
        binding: source.binding().clone(),
        run: None,
        control: Default::default(),
    };
    let command = source.prepare(coordinates, action.into()).unwrap();
    let id = command.id;
    futures::executor::block_on(source.stage(Arc::new(Allocated::new(command, lease())), lease()))
        .unwrap();
    id
}

#[test]
fn a_prompt_wakes_the_seat_before_the_hub_stores_it() {
    let (inner, seat, source) = waking(Ok(()));
    let id = staged(&source, ConversationAction::Prompt("hello".into()));
    assert_eq!(seat.wakes.load(Ordering::SeqCst), 0, "staging is local");
    let delivered = futures::executor::block_on(source.deliver(id, lease()));
    assert_eq!(delivered, Ok(CommandDelivery::Delivered));
    assert_eq!(seat.wakes.load(Ordering::SeqCst), 1);
    assert_eq!(inner.effects().delivered, [id]);
    // A retry asks the runner for the receipt, which a sleeping runner never answers.
    futures::executor::block_on(source.query(id, lease())).unwrap();
    assert_eq!(seat.wakes.load(Ordering::SeqCst), 2);
}

#[test]
fn a_seat_that_cannot_wake_keeps_the_prompt_staged_for_a_retry() {
    let (inner, seat, source) = waking(Err(ProjectionError::Unavailable));
    let id = staged(&source, ConversationAction::Prompt("hello".into()));
    let delivered = futures::executor::block_on(source.deliver(id, lease()));
    assert_eq!(delivered, Err(ProjectionError::Unavailable));
    assert_eq!(seat.wakes.load(Ordering::SeqCst), 1);
    assert!(
        inner.effects().delivered.is_empty(),
        "nothing reached the hub"
    );
    assert_eq!(inner.effects().staged, [id]);
}

#[test]
fn a_stop_is_delivered_without_waking_the_seat() {
    let (inner, seat, source) = waking(Ok(()));
    let id = staged(&source, ConversationAction::Stop);
    futures::executor::block_on(source.deliver(id, lease())).unwrap();
    assert_eq!(seat.wakes.load(Ordering::SeqCst), 0);
    assert_eq!(inner.effects().delivered, [id]);
}
