//! Poll admission under a caller's original authority, including completed work.
use std::future::{Future, poll_fn};
use std::task::Poll;

pub(crate) async fn during<T, E>(
    check: impl Fn() -> Result<(), E>,
    future: impl Future<Output = T>,
) -> Result<T, E> {
    let mut future = std::pin::pin!(future);
    poll_fn(|cx| {
        check()?;
        match future.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(value) => Poll::Ready(check().map(|()| value)),
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn invalid_authority_never_polls_the_operation() {
        let polled = Cell::new(false);
        let result =
            futures::executor::block_on(during(|| Err("stale"), async { polled.set(true) }));
        assert_eq!(result, Err("stale"));
        assert!(!polled.get());
    }

    #[test]
    fn resumed_work_rechecks_authority_before_polling_again() {
        let valid = Cell::new(true);
        let polls = Cell::new(0);
        let work = poll_fn(|cx| {
            polls.set(polls.get() + 1);
            valid.set(false);
            cx.waker().wake_by_ref();
            Poll::<()>::Pending
        });
        let result =
            futures::executor::block_on(during(|| valid.get().then_some(()).ok_or("stale"), work));
        assert_eq!(result, Err("stale"));
        assert_eq!(polls.get(), 1);
    }

    #[test]
    fn completion_rechecks_authority_before_delivering_the_result() {
        let valid = Cell::new(true);
        let result = futures::executor::block_on(during(
            || valid.get().then_some(()).ok_or("stale"),
            async {
                valid.set(false);
                7
            },
        ));
        assert_eq!(result, Err("stale"));
        assert_eq!(
            futures::executor::block_on(during(|| Ok::<_, ()>(()), async { 7 })),
            Ok(7)
        );
    }
}
