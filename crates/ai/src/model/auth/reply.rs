//! One answer on a prepaid, known-layout owner; no hidden channel allocation.
use super::PermissionOutcome;
use crate::{
    history::source::{NativeAllocation, SourceContext},
    model::json::{deferred::JsonDecodeError, lifetime::Shared},
};
use std::{
    future::Future,
    pin::Pin,
    sync::Mutex,
    task::{Context, Poll, Waker},
};

struct State {
    reply: Option<PermissionOutcome>,
    receiving: bool,
    waker: Option<Waker>,
}
struct Channel {
    state: Mutex<State>,
    _allocation: NativeAllocation,
}
#[derive(Debug)]
pub(super) struct Sender(Shared<Channel>);
#[derive(Debug)]
pub struct PermissionAnswer {
    channel: Shared<Channel>,
    finished: bool,
}
#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("permission answer is no longer available")]
pub struct PermissionAnswerClosed;

pub(super) fn channel(
    context: &SourceContext,
) -> Result<(Sender, PermissionAnswer), JsonDecodeError> {
    let bytes = crate::model::allocations::shared_inline_bytes::<Channel>();
    let layout = std::alloc::Layout::from_size_align(bytes, std::mem::align_of::<Channel>())
        .map_err(|_| JsonDecodeError::Arithmetic)?;
    let allocation = context.reserve_native(layout)?;
    let channel = Shared::new(Channel {
        state: Mutex::new(State {
            reply: None,
            receiving: true,
            waker: None,
        }),
        _allocation: allocation,
    });
    Ok((
        Sender(channel.clone()),
        PermissionAnswer {
            channel,
            finished: false,
        },
    ))
}
impl Sender {
    pub(super) fn send(self, reply: PermissionOutcome) {
        let (waker, discarded) = {
            let mut state = self
                .0
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let discarded = if state.receiving {
                state.reply.replace(reply)
            } else {
                Some(reply)
            };
            (state.waker.take(), discarded)
        };
        drop(discarded);
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}
impl Future for PermissionAnswer {
    type Output = Result<PermissionOutcome, PermissionAnswerClosed>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.finished {
            return Poll::Ready(Err(PermissionAnswerClosed));
        }
        let replacement = cx.waker().clone();
        let (reply, old) = {
            let mut state = self
                .channel
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let reply = state.reply.take();
            let old = if reply.is_some() {
                state.waker.take()
            } else {
                state.waker.replace(replacement)
            };
            (reply, old)
        };
        drop(old);
        if let Some(reply) = reply {
            self.finished = true;
            Poll::Ready(Ok(reply))
        } else {
            Poll::Pending
        }
    }
}
impl Drop for PermissionAnswer {
    fn drop(&mut self) {
        let (reply, waker) = {
            let mut state = self
                .channel
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.receiving = false;
            (state.reply.take(), state.waker.take())
        };
        drop(reply);
        drop(waker);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        history::source::SourceProducer,
        model::{
            auth::PermissionResponder,
            retention::{RetentionAuthority, document::DocumentRetention},
        },
    };
    fn source() -> (
        SourceProducer,
        crate::history::source::SourceLease,
        SourceContext,
    ) {
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
        let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
        let lease = producer.capture().unwrap();
        let context = lease.context().unwrap();
        (producer, lease, context)
    }
    #[test]
    fn managed_abandonment_and_receiver_cancel_refund_only_after_last_endpoint() {
        let (_producer, lease, context) = source();
        let baseline = context.ledger().live_requested_bytes();
        let (sender, receiver) = PermissionResponder::managed_channel(&context).unwrap();
        let paid = context.ledger().live_requested_bytes();
        assert_eq!(
            paid - baseline,
            crate::model::allocations::shared_inline_bytes::<Channel>()
        );
        drop(sender);
        assert_eq!(context.ledger().live_requested_bytes(), paid);
        drop(lease);
        assert_eq!(
            futures::executor::block_on(receiver).unwrap(),
            PermissionOutcome::Cancelled
        );
        assert_eq!(context.ledger().live_requested_bytes(), baseline);
        assert!(PermissionResponder::managed_channel(&context).is_err());
        assert_eq!(context.ledger().live_requested_bytes(), baseline);

        let (_producer, _lease, context) = source();
        let baseline = context.ledger().live_requested_bytes();
        let (sender, receiver) = PermissionResponder::managed_channel(&context).unwrap();
        drop(receiver);
        assert!(context.ledger().live_requested_bytes() > baseline);
        sender.respond(PermissionOutcome::Cancelled);
        assert_eq!(context.ledger().live_requested_bytes(), baseline);
    }
    #[test]
    fn managed_reply_wakes_waiter_and_delivers_original_selection_once() {
        use futures::task::{ArcWake, waker};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Wake(AtomicUsize);
        impl ArcWake for Wake {
            fn wake_by_ref(arc: &Arc<Self>) {
                arc.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let (_producer, _lease, context) = source();
        let wake = Arc::new(Wake(AtomicUsize::new(0)));
        let waker = waker(wake.clone());
        let mut cx = Context::from_waker(&waker);
        let (sender, mut receiver) = PermissionResponder::managed_channel(&context).unwrap();
        assert!(Pin::new(&mut receiver).poll(&mut cx).is_pending());
        let selected =
            PermissionOutcome::Selected(crate::model::ids::PermissionOptionId::new(" 001 "));
        sender.respond(selected.clone());
        assert_eq!(wake.0.load(Ordering::SeqCst), 1);
        assert_eq!(
            Pin::new(&mut receiver).poll(&mut cx).map(Result::unwrap),
            Poll::Ready(selected)
        );
        assert!(matches!(
            Pin::new(&mut receiver).poll(&mut cx),
            Poll::Ready(Err(PermissionAnswerClosed))
        ));
    }
}
