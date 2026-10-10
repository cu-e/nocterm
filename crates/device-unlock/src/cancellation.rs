//! One joined watchdog invalidates a native context while its query is blocked.
use nocterm_vault::{DeviceCancellation, DeviceUnlockError as E};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
struct Completion(mpsc::Sender<()>);
impl Drop for Completion {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

pub(crate) fn run<T>(
    cancel: &DeviceCancellation,
    timeout: Duration,
    invalidate: impl FnOnce() + Send,
    query: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    cancel.check()?;
    let deadline = Instant::now() + timeout;
    std::thread::scope(|scope| {
        let (complete, completion) = mpsc::channel();
        let watcher = std::thread::Builder::new()
            .name("nocterm-auth-cancel".into())
            .spawn_scoped(scope, move || {
                loop {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if cancel.is_cancelled() || remaining.is_zero() {
                        invalidate();
                        return true;
                    }
                    match completion.recv_timeout(remaining.min(Duration::from_millis(50))) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return false,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
            })
            .map_err(crate::platform)?;
        // The native key query stays on this thread. Invalidation terminates
        // its policy evaluation; no detached query or result can outlive run.
        let complete = Completion(complete);
        let result = query();
        drop(complete);
        let invalidated = watcher
            .join()
            .map_err(|_| E::Platform("Native cancellation watcher failed".into()))?;
        cancel.check()?;
        if invalidated || Instant::now() >= deadline {
            return Err(E::Cancelled);
        }
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_vault::VaultKey;
    use std::sync::Arc;

    use crate::testing::Context;
    #[test]
    fn timeout_invalidates_native_context_and_rejects_late_success() {
        let context = Arc::new(Context::default());
        let invalidation = context.clone();
        let started = Instant::now();
        let result = run(
            &DeviceCancellation::never(),
            Duration::from_millis(20),
            move || invalidation.invalidate(),
            || {
                context.query();
                Ok(VaultKey::new([42; 32]))
            },
        );
        assert!(matches!(result, Err(E::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn successful_query_joins_watchdog_without_invalidating() {
        let context = Arc::new(Context::default());
        let invalidation = context.clone();
        let result = run(
            &DeviceCancellation::never(),
            Duration::from_secs(30),
            move || invalidation.invalidate(),
            || Ok(42),
        );
        assert_eq!(result.unwrap(), 42);
        assert!(!context.was_invalidated());
    }
}
