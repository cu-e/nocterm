//! Foreground acknowledgement is the exact original fence, never enqueue.
use super::fixture;
use crate::managed::events;
use nocterm_ai::connection::ManagedAgentEvent;
use std::task::{Context, Poll};
#[test]
fn real_event_fence_waits_for_consumption_and_detects_original_abandonment() {
    let fixture = fixture();
    let (sender, mut receiver) = events::channel(&fixture.budget).unwrap();
    let (barrier, mut completion) = events::fence(&fixture.budget).unwrap();
    sender
        .try_send(ManagedAgentEvent::Barrier(barrier))
        .map_err(|_| ())
        .unwrap();
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    assert!(completion.poll_recv(&mut cx).is_pending());
    let Poll::Ready(Some(ManagedAgentEvent::Barrier(barrier))) = receiver.poll_event(&mut cx)
    else {
        panic!("original FIFO event")
    };
    assert!(completion.poll_recv(&mut cx).is_pending());
    barrier.acknowledge();
    assert_eq!(completion.poll_recv(&mut cx), Poll::Ready(Some(())));
    let (barrier, mut abandoned) = events::fence(&fixture.budget).unwrap();
    sender
        .try_send(ManagedAgentEvent::Barrier(barrier))
        .map_err(|_| ())
        .unwrap();
    drop(receiver);
    assert_eq!(abandoned.poll_recv(&mut cx), Poll::Ready(None));
}
