//! Waits for the runner to claim a new chat, republishing the stored grant meanwhile.

use std::time::Duration;

use futures::future::BoxFuture;
use nocterm_cloud_model::projection::ProjectionError;
use nocterm_cloud_model::tool::ToolTimer;

use super::error::CloudChatError;

/// How long a new chat waits for its runner before giving up.
pub(super) const PATIENCE: Duration = Duration::from_secs(60);
/// How often the stored grant is sent again: a runner that crashed after acknowledging it but
/// before claiming the chat needs it once more.
pub(super) const REPUBLISH: Duration = Duration::from_secs(15);
const BACKOFF: Duration = Duration::from_secs(1);

/// Sends the grant, then tries `open` until the runner has claimed the chat.
///
/// Only `Unavailable` means "not yet"; every other refusal ends the wait. A zero `patience` tries
/// once.
pub(super) async fn until_open<T>(
    timer: &dyn ToolTimer,
    patience: Duration,
    mut publish: impl FnMut() -> BoxFuture<'static, Result<(), CloudChatError>>,
    mut open: impl FnMut() -> BoxFuture<'static, Result<T, ProjectionError>>,
) -> Result<T, CloudChatError> {
    let started = timer.now().as_millis();
    let mut published = started;
    publish().await?;
    loop {
        match open().await {
            Ok(value) => return Ok(value),
            Err(ProjectionError::Unavailable) => {}
            Err(error) => return Err(error.into()),
        }
        let now = timer.now().as_millis();
        if now.saturating_sub(started) >= millis(patience) {
            return Err(CloudChatError::RunnerNotReady);
        }
        if now.saturating_sub(published) >= millis(REPUBLISH) {
            published = now;
            publish().await?;
        }
        timer.delay(BACKOFF).await;
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use futures::FutureExt as _;
    use nocterm_cloud_model::testing::ManualToolTimer;

    use super::*;

    /// Opens after `ready` unavailable answers; counts publications.
    struct Run {
        publications: Arc<AtomicUsize>,
        opens: Arc<AtomicUsize>,
    }

    impl Run {
        fn new() -> Self {
            Self {
                publications: Arc::default(),
                opens: Arc::default(),
            }
        }

        async fn drive(
            &self,
            timer: &ManualToolTimer,
            patience: Duration,
            ready: Option<usize>,
        ) -> Result<usize, CloudChatError> {
            let publications = self.publications.clone();
            let opens = self.opens.clone();
            let wait = until_open(
                timer,
                patience,
                move || {
                    publications.fetch_add(1, Ordering::SeqCst);
                    async { Ok(()) }.boxed()
                },
                move || {
                    let attempt = opens.fetch_add(1, Ordering::SeqCst) + 1;
                    let answer = match ready {
                        Some(ready) if attempt > ready => Ok(attempt),
                        _ => Err(ProjectionError::Unavailable),
                    };
                    async move { answer }.boxed()
                },
            );
            let mut wait = std::pin::pin!(wait);
            loop {
                tokio::select! {
                    result = &mut wait => return result,
                    () = tokio::task::yield_now() => timer.advance(1000),
                }
            }
        }
    }

    #[tokio::test]
    async fn a_chat_opens_as_soon_as_the_runner_claims_it() {
        let run = Run::new();
        let opened = run
            .drive(&ManualToolTimer::default(), PATIENCE, Some(3))
            .await
            .unwrap();
        assert_eq!(opened, 4);
        assert_eq!(run.publications.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_chat_the_runner_never_claims_times_out_as_not_ready() {
        let run = Run::new();
        let outcome = run.drive(&ManualToolTimer::default(), PATIENCE, None).await;
        assert!(matches!(outcome, Err(CloudChatError::RunnerNotReady)));
        assert!(outcome.unwrap_err().is_retryable());
    }

    #[tokio::test]
    async fn the_stored_grant_is_sent_again_every_fifteen_seconds_while_waiting() {
        let run = Run::new();
        let _ = run.drive(&ManualToolTimer::default(), PATIENCE, None).await;
        // First send plus a repeat at 15 s, 30 s and 45 s; the deadline ends the fourth period.
        assert_eq!(run.publications.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn a_zero_patience_tries_once_and_publishes_once() {
        let run = Run::new();
        let outcome = run
            .drive(&ManualToolTimer::default(), Duration::ZERO, None)
            .await;
        assert!(matches!(outcome, Err(CloudChatError::RunnerNotReady)));
        assert_eq!(run.opens.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn another_refusal_ends_the_wait_without_retrying() {
        let timer = ManualToolTimer::default();
        let opens = Arc::new(AtomicUsize::new(0));
        let counted = opens.clone();
        let outcome: Result<(), _> = until_open(
            &timer,
            PATIENCE,
            || async { Ok(()) }.boxed(),
            move || {
                counted.fetch_add(1, Ordering::SeqCst);
                async { Err(ProjectionError::WrongSource) }.boxed()
            },
        )
        .await;
        assert!(matches!(
            outcome,
            Err(CloudChatError::Projection(ProjectionError::WrongSource))
        ));
        assert_eq!(opens.load(Ordering::SeqCst), 1);
    }
}
