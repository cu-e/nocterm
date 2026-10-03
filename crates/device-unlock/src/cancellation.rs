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
    use nocterm_vault::{
        DeviceAvailability, DeviceCapability, DeviceUnlockProvider, VaultBinding, VaultKey,
        VaultService,
    };
    use std::sync::{Arc, Condvar, Mutex};

    #[derive(Default)]
    struct Context {
        invalidated: Mutex<bool>,
        changed: Condvar,
    }
    impl Context {
        fn invalidate(&self) {
            *self.invalidated.lock().unwrap() = true;
            self.changed.notify_all();
        }
        fn query(&self) {
            let state = self.invalidated.lock().unwrap();
            let (state, _) = self
                .changed
                .wait_timeout_while(state, Duration::from_secs(5), |state| !*state)
                .unwrap();
            assert!(*state, "native authentication was never invalidated");
        }
    }
    struct Provider {
        key: Mutex<Option<VaultKey>>,
        started: mpsc::Sender<()>,
        context: Arc<Context>,
    }
    impl DeviceUnlockProvider for Provider {
        fn probe(&self) -> Result<DeviceCapability, E> {
            Ok(DeviceCapability {
                availability: DeviceAvailability::Available,
                label: "Native test".into(),
                detail: String::new(),
                enabled: false,
                session_only: false,
            })
        }
        fn enroll(
            &self,
            _binding: VaultBinding,
            key: VaultKey,
            _cancel: &DeviceCancellation,
        ) -> Result<Vec<u8>, E> {
            *self.key.lock().unwrap() = Some(key);
            Ok(vec![1])
        }
        fn release(
            &self,
            _binding: VaultBinding,
            _token: &[u8],
            cancel: &DeviceCancellation,
        ) -> Result<VaultKey, E> {
            let context = self.context.clone();
            run(
                cancel,
                Duration::from_secs(30),
                move || context.invalidate(),
                || {
                    self.started.send(()).unwrap();
                    self.context.query();
                    // Native late success must still be rejected after invalidation.
                    Ok(self.key.lock().unwrap().as_ref().unwrap().clone())
                },
            )
        }
        fn remove(&self, _binding: VaultBinding, _token: &[u8]) -> Result<(), E> {
            Ok(())
        }
    }
    #[test]
    fn cancel_releases_single_vault_worker_for_password_without_native_resume() {
        let directory = tempfile::tempdir().unwrap();
        let (started, event) = mpsc::channel();
        let provider = Arc::new(Provider {
            key: Mutex::new(None),
            started,
            context: Arc::new(Context::default()),
        });
        let service = VaultService::new_with_device_unlock(
            directory.path().join("vault.bin"),
            Duration::from_secs(60),
            Some(provider.clone()),
        )
        .unwrap();
        futures::executor::block_on(
            service.create(nocterm_session::Secret::new("native test password")),
        )
        .unwrap();
        futures::executor::block_on(service.enable_device_unlock()).unwrap();
        service.lock();
        let native = service.unlock_with_device();
        event.recv_timeout(Duration::from_secs(2)).unwrap();
        service.lock();
        assert!(!service.is_unlocked());
        let password = service.unlock(nocterm_session::Secret::new("native test password"));
        let (finished, completion) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let native = futures::executor::block_on(native);
            let password = futures::executor::block_on(password);
            finished.send((native, password)).unwrap();
        });
        let (native, password) = completion.recv_timeout(Duration::from_secs(2)).unwrap();
        waiter.join().unwrap();
        assert!(matches!(native, Err(nocterm_vault::VaultError::Cancelled)));
        password.unwrap();
        assert!(*provider.context.invalidated.lock().unwrap());
        assert!(service.is_unlocked());
    }
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
        assert!(!*context.invalidated.lock().unwrap());
    }
}
