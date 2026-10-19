//! One shared in-flight cleanup, with one final retry after an observed failure.
use super::Completion;
#[derive(Default)]
pub(super) struct Attempts {
    current: Option<Completion>,
    final_attempted: bool,
}
impl Attempts {
    pub(super) fn existing(&mut self, force: bool) -> Option<Completion> {
        let retry = force
            && !self.final_attempted
            && self
                .current
                .as_ref()
                .and_then(Completion::peek)
                .is_some_and(Result::is_err);
        if retry {
            self.current = None;
        }
        if force && self.current.is_none() {
            self.final_attempted = true;
        }
        self.current.clone()
    }
    pub(super) fn record(&mut self, completion: Completion) {
        self.current = Some(completion);
    }
}
#[cfg(test)]
mod tests {
    use super::super::CleanupError;
    use super::*;
    use futures::{FutureExt, executor::block_on, future};
    #[test]
    fn one_failed_normal_attempt_can_be_retried_once_by_final_cleanup() {
        let mut attempts = Attempts::default();
        assert!(attempts.existing(false).is_none());
        let first = future::ready(Err(CleanupError::WorkerLost))
            .boxed()
            .shared();
        attempts.record(first.clone());
        assert!(block_on(first).is_err());
        assert!(attempts.existing(false).is_some());
        assert!(attempts.existing(true).is_none());
        let final_attempt = future::ready(Err(CleanupError::WorkerLost))
            .boxed()
            .shared();
        attempts.record(final_attempt.clone());
        assert!(block_on(final_attempt).is_err());
        assert!(attempts.existing(true).is_some());
        assert!(attempts.existing(true).is_some());
    }
    #[test]
    fn final_cleanup_reuses_inflight_and_successful_original_completion() {
        let mut attempts = Attempts::default();
        let pending = future::pending().boxed().shared();
        attempts.record(pending);
        assert!(attempts.existing(true).is_some());
        let success = future::ready(Ok(())).boxed().shared();
        attempts.record(success.clone());
        assert!(block_on(success).is_ok());
        assert!(attempts.existing(true).is_some());
    }
}
