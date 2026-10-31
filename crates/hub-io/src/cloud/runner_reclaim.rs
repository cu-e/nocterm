//! Receipt reclaim for the independent runner (capability bit 64): the retry horizon of
//! a publication and the runner-signed deletion of a chat. A hub without the bit keeps
//! the old behaviour: no horizon is sent and a deletion stays local.
use super::{RunnerHubError, RunnerHubPeer, runner_peer::sdk_error};
use nocterm_cloud_model::journal::OUTBOX_RETRY_WINDOW_MS as RETRY_HORIZON_MS;
use nocterm_hub_client::cloud::{CloudError, RetireChatError};
use nocterm_hub_model::cloud::CloudTextId;
use std::{
    future::Future,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// The horizon for one publication attempt: `0` (none) unless the hub negotiated reclaim.
pub(super) fn retry_until(now_ms: i64, supported: bool) -> i64 {
    if supported {
        now_ms.saturating_add(RETRY_HORIZON_MS)
    } else {
        0
    }
}
pub(super) fn now_ms() -> Result<i64, RunnerHubError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .ok_or(RunnerHubError::Clock)
}

/// Where a requested retirement stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatRetirementOutcome {
    /// The stream left the active table.
    Compacted,
    /// A reader has not acknowledged the deletion yet; ask again later.
    Pending,
}

#[derive(Debug, thiserror::Error)]
pub enum RunnerReclaimError {
    /// The hub did not negotiate reclaim; the chat simply stays as it is.
    #[error("this hub does not support receipt reclaim")]
    Unsupported,
    /// The stream is already retired; nothing is left to do.
    #[error("the Cloud stream is already retired")]
    Retired,
    /// The hub refused the bounds of the deletion; repeating cannot help.
    #[error("the hub refused the bounds of the deletion")]
    Bounds,
    #[error("the deletion could not be signed or sent")]
    Hub(#[source] RunnerHubError),
}

impl RunnerHubPeer {
    /// Signs the deletion of `chat` with the runner's proof key and announces it.
    /// `requested_by` is the device that asked for the deletion.
    pub async fn retire_chat(
        &self,
        chat: [u8; 16],
        key_epoch: u64,
        requested_by: &str,
    ) -> Result<ChatRetirementOutcome, RunnerReclaimError> {
        let connection = self.connection();
        let requested_by = CloudTextId::new(requested_by)
            .map_err(|_| RunnerReclaimError::Hub(RunnerHubError::Binding))?;
        let runner = connection.runner().map_err(RunnerReclaimError::Hub)?;
        if !runner.supports_receipt_reclaim() {
            return Err(RunnerReclaimError::Unsupported);
        }
        let deleted_at = now_ms().map_err(RunnerReclaimError::Hub)?;
        let deletion = runner
            .sign_chat_deletion(key_epoch, chat, requested_by, deleted_at)
            .map_err(signing)?;
        let retirement = runner
            .retire_chat(&deletion)
            .await
            .map_err(|error| match error {
                RetireChatError::Unsupported => RunnerReclaimError::Unsupported,
                RetireChatError::Retired => RunnerReclaimError::Retired,
                RetireChatError::Bounds => RunnerReclaimError::Bounds,
                other => RunnerReclaimError::Hub(RunnerHubError::Remote(
                    nocterm_core::SharedCause::new(other),
                )),
            })?;
        connection.runner().map_err(RunnerReclaimError::Hub)?;
        Ok(if retirement.compacted {
            ChatRetirementOutcome::Compacted
        } else {
            ChatRetirementOutcome::Pending
        })
    }
}
fn signing(error: CloudError) -> RunnerReclaimError {
    RunnerReclaimError::Hub(sdk_error(error))
}

/// Repeats `attempt` every `interval` until the hub reports the stream compacted (or
/// already retired), up to `attempts` times. An unsupported hub or refused bounds end
/// it at once, as repeating cannot change them. Returns the last outcome.
pub async fn retire_until_compacted<F, Fut>(
    mut attempt: F,
    interval: Duration,
    attempts: u32,
) -> Result<ChatRetirementOutcome, RunnerReclaimError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<ChatRetirementOutcome, RunnerReclaimError>>,
{
    let mut last = ChatRetirementOutcome::Pending;
    for round in 0..attempts.max(1) {
        if round > 0 {
            tokio::time::sleep(interval).await;
        }
        match attempt().await {
            Ok(ChatRetirementOutcome::Compacted) | Err(RunnerReclaimError::Retired) => {
                return Ok(ChatRetirementOutcome::Compacted);
            }
            Ok(ChatRetirementOutcome::Pending) => last = ChatRetirementOutcome::Pending,
            Err(error @ (RunnerReclaimError::Unsupported | RunnerReclaimError::Bounds)) => {
                return Err(error);
            }
            // A transient hub failure is retried like a pending acknowledgement.
            Err(RunnerReclaimError::Hub(_)) => {}
        }
    }
    Ok(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    #[test]
    fn a_horizon_is_sent_only_when_the_hub_negotiated_reclaim() {
        assert_eq!(retry_until(1_000, false), 0);
        let horizon = retry_until(1_000, true);
        assert!(horizon > 1_000 && horizon - 1_000 <= 30 * 24 * 60 * 60 * 1000);
    }

    fn script(
        outcomes: Vec<Result<ChatRetirementOutcome, RunnerReclaimError>>,
    ) -> (
        Arc<AtomicU32>,
        impl FnMut() -> std::future::Ready<Result<ChatRetirementOutcome, RunnerReclaimError>>,
    ) {
        let calls = Arc::new(AtomicU32::new(0));
        let seen = calls.clone();
        let mut outcomes = outcomes.into_iter();
        (calls, move || {
            seen.fetch_add(1, Ordering::SeqCst);
            std::future::ready(
                outcomes
                    .next()
                    .unwrap_or(Ok(ChatRetirementOutcome::Pending)),
            )
        })
    }

    #[tokio::test(start_paused = true)]
    async fn a_deletion_is_repeated_until_a_reader_acknowledges_it() {
        let (calls, attempt) = script(vec![
            Ok(ChatRetirementOutcome::Pending),
            Err(RunnerReclaimError::Hub(RunnerHubError::Unavailable)),
            Ok(ChatRetirementOutcome::Compacted),
        ]);
        let outcome = retire_until_compacted(attempt, Duration::from_secs(60), 10).await;
        assert_eq!(outcome.unwrap(), ChatRetirementOutcome::Compacted);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn an_already_retired_stream_counts_as_done() {
        let (calls, attempt) = script(vec![Err(RunnerReclaimError::Retired)]);
        let outcome = retire_until_compacted(attempt, Duration::from_secs(60), 10).await;
        assert_eq!(outcome.unwrap(), ChatRetirementOutcome::Compacted);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn an_old_hub_or_refused_bounds_stop_at_once_and_pending_is_bounded() {
        let (calls, attempt) = script(vec![Err(RunnerReclaimError::Unsupported)]);
        let outcome = retire_until_compacted(attempt, Duration::from_secs(60), 10).await;
        assert!(matches!(outcome, Err(RunnerReclaimError::Unsupported)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let (calls, attempt) = script(vec![Err(RunnerReclaimError::Bounds)]);
        assert!(matches!(
            retire_until_compacted(attempt, Duration::from_secs(60), 10).await,
            Err(RunnerReclaimError::Bounds)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let (calls, attempt) = script(Vec::new());
        let outcome = retire_until_compacted(attempt, Duration::from_secs(60), 4).await;
        assert_eq!(outcome.unwrap(), ChatRetirementOutcome::Pending);
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }
}
