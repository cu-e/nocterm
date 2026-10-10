//! Scripted lifecycle contract for consumer tests. No filesystem or cryptography.
use crate::*;
use futures::{
    FutureExt as _,
    channel::{mpsc as progress, oneshot},
    future::BoxFuture,
};
use nocterm_session::{CredentialId, Secret};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Clone)]
pub struct ScriptedVaultAccess {
    memory: Arc<Mutex<Memory>>,
}
struct Memory {
    last_password: Option<Secret>,
    status: VaultStatus,
    revision: u64,
    records: Vec<(CredentialInfo, Secret)>,
    next: u128,
    device: DeviceCapability,
    next_unlock_error: Option<VaultError>,
    device_error: Option<DeviceUnlockError>,
    changes: Vec<oneshot::Sender<u64>>,
    unlocked: Vec<oneshot::Sender<()>>,
    hold_device: bool,
    device_completion: Option<oneshot::Sender<Result<(), VaultError>>>,
    progress: Option<progress::UnboundedSender<u8>>,
    probes: Option<Vec<oneshot::Sender<DeviceCapability>>>,
}
impl Memory {
    fn publish(&mut self, status: VaultStatus) {
        if self.status == status {
            return;
        }
        self.status = status;
        self.revision += 1;
        for waiter in self.changes.drain(..) {
            let _ = waiter.send(self.revision);
        }
        if status == VaultStatus::Unlocked {
            for waiter in self.unlocked.drain(..) {
                let _ = waiter.send(());
            }
        }
    }
}
impl Default for ScriptedVaultAccess {
    fn default() -> Self {
        Self {
            memory: Arc::new(Mutex::new(Memory {
                last_password: None,
                status: VaultStatus::Missing,
                revision: 0,
                records: Vec::new(),
                next: 0,
                device: DeviceCapability {
                    availability: DeviceAvailability::Unavailable,
                    label: "Device unlock".into(),
                    detail: "Device unlock is not configured. Use the master password.".into(),
                    enabled: false,
                    armed: false,
                    session_only: false,
                },
                next_unlock_error: None,
                device_error: None,
                changes: Vec::new(),
                unlocked: Vec::new(),
                hold_device: false,
                device_completion: None,
                progress: None,
                probes: None,
            })),
        }
    }
}
impl ScriptedVaultAccess {
    pub fn last_password(&self) -> Option<String> {
        self.memory
            .lock()
            .unwrap()
            .last_password
            .as_ref()
            .map(|s| s.expose().to_owned())
    }
    pub fn set_device_capability(&self, capability: DeviceCapability) {
        self.memory.lock().unwrap().device = capability;
    }
    pub fn fail_next_unlock(&self, error: VaultError) {
        self.memory.lock().unwrap().next_unlock_error = Some(error);
    }
    pub fn fail_device_unlock(&self, error: DeviceUnlockError) {
        self.memory.lock().unwrap().device_error = Some(error);
    }
    pub fn hold_device_probe(&self) {
        self.memory.lock().unwrap().probes = Some(Vec::new());
    }
    pub fn complete_device_probe(&self) {
        let mut memory = self.memory.lock().unwrap();
        if let Some(probes) = memory.probes.take() {
            for probe in probes {
                let _ = probe.send(memory.device.clone());
            }
        }
    }
    pub fn hold_device_unlock(&self) {
        self.memory.lock().unwrap().hold_device = true;
    }
    pub fn report_progress(&self, remaining: u8) {
        if let Some(sender) = &self.memory.lock().unwrap().progress {
            let _ = sender.unbounded_send(remaining);
        }
    }
    pub fn complete_device_unlock(&self, result: Result<(), VaultError>) {
        let mut memory = self.memory.lock().unwrap();
        if matches!(&result, Err(VaultError::Device(DeviceUnlockError::Locked))) {
            memory.device.availability = DeviceAvailability::LockedOut;
            memory.device.detail = DeviceUnlockError::Locked.to_string();
        }
        if let Some(sender) = memory.device_completion.take() {
            let _ = sender.send(result);
        }
    }
    fn request<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Memory) -> Result<T, VaultError> + Send + 'static,
    ) -> VaultFuture<T> {
        let memory = self.memory.clone();
        async move { operation(&mut memory.lock().unwrap()) }.boxed()
    }
}
impl VaultAccess for ScriptedVaultAccess {
    fn status(&self) -> VaultStatus {
        self.memory.lock().unwrap().status
    }
    fn is_unlocked(&self) -> bool {
        self.status() == VaultStatus::Unlocked
    }
    fn exists(&self) -> bool {
        self.status() != VaultStatus::Missing
    }
    fn revision(&self) -> u64 {
        self.memory.lock().unwrap().revision
    }
    fn changed(&self, seen: u64) -> BoxFuture<'static, u64> {
        let mut memory = self.memory.lock().unwrap();
        if memory.revision != seen {
            return futures::future::ready(memory.revision).boxed();
        }
        let (tx, rx) = oneshot::channel();
        memory.changes.push(tx);
        async move { rx.await.unwrap_or(seen) }.boxed()
    }
    fn wait_unlocked(&self) -> BoxFuture<'static, ()> {
        let mut memory = self.memory.lock().unwrap();
        if memory.status == VaultStatus::Unlocked {
            return futures::future::ready(()).boxed();
        }
        let (tx, rx) = oneshot::channel();
        memory.unlocked.push(tx);
        async move {
            let _ = rx.await;
        }
        .boxed()
    }
    fn set_auto_lock(&self, _: Duration) {}
    fn lock(&self) {
        let mut memory = self.memory.lock().unwrap();
        let status = if memory.status == VaultStatus::Missing {
            VaultStatus::Missing
        } else {
            VaultStatus::Locked
        };
        memory.publish(status);
        memory.progress = None;
        if let Some(sender) = memory.device_completion.take() {
            let _ = sender.send(Err(VaultError::Cancelled));
        }
    }
    fn create(&self, password: Secret) -> VaultFuture<()> {
        self.request(move |memory| {
            memory.last_password = Some(password);
            if memory.status != VaultStatus::Missing {
                return Err(VaultError::AlreadyExists);
            }
            memory.publish(VaultStatus::Unlocked);
            Ok(())
        })
    }
    fn unlock(&self, password: Secret) -> VaultFuture<()> {
        self.request(move |memory| {
            memory.last_password = Some(password);
            if let Some(error) = memory.next_unlock_error.take() {
                return Err(error);
            }
            memory.publish(VaultStatus::Unlocked);
            Ok(())
        })
    }
    fn change_password(&self, password: Secret) -> VaultFuture<()> {
        self.request(move |memory| {
            memory.last_password = Some(password);
            if memory.status == VaultStatus::Unlocked {
                Ok(())
            } else {
                Err(VaultError::Locked)
            }
        })
    }
    fn list(&self) -> VaultFuture<Vec<CredentialInfo>> {
        self.request(|memory| {
            if memory.status != VaultStatus::Unlocked {
                return Err(VaultError::Locked);
            }
            Ok(memory
                .records
                .iter()
                .map(|(info, _)| info.clone())
                .collect())
        })
    }
    fn get(&self, id: CredentialId, binding: CredentialBinding) -> VaultFuture<Secret> {
        self.request(move |memory| {
            if memory.status != VaultStatus::Unlocked {
                return Err(VaultError::Locked);
            }
            let (info, secret) = memory
                .records
                .iter()
                .find(|(info, _)| info.id == id)
                .ok_or(VaultError::NotFound)?;
            if info.binding != binding {
                return Err(VaultError::Binding);
            }
            Ok(secret.clone())
        })
    }
    fn put(
        &self,
        id: Option<CredentialId>,
        label: String,
        binding: CredentialBinding,
        secret: Secret,
    ) -> VaultFuture<CredentialId> {
        self.request(move |memory| {
            if memory.status != VaultStatus::Unlocked {
                return Err(VaultError::Locked);
            }
            memory.next += 1;
            let id = id.unwrap_or_else(|| CredentialId::from_bytes(memory.next.to_be_bytes()));
            memory.records.retain(|(info, _)| info.id != id);
            memory
                .records
                .push((CredentialInfo { id, label, binding }, secret));
            Ok(id)
        })
    }
    fn delete(&self, id: CredentialId) -> VaultFuture<()> {
        self.request(move |memory| {
            if memory.status != VaultStatus::Unlocked {
                return Err(VaultError::Locked);
            }
            memory.records.retain(|(info, _)| info.id != id);
            Ok(())
        })
    }
    fn probe_device_unlock(&self) -> VaultFuture<DeviceCapability> {
        let mut memory = self.memory.lock().unwrap();
        if let Some(probes) = memory.probes.as_mut() {
            let (sender, receiver) = oneshot::channel();
            probes.push(sender);
            async move { receiver.await.map_err(|_| VaultError::Cancelled) }.boxed()
        } else {
            futures::future::ready(Ok(memory.device.clone())).boxed()
        }
    }
    fn enable_device_unlock(&self) -> VaultFuture<()> {
        self.request(|memory| {
            if memory.device.availability != DeviceAvailability::Available {
                return Err(DeviceUnlockError::Unavailable.into());
            }
            memory.device.enabled = true;
            memory.device.armed = true;
            Ok(())
        })
    }
    fn disable_device_unlock(&self) -> VaultFuture<()> {
        self.request(|memory| {
            memory.device.enabled = false;
            memory.device.armed = false;
            Ok(())
        })
    }
    fn unlock_with_device(&self) -> VaultFuture<()> {
        self.unlock_with_device_progress().0
    }
    fn unlock_with_device_progress(&self) -> (VaultFuture<()>, progress::UnboundedReceiver<u8>) {
        let (progress, receiver) = progress::unbounded();
        let (tx, rx) = oneshot::channel();
        let mut memory = self.memory.lock().unwrap();
        memory.progress = Some(progress);
        if memory.hold_device {
            memory.device_completion = Some(tx);
        } else {
            let result = memory
                .device_error
                .take()
                .map(|error| Err(error.into()))
                .unwrap_or(Ok(()));
            let _ = tx.send(result);
        }
        drop(memory);
        let memory = self.memory.clone();
        (
            async move {
                rx.await.unwrap_or(Err(VaultError::Cancelled))?;
                memory.lock().unwrap().publish(VaultStatus::Unlocked);
                Ok(())
            }
            .boxed(),
            receiver,
        )
    }
}
