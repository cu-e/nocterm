//! A bounded worker keeps expensive KDF and atomic file writes off UI threads.
use crate::{
    CredentialBinding, CredentialInfo, DeviceCapability, DeviceUnlockProvider, Vault, VaultError,
};
use futures::{FutureExt as _, channel::oneshot, future::BoxFuture};
use nocterm_session::{CredentialId, Secret};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

pub type VaultFuture<T> = BoxFuture<'static, Result<T, VaultError>>;
type Operation = Box<dyn FnOnce(&mut Vault) + Send>;
struct Job {
    epoch: u64,
    operation: Operation,
}
struct State {
    epoch: Arc<AtomicU64>,
    unlocked: AtomicBool,
    exists: AtomicBool,
    published_epoch: AtomicU64,
    auto_lock_secs: AtomicU64,
}

/// Thread-safe provider. No method waits for a KDF or file I/O on the caller.
/// At most eight operations are queued behind a single worker. Lock invalidates
/// queued and in-flight results; intermediate secrets are zeroized when dropped.
#[derive(Clone)]
pub struct VaultService {
    jobs: SyncSender<Job>,
    state: Arc<State>,
}
impl VaultService {
    pub fn new(path: impl Into<PathBuf>, auto_lock: Duration) -> Result<Self, VaultError> {
        Self::new_with_device_unlock(path, auto_lock, None)
    }
    pub fn new_with_device_unlock(
        path: impl Into<PathBuf>,
        auto_lock: Duration,
        device: Option<Arc<dyn DeviceUnlockProvider>>,
    ) -> Result<Self, VaultError> {
        let path = path.into();
        let state = Arc::new(State {
            epoch: Arc::new(AtomicU64::new(0)),
            unlocked: AtomicBool::new(false),
            exists: AtomicBool::new(path.exists()),
            published_epoch: AtomicU64::new(0),
            auto_lock_secs: AtomicU64::new(auto_lock.as_secs().max(1)),
        });
        let (sender, receiver) = mpsc::sync_channel::<Job>(8);
        let worker_state = state.clone();
        thread::Builder::new()
            .name("nocterm-vault".into())
            .spawn(move || {
                let mut vault = Vault::new(path);
                vault.device = device;
                let mut epoch = 0;
                let mut touched = Instant::now();
                loop {
                    let now_epoch = worker_state.epoch.load(Ordering::SeqCst);
                    if epoch != now_epoch {
                        vault.lock();
                        epoch = now_epoch;
                    }
                    if vault.is_unlocked()
                        && touched.elapsed().as_secs()
                            >= worker_state.auto_lock_secs.load(Ordering::Relaxed)
                    {
                        vault.lock();
                        epoch = worker_state.epoch.fetch_add(1, Ordering::SeqCst) + 1;
                        worker_state.unlocked.store(false, Ordering::SeqCst);
                    }
                    match receiver.recv_timeout(Duration::from_millis(250)) {
                        Ok(job) => {
                            if job.epoch != worker_state.epoch.load(Ordering::SeqCst) {
                                drop(job);
                                continue;
                            }
                            vault.guard = Some((worker_state.epoch.clone(), job.epoch));
                            (job.operation)(&mut vault);
                            vault.guard = None;
                            if job.epoch != worker_state.epoch.load(Ordering::SeqCst) {
                                vault.lock();
                            }
                            worker_state
                                .published_epoch
                                .store(job.epoch, Ordering::SeqCst);
                            worker_state
                                .unlocked
                                .store(vault.is_unlocked(), Ordering::SeqCst);
                            worker_state.exists.store(vault.exists(), Ordering::SeqCst);
                            touched = Instant::now();
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            })?;
        Ok(Self {
            jobs: sender,
            state,
        })
    }
    pub fn is_unlocked(&self) -> bool {
        self.state.unlocked.load(Ordering::SeqCst)
            && self.state.published_epoch.load(Ordering::SeqCst)
                == self.state.epoch.load(Ordering::SeqCst)
    }
    pub fn exists(&self) -> bool {
        self.state.exists.load(Ordering::SeqCst)
    }
    pub fn set_auto_lock(&self, duration: Duration) {
        self.state
            .auto_lock_secs
            .store(duration.as_secs().max(1), Ordering::Relaxed);
    }
    pub fn lock(&self) {
        self.state.epoch.fetch_add(1, Ordering::SeqCst);
        self.state.unlocked.store(false, Ordering::SeqCst);
        // Wake the worker. If its bounded queue is full, the next job already
        // wakes it and will observe the changed epoch before accessing secrets.
        let _ = self.jobs.try_send(Job {
            epoch: self.state.epoch.load(Ordering::SeqCst),
            operation: Box::new(|vault| vault.lock()),
        });
    }
    fn request<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Vault) -> Result<T, VaultError> + Send + 'static,
    ) -> VaultFuture<T> {
        let (sender, receiver) = oneshot::channel();
        let state = self.state.clone();
        let epoch = state.epoch.load(Ordering::SeqCst);
        let job = Job {
            epoch,
            operation: Box::new(move |vault| {
                let result = operation(vault);
                let result = if epoch == state.epoch.load(Ordering::SeqCst) {
                    result
                } else {
                    Err(VaultError::Cancelled)
                };
                state.exists.store(vault.exists(), Ordering::SeqCst);
                state.published_epoch.store(epoch, Ordering::SeqCst);
                state.unlocked.store(vault.is_unlocked(), Ordering::SeqCst);
                let _ = sender.send(result);
            }),
        };
        let sent = self.jobs.try_send(job).is_ok();
        async move {
            if !sent {
                return Err(VaultError::Busy);
            }
            receiver.await.unwrap_or(Err(VaultError::Cancelled))
        }
        .boxed()
    }
    pub fn create(&self, password: Secret) -> VaultFuture<()> {
        self.request(move |vault| vault.create(password))
    }
    pub fn unlock(&self, password: Secret) -> VaultFuture<()> {
        self.request(move |vault| vault.unlock(password))
    }
    pub fn probe_device_unlock(&self) -> VaultFuture<DeviceCapability> {
        self.request(|vault| vault.probe_device_unlock())
    }
    pub fn enable_device_unlock(&self) -> VaultFuture<()> {
        self.request(|vault| vault.enable_device_unlock())
    }
    pub fn unlock_with_device(&self) -> VaultFuture<()> {
        self.request(|vault| vault.unlock_with_device())
    }
    pub fn disable_device_unlock(&self) -> VaultFuture<()> {
        self.request(|vault| vault.disable_device_unlock())
    }
    pub fn change_password(&self, password: Secret) -> VaultFuture<()> {
        self.request(move |vault| vault.change_password(password))
    }
    pub fn list(&self) -> VaultFuture<Vec<CredentialInfo>> {
        self.request(|vault| vault.list())
    }
    pub fn get(&self, id: CredentialId, binding: CredentialBinding) -> VaultFuture<Secret> {
        self.request(move |vault| vault.get(id, &binding))
    }
    pub fn put(
        &self,
        id: Option<CredentialId>,
        label: String,
        binding: CredentialBinding,
        secret: Secret,
    ) -> VaultFuture<CredentialId> {
        self.request(move |vault| vault.put(id, label, binding, secret))
    }
    pub fn delete(&self, id: CredentialId) -> VaultFuture<()> {
        self.request(move |vault| vault.delete(id))
    }
}
