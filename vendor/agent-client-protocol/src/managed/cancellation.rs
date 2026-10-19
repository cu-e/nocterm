//! Original cancellation and fallibly prepaid async waiter registrations.
use super::{Budget, Buffer, ManagedError, Shared, Weak};
use std::{
    future::Future,
    pin::Pin,
    sync::Mutex,
    task::{Context, Poll, Waker},
};

struct WaitState {
    cancelled: bool,
    waker: Option<Waker>,
}
struct Waiter {
    state: Mutex<WaitState>,
}
struct State<B: Budget> {
    cancelled: bool,
    waiters: Buffer<Weak<Waiter, B>, B>,
}
struct Body<B: Budget> {
    state: Mutex<State<B>>,
    budget: B,
}
/// Original peer cancellation identity. Each async waiter is an independently
/// prepaid actual shared node; no hidden future/oneshot/shared-waiter allocation.
pub struct Cancellation<B: Budget> {
    body: Shared<Body<B>, B>,
}
impl<B: Budget> Clone for Cancellation<B> {
    fn clone(&self) -> Self {
        Self {
            body: self.body.clone(),
        }
    }
}
impl<B: Budget> Cancellation<B> {
    pub(super) fn new(budget: &B) -> Result<Self, ManagedError<B::Error>> {
        let body = Body {
            state: Mutex::new(State {
                cancelled: false,
                waiters: Buffer::new(budget.clone()),
            }),
            budget: budget.clone(),
        };
        Ok(Self {
            body: Shared::new(body, budget)?,
        })
    }
    /// Whether the original peer has cancelled this original request.
    pub fn is_cancelled(&self) -> bool {
        self.body
            .get()
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .cancelled
    }
    /// Register an original async waiter before allocating its physical state.
    /// Native refusal preserves the marker and all previously registered waiters.
    pub fn cancelled(&self) -> Result<Cancelled<B>, ManagedError<B::Error>> {
        let mut state = self
            .body
            .get()
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.cancelled {
            return Ok(Cancelled {
                marker: self.clone(),
                waiter: None,
            });
        }
        state.waiters.reserve_amortized(1)?;
        let waiter = Shared::new(
            Waiter {
                state: Mutex::new(WaitState {
                    cancelled: false,
                    waker: None,
                }),
            },
            &self.body.get().budget,
        )?;
        state
            .waiters
            .push(waiter.downgrade())
            .map_err(|(error, _)| error)?;
        Ok(Cancelled {
            marker: self.clone(),
            waiter: Some(waiter),
        })
    }
    pub(super) fn cancel(&self) {
        let waiters = {
            let mut state = self
                .body
                .get()
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state.cancelled {
                return;
            }
            state.cancelled = true;
            std::mem::replace(
                &mut state.waiters,
                Buffer::new(self.body.get().budget.clone()),
            )
        };
        for waiter in waiters.into_values() {
            if let Some(waiter) = waiter.upgrade() {
                let wake = {
                    let mut state = waiter
                        .get()
                        .state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    state.cancelled = true;
                    state.waker.take()
                };
                if let Some(wake) = wake {
                    wake.wake();
                }
            }
        }
    }
}
/// One actual prepaid cancellation future, returned without boxing.
pub struct Cancelled<B: Budget> {
    marker: Cancellation<B>,
    waiter: Option<Shared<Waiter, B>>,
}
impl<B: Budget> Unpin for Cancelled<B> {}
impl<B: Budget> Future for Cancelled<B> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let Some(waiter) = &self.get_mut().waiter else {
            return Poll::Ready(());
        };
        // Executor-owned wake handles are cloned outside the SDK lock. The
        // native host must retain/pay its original task, not fabricate SDK credit.
        let wake = cx.waker().clone();
        let previous = {
            let mut state = waiter
                .get()
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state.cancelled {
                return Poll::Ready(());
            }
            state.waker.replace(wake)
        };
        drop(previous);
        Poll::Pending
    }
}
impl<B: Budget> Drop for Cancelled<B> {
    fn drop(&mut self) {
        let Some(waiter) = self.waiter.take() else {
            return;
        };
        let original = waiter.downgrade();
        let removed = {
            let mut state = self
                .marker
                .body
                .get()
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let index = state
                .waiters
                .as_slice()
                .iter()
                .position(|weak| weak.same_allocation(&original));
            index.map(|index| state.waiters.remove(index))
        };
        drop(removed);
        drop(original);
        drop(waiter);
    }
}
