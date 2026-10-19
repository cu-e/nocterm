//! Original FIFO positions, bounded backpressure and cancelled native sends.
use super::fixture;
use crate::managed::channel;
use futures::{
    executor::block_on,
    task::{ArcWake, waker},
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};
struct Wake(AtomicUsize);
impl ArcWake for Wake {
    fn wake_by_ref(this: &Arc<Self>) {
        this.0.fetch_add(1, Ordering::SeqCst);
    }
}
#[test]
fn all_original_event_positions_remain_fifo_and_pending_send_cancellation_is_reusable() {
    let fixture = fixture();
    let (sender, mut receiver) = channel::bounded(fixture.budget.clone(), 256).unwrap();
    for index in 0..256 {
        block_on(sender.send(index)).unwrap();
    }
    let wake = Arc::new(Wake(AtomicUsize::new(0)));
    let waker = waker(wake.clone());
    let mut cx = Context::from_waker(&waker);
    let mut cancelled = sender.send(256);
    let mut next = sender.send(257);
    assert!(Pin::new(&mut cancelled).poll(&mut cx).is_pending());
    assert!(Pin::new(&mut next).poll(&mut cx).is_pending());
    assert_eq!(receiver.try_recv(), Some(0));
    assert!(wake.0.load(Ordering::SeqCst) > 0);
    drop(cancelled);
    assert!(matches!(
        Pin::new(&mut next).poll(&mut cx),
        Poll::Ready(Ok(()))
    ));
    for index in 1..256 {
        assert_eq!(receiver.try_recv(), Some(index));
    }
    assert_eq!(receiver.try_recv(), Some(257));
    assert!(receiver.try_recv().is_none());
    let mut reused = sender.send(258);
    assert!(matches!(
        Pin::new(&mut reused).poll(&mut cx),
        Poll::Ready(Ok(()))
    ));
    sender.close();
    assert_eq!(receiver.try_recv(), Some(258));
    assert!(matches!(receiver.poll_recv(&mut cx), Poll::Ready(None)));
}
#[test]
fn closed_source_preserves_already_paid_channel_and_abandonment_refuses_late_send() {
    let fixture = fixture();
    let (sender, mut receiver) = channel::bounded(fixture.budget.clone(), 256).unwrap();
    block_on(sender.send(7)).unwrap();
    let before = fixture.budget.ledger().live_requested_bytes();
    drop(fixture._lease);
    assert_eq!(receiver.try_recv(), Some(7));
    assert!(fixture.budget.ledger().live_requested_bytes() >= before);
    // Existing queue slots need no new reservation after source closes.
    block_on(sender.send(8)).unwrap();
    assert_eq!(receiver.try_recv(), Some(8));
    drop(receiver);
    assert!(matches!(
        block_on(sender.send(9)),
        Err(channel::SendError::Closed(9))
    ));
    drop(sender);
}

#[test]
fn final_exit_preserves_original_full_queue_eviction_of_exactly_one_oldest_position() {
    let fixture = fixture();
    let (sender, mut receiver) = channel::bounded(fixture.budget.clone(), 256).unwrap();
    for index in 0..256 {
        block_on(sender.send(index)).unwrap();
    }
    assert_eq!(sender.publish_final(256).unwrap(), Some(0));
    sender.close();
    for index in 1..=256 {
        assert_eq!(receiver.try_recv(), Some(index));
    }
    assert!(receiver.try_recv().is_none());
}

#[test]
fn nonblocking_send_refuses_full_and_closed_with_the_same_original_payload() {
    let fixture = fixture();
    let (sender, mut receiver) = channel::bounded(fixture.budget.clone(), 1).unwrap();
    sender.try_send(Box::new(1usize)).unwrap();
    let payload = Box::new(2usize);
    let original = &*payload as *const usize;
    let Err(channel::TrySendError::Full(payload)) = sender.try_send(payload) else {
        panic!("full")
    };
    assert_eq!(&*payload as *const usize, original);
    assert_eq!(*receiver.try_recv().unwrap(), 1);
    sender.close();
    let Err(channel::TrySendError::Closed(payload)) = sender.try_send(payload) else {
        panic!("closed")
    };
    assert_eq!(&*payload as *const usize, original);
}
