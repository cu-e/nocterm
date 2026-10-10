//! Scoped native cancellation fixture without OS authentication prompts.
use crate::cancellation::run;
use nocterm_vault::{
    DeviceAvailability, DeviceCancellation, DeviceCapability, DeviceUnlockError as E,
    DeviceUnlockProvider, VaultBinding, VaultKey,
};
use std::{
    sync::{Arc, Condvar, Mutex, mpsc},
    time::Duration,
};
#[derive(Default)]
pub(crate) struct Context {
    invalidated: Mutex<bool>,
    changed: Condvar,
}
impl Context {
    pub(crate) fn was_invalidated(&self) -> bool {
        *self.invalidated.lock().unwrap()
    }
    pub(crate) fn invalidate(&self) {
        *self.invalidated.lock().unwrap() = true;
        self.changed.notify_all();
    }
    pub(crate) fn query(&self) {
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
            armed: false,
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

pub struct NativeCancellationProbe {
    started: mpsc::Receiver<()>,
    context: Arc<Context>,
}
impl NativeCancellationProbe {
    pub fn wait_started(&self, timeout: Duration) -> Result<(), mpsc::RecvTimeoutError> {
        self.started.recv_timeout(timeout)
    }
    pub fn was_invalidated(&self) -> bool {
        self.context.was_invalidated()
    }
}
pub fn scoped_cancellation_provider() -> (Arc<dyn DeviceUnlockProvider>, NativeCancellationProbe) {
    let (started, event) = mpsc::channel();
    let context = Arc::new(Context::default());
    (
        Arc::new(Provider {
            key: Mutex::new(None),
            started,
            context: context.clone(),
        }),
        NativeCancellationProbe {
            started: event,
            context,
        },
    )
}
